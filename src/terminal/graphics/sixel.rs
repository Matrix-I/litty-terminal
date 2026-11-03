//! Sixel decoder: DCS body to RGBA, bounded by [`MAX_SIDE`] and [`MAX_PIXELS`].

use super::{MAX_PIXELS, MAX_SIDE};

/// VT340 default colors for the first 16 registers.
const SIXEL_PALETTE: [u32; 16] = [
    0x000000, 0x3333CC, 0xCC2121, 0x33CC33, 0xCC33CC, 0x33CCCC, 0xCCCC33, 0x878787, 0x424242, 0x545499, 0x994242,
    0x549954, 0x995499, 0x549999, 0x999954, 0xCCCCCC,
];

/// Decode sixel data (the DCS body after `q`). `transparent` is DCS P2 = 1:
/// pixels never drawn stay transparent instead of taking color 0.
pub fn decode_sixel(data: &[u8], transparent: bool) -> Option<(u32, u32, Vec<u8>)> {
    let mut palette = [0u32; 256];
    palette[..16].copy_from_slice(&SIXEL_PALETTE);
    // Pixels as palette color + 1 (0 = never drawn), grown as needed. `u16`
    // holds all 257 values at half the memory of `u32`.
    let (mut w, mut h) = (0usize, 0usize);
    let (mut max_w, mut max_h) = (0usize, 0usize);
    let mut px: Vec<u16> = Vec::new();
    let (mut x, mut band) = (0usize, 0usize);
    let mut color = 0usize;
    // Up to eight full-canvas passes: allow normal multi-color images while
    // bounding tiny streams which repeatedly overpaint a single band.
    let mut work_left = MAX_PIXELS / 6 * 8;
    let mut i = 0;
    let num = |i: &mut usize| -> u32 {
        let mut n = 0u32;
        while *i < data.len() && data[*i].is_ascii_digit() {
            n = n.saturating_mul(10).saturating_add((data[*i] - b'0') as u32);
            *i += 1;
        }
        n
    };
    let params = |i: &mut usize| -> [u32; 5] {
        let mut p = [0u32; 5];
        for (k, slot) in p.iter_mut().enumerate() {
            *slot = num(i);
            if k < 4 && *i < data.len() && data[*i] == b';' {
                *i += 1;
            } else {
                break;
            }
        }
        p
    };
    let mut canvas = SixelCanvas {
        px: &mut px,
        w: &mut w,
        h: &mut h,
        max_w: &mut max_w,
        max_h: &mut max_h,
    };
    while i < data.len() {
        let b = data[i];
        match b {
            b'"' => {
                i += 1;
                let p = params(&mut i);
                // Raster attributes: aspect, then the size hint Ph x Pv. A hint
                // above the pixel cap is ignored (a 30-byte "8192x8192" bomb).
                let (pw, ph) = (p[2] as usize, p[3] as usize);
                if pw > 0
                    && ph > 0
                    && pw <= MAX_SIDE as usize
                    && ph <= MAX_SIDE as usize
                    && pw * ph <= MAX_PIXELS
                    && canvas.px.is_empty()
                {
                    *canvas.w = pw;
                    *canvas.h = ph;
                    *canvas.max_w = pw;
                    *canvas.max_h = ph;
                    *canvas.px = vec![0; pw * ph];
                }
            }
            b'#' => {
                i += 1;
                let p = params(&mut i);
                color = (p[0] as usize).min(255);
                if p[1] == 2 {
                    let pc = |v: u32| (v.min(100) * 255 / 100) & 0xFF;
                    palette[color] = (pc(p[2]) << 16) | (pc(p[3]) << 8) | pc(p[4]);
                } else if p[1] == 1 {
                    palette[color] = hls_to_rgb(p[2], p[3], p[4]);
                }
            }
            b'!' => {
                i += 1;
                let n = num(&mut i).max(1) as usize;
                if i < data.len() && (0x3F..=0x7E).contains(&data[i]) {
                    let bits = data[i] - 0x3F;
                    i += 1;
                    let count = n.min(MAX_SIDE as usize);
                    work_left = work_left.checked_sub(count)?;
                    for _ in 0..count {
                        canvas.put(x, band, bits, color)?;
                        x += 1;
                    }
                }
            }
            b'$' => {
                x = 0;
                i += 1;
            }
            b'-' => {
                x = 0;
                band += 1;
                i += 1;
            }
            0x3F..=0x7E => {
                work_left = work_left.checked_sub(1)?;
                canvas.put(x, band, b - 0x3F, color)?;
                x += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    if max_w == 0 || max_h == 0 {
        return None;
    }
    let mut rgba = Vec::with_capacity(max_w * max_h * 4);
    for y in 0..max_h {
        for x in 0..max_w {
            match px[y * w + x] {
                0 if transparent => rgba.extend_from_slice(&[0, 0, 0, 0]),
                0 => rgba.extend_from_slice(&rgb_bytes(palette[0])),
                c => rgba.extend_from_slice(&rgb_bytes(palette[(c - 1) as usize])),
            }
        }
    }
    Some((max_w as u32, max_h as u32, rgba))
}

fn rgb_bytes(c: u32) -> [u8; 4] {
    [(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]
}

struct SixelCanvas<'a> {
    px: &'a mut Vec<u16>,
    w: &'a mut usize,
    h: &'a mut usize,
    max_w: &'a mut usize,
    max_h: &'a mut usize,
}

impl SixelCanvas<'_> {
    /// Set the six pixels of one sixel, growing the canvas geometrically when needed.
    fn put(&mut self, x: usize, band: usize, bits: u8, color: usize) -> Option<()> {
        let need_w = x + 1;
        let need_h = band * 6 + 6;
        if need_w > MAX_SIDE as usize || need_h > MAX_SIDE as usize {
            return None;
        }
        if need_w > *self.w || need_h > *self.h {
            let mut nw = need_w.max(*self.w).max(64).next_power_of_two().min(MAX_SIDE as usize).max(need_w);
            let mut nh = need_h.max(*self.h).max(64).next_power_of_two().min(MAX_SIDE as usize).max(need_h);
            if nw * nh > MAX_PIXELS {
                // Geometric growth would overshoot the cap: grow exactly instead.
                (nw, nh) = (need_w.max(*self.w), need_h.max(*self.h));
                if nw * nh > MAX_PIXELS {
                    return None;
                }
            }
            let mut grown = vec![0u16; nw * nh];
            for y in 0..*self.max_h {
                grown[y * nw..y * nw + *self.max_w].copy_from_slice(&self.px[y * *self.w..y * *self.w + *self.max_w]);
            }
            *self.px = grown;
            *self.w = nw;
            *self.h = nh;
        }
        *self.max_w = (*self.max_w).max(need_w);
        *self.max_h = (*self.max_h).max(need_h);
        for k in 0..6 {
            if bits & (1 << k) != 0 {
                self.px[(band * 6 + k) * *self.w + x] = color as u16 + 1;
            }
        }
        Some(())
    }
}

/// Sixel HLS (hue 0..360 with blue at 0, lightness and saturation 0..100).
fn hls_to_rgb(h: u32, l: u32, s: u32) -> u32 {
    let (h, l, s) = (((h + 240) % 360) as f32 / 360.0, l.min(100) as f32 / 100.0, s.min(100) as f32 / 100.0);
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let channel = |t: f32| {
        let t = t.rem_euclid(1.0);
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round() as u32
    };
    (channel(h + 1.0 / 3.0) << 16) | (channel(h) << 8) | channel(h - 1.0 / 3.0)
}
