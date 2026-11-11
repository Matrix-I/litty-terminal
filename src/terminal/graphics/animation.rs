//! Kitty animation: extra frames (`a=f`), frame composition (`a=c`) and
//! clock-driven playback (`a=a`). Frame data stays on the CPU side; the
//! renderer picks the frame to upload through [`Image::shown`].

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::kitty::{compose_pixel, DecodedImage, KittyCommand};
use super::store::Store;
use super::{next_key, MAX_ANIMATION_BYTES, MAX_ANIMATION_FRAMES};

pub struct Image {
    /// The root frame (frame 1).
    pub key: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
    /// Whether this image was automatically assigned an ID by the terminal.
    pub auto: bool,
    /// Kitty animation frames 2.. (`a=f`) and playback state (`a=a`).
    pub(super) frames: Vec<Frame>,
    pub(super) anim: Animation,
}

pub(super) struct Frame {
    pub(super) key: u64,
    pub(super) rgba: Arc<[u8]>,
    /// Milliseconds before the next frame; 0 or less = skipped (gapless).
    pub(super) gap: i32,
}

/// Kitty's default gap of a new frame.
pub(super) const DEFAULT_GAP: i32 = 40;

/// Set once any terminal starts an animation, so the event loop only looks
/// for frame deadlines after that.
pub(super) static ANIMATIONS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn any_animation() -> bool {
    ANIMATIONS.load(Ordering::Relaxed)
}

/// Playback is derived from the clock (no timer ticks): frame `i` shows while
/// the time since `started`, modulo the cycle, falls into its gap.
#[derive(Default)]
pub(super) struct Animation {
    /// 1 = stopped, 2 = loading (play once, then wait), 3 = running.
    pub(super) state: u8,
    /// Frame shown while stopped (0-based).
    pub(super) current: usize,
    /// Start of the timeline (frame 0), while playing.
    pub(super) started: Option<Instant>,
    /// Plays of the whole cycle, 0 = forever.
    pub(super) plays: u32,
    pub(super) root_gap: i32,
}

impl Image {
    pub(super) fn new(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        let anim = Animation { root_gap: DEFAULT_GAP, ..Animation::default() };
        Image { key: next_key(), width, height, rgba: rgba.into(), auto: false, frames: Vec::new(), anim }
    }

    pub(super) fn frame_count(&self) -> usize {
        1 + self.frames.len()
    }

    pub(super) fn frame(&self, i: usize) -> (u64, &Arc<[u8]>) {
        match i.checked_sub(1).and_then(|j| self.frames.get(j)) {
            Some(f) => (f.key, &f.rgba),
            None => (self.key, &self.rgba),
        }
    }

    pub(super) fn gap(&self, i: usize) -> u64 {
        let gap = if i == 0 { self.anim.root_gap } else { self.frames.get(i - 1).map_or(0, |f| f.gap) };
        gap.max(0) as u64
    }

    pub(super) fn bytes(&self) -> usize {
        self.rgba.len() + self.frames.iter().map(|f| f.rgba.len()).sum::<usize>()
    }

    /// Start of frame `i` on the timeline, in ms.
    pub(super) fn offset(&self, i: usize) -> u64 {
        (0..i).map(|j| self.gap(j)).sum()
    }

    /// The frame shown at `now` and when that changes (`None`: it does not).
    pub(super) fn frame_at(&self, now: Instant) -> (usize, Option<Instant>) {
        let a = &self.anim;
        let n = self.frame_count();
        let cycle = self.offset(n);
        let (Some(started), true, true) = (a.started, n > 1, cycle > 0) else { return (a.current.min(n - 1), None) };
        let plays = if a.state == 2 { 1 } else { a.plays };
        let elapsed = now.saturating_duration_since(started).as_millis() as u64;
        // Gaps and the loop count come from program output: their product can
        // exceed u64, which must read as "not finished yet", not wrap around.
        if plays != 0 && elapsed >= cycle.saturating_mul(plays as u64) {
            return (n - 1, None);
        }
        let mut t = elapsed % cycle;
        for i in 0..n {
            let gap = self.gap(i);
            if t < gap {
                return (i, Some(now + Duration::from_millis(gap - t)));
            }
            t -= gap;
        }
        (n - 1, None)
    }

    /// Key and pixels of the frame to draw at `now`.
    pub fn shown(&self, now: Instant) -> (u64, &Arc<[u8]>) {
        if self.frames.is_empty() {
            return (self.key, &self.rgba);
        }
        self.frame(self.frame_at(now).0)
    }
}

impl Store {
    /// `a=f`: add (or with `r=`, edit) an animation frame. The frame data
    /// (`s`/`v` sized, default the whole image) goes at `x`,`y` over a copy
    /// of frame `c` (or a transparent canvas), blended unless `X=1`.
    pub(super) fn kitty_frame(&mut self, cmd: &KittyCommand, decoded: DecodedImage) -> Result<(), String> {
        let img = self.images.get(&cmd.id).ok_or("ENOENT:image not found")?;
        let (edit, base) = (cmd.rows as usize, cmd.cols as usize);
        let n = img.frame_count();
        if edit > n || base > n {
            return Err("EINVAL:no such frame".into());
        }
        let (iw, ih) = (img.width, img.height);
        if edit == 0 {
            if img.frames.len() >= MAX_ANIMATION_FRAMES {
                return Err("EBIG:too many animation frames".into());
            }
            let frame_bytes = (iw as usize).saturating_mul(ih as usize).saturating_mul(4);
            if img.bytes().saturating_add(frame_bytes) > MAX_ANIMATION_BYTES {
                return Err("EBIG:animation exceeds memory limit".into());
            }
        }
        let (fw, fh, pixels) = decoded?;
        let mut canvas = match (edit, base) {
            (0, 0) => vec![0u8; (iw * ih * 4) as usize],
            (0, b) => img.frame(b - 1).1.to_vec(),
            (e, _) => img.frame(e - 1).1.to_vec(),
        };
        for y in 0..fh.min(ih.saturating_sub(cmd.y)) {
            for x in 0..fw.min(iw.saturating_sub(cmd.x)) {
                let src = &pixels[((y * fw + x) * 4) as usize..][..4];
                let dst = &mut canvas[(((cmd.y + y) * iw + cmd.x + x) * 4) as usize..][..4];
                compose_pixel(dst, src, cmd.replace);
            }
        }
        let len = canvas.len();
        let img = self.images.get_mut(&cmd.id).ok_or("ENOENT:image not found")?;
        let gap = if cmd.z != 0 { cmd.z } else { DEFAULT_GAP };
        match edit {
            0 => img.frames.push(Frame { key: next_key(), rgba: canvas.into(), gap }),
            1 => {
                img.key = next_key();
                img.rgba = canvas.into();
                if cmd.z != 0 {
                    img.anim.root_gap = cmd.z;
                }
                self.bytes -= len;
            }
            e => {
                let f = &mut img.frames[e - 2];
                *f = Frame { key: next_key(), rgba: canvas.into(), gap: if cmd.z != 0 { cmd.z } else { f.gap } };
                self.bytes -= len;
            }
        }
        self.bytes += len;
        self.evict();
        Ok(())
    }

    /// `a=c`: copy a `w` x `h` rectangle at `X`,`Y` of frame `r` to `x`,`y`
    /// of frame `c`, blended unless `C=1`.
    pub(super) fn kitty_compose(&mut self, cmd: &KittyCommand) -> Result<(), String> {
        let img = self.images.get_mut(&cmd.id).ok_or("ENOENT:image not found")?;
        let (src, dst) = (cmd.rows as usize, cmd.cols as usize);
        let n = img.frame_count();
        if !(1..=n).contains(&src) || !(1..=n).contains(&dst) {
            return Err("ENOENT:no such frame".into());
        }
        let (iw, ih) = (img.width, img.height);
        let w = if cmd.src_w == 0 { iw } else { cmd.src_w };
        let h = if cmd.src_h == 0 { ih } else { cmd.src_h };
        if [(cmd.src_left, w, iw), (cmd.src_top, h, ih), (cmd.x, w, iw), (cmd.y, h, ih)]
            .into_iter().any(|(start, size, bound)| start.checked_add(size).is_none_or(|end| end > bound)) {
            return Err("EINVAL:rectangle out of bounds".into());
        }
        let source = img.frame(src - 1).1.clone();
        let destination = img.frame(dst - 1).1;
        let image_bytes = (iw as usize).checked_mul(ih as usize).and_then(|n| n.checked_mul(4));
        if image_bytes != Some(source.len()) || image_bytes != Some(destination.len()) {
            return Err("EINVAL:invalid frame dimensions".into());
        }
        let mut canvas = img.frame(dst - 1).1.to_vec();
        // The rectangle endpoints and full frame byte lengths were validated
        // above. Use usize for offsets so intermediate u32 products cannot wrap.
        let stride = iw as usize * 4;
        for y in 0..h {
            let src_row = (cmd.src_top + y) as usize * stride;
            let dst_row = (cmd.y + y) as usize * stride;
            for x in 0..w {
                let src_offset = src_row + (cmd.src_left + x) as usize * 4;
                let dst_offset = dst_row + (cmd.x + x) as usize * 4;
                let s = &source[src_offset..src_offset + 4];
                let d = &mut canvas[dst_offset..dst_offset + 4];
                compose_pixel(d, s, cmd.overwrite);
            }
        }
        match dst {
            1 => {
                img.key = next_key();
                img.rgba = canvas.into();
            }
            d => {
                let f = &mut img.frames[d - 2];
                f.key = next_key();
                f.rgba = canvas.into();
            }
        }
        Ok(())
    }

    /// `a=a`: control playback. `s` = 1 stop / 2 loading / 3 run, `c` = frame
    /// to show, `r` + `z` = that frame's gap, `v` = loops (1 = forever).
    pub(super) fn kitty_animate(&mut self, cmd: &KittyCommand, now: Instant) -> Result<(), String> {
        let img = self.images.get_mut(&cmd.id).ok_or("ENOENT:image not found")?;
        let n = img.frame_count();
        if cmd.rows > 0 && cmd.z != 0 {
            match cmd.rows as usize {
                1 => img.anim.root_gap = cmd.z,
                r if r <= n => img.frames[r - 2].gap = cmd.z,
                _ => return Err("EINVAL:no such frame".into()),
            }
        }
        if cmd.height > 0 {
            img.anim.plays = if cmd.height == 1 { 0 } else { cmd.height - 1 };
        }
        let mut current = img.frame_at(now).0;
        if cmd.cols > 0 {
            current = (cmd.cols as usize - 1).min(n - 1);
        }
        let state = if cmd.width > 0 { cmd.width as u8 } else { img.anim.state };
        img.anim.state = state;
        img.anim.current = current;
        // Playing: put frame `current` at `now` on the timeline.
        img.anim.started = matches!(state, 2 | 3).then(|| now - Duration::from_millis(img.offset(current)));
        if img.anim.started.is_some() {
            ANIMATIONS.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
}
