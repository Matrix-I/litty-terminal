//! Kitty graphics protocol: command parsing, chunked transmission, reply
//! formatting and the CPU-side decoders (raw RGB/RGBA, PNG, file and
//! shared-memory media).

use std::time::{Duration, Instant};

use super::store::Store;
use super::{MAX_PIXELS, MAX_SIDE, STORE_LIMIT};

/// The `key=value` control data of a Kitty graphics command.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KittyCommand {
    pub action: u8,
    pub quiet: u8,
    pub format: u32,
    pub medium: u8,
    pub compression: u8,
    pub more: bool,
    pub id: u32,
    pub number: u32,
    pub placement_id: u32,
    pub width: u32,
    pub height: u32,
    pub x: u32,
    pub y: u32,
    pub src_w: u32,
    pub src_h: u32,
    pub cols: u32,
    pub rows: u32,
    pub z: i32,
    pub no_cursor_move: bool,
    /// `U=1`: a virtual placement shown through unicode placeholders.
    pub unicode: bool,
    /// `X=1` (`a=f`): replace the frame's pixels instead of blending over them.
    pub replace: bool,
    /// `X=` / `Y=` (`a=c`): top-left of the source rectangle; `C=1`: overwrite.
    pub src_left: u32,
    pub src_top: u32,
    pub overwrite: bool,
    /// `S=` / `O=`: bytes to read and where to start, for file and shared memory.
    pub size: u32,
    pub offset: u32,
    /// `d=` delete selector.
    pub delete: u8,
}

/// Parse `G<control>;<payload>` (the APC body after `ESC _`).
pub fn parse_kitty(body: &[u8]) -> Option<(KittyCommand, &[u8])> {
    let body = body.strip_prefix(b"G")?;
    let (control, payload) = match body.iter().position(|&b| b == b';') {
        Some(i) => (&body[..i], &body[i + 1..]),
        None => (body, &b""[..]),
    };
    let mut c = KittyCommand { action: b't', format: 32, medium: b'd', ..Default::default() };
    for kv in control.split(|&b| b == b',') {
        let Some(eq) = kv.iter().position(|&b| b == b'=') else { continue };
        let (k, v) = (&kv[..eq], &kv[eq + 1..]);
        let num = || std::str::from_utf8(v).ok().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
        let ch = v.first().copied().unwrap_or(0);
        match k {
            b"a" => c.action = ch,
            b"q" => c.quiet = num() as u8,
            b"f" => c.format = num(),
            b"t" => c.medium = ch,
            b"o" => c.compression = ch,
            b"m" => c.more = num() == 1,
            b"i" => c.id = num(),
            b"I" => c.number = num(),
            b"p" => c.placement_id = num(),
            b"s" => c.width = num(),
            b"v" => c.height = num(),
            b"x" => c.x = num(),
            b"y" => c.y = num(),
            b"w" => c.src_w = num(),
            b"h" => c.src_h = num(),
            b"c" => c.cols = num(),
            b"r" => c.rows = num(),
            b"z" => c.z = std::str::from_utf8(v).ok().and_then(|s| s.parse().ok()).unwrap_or(0),
            // `C=1`: no cursor movement when placing; overwrite when composing (`a=c`).
            b"C" => {
                c.no_cursor_move = num() == 1;
                c.overwrite = c.no_cursor_move;
            }
            b"U" => c.unicode = num() == 1,
            b"X" => {
                c.replace = num() == 1;
                c.src_left = num();
            }
            b"Y" => c.src_top = num(),
            b"S" => c.size = num(),
            b"O" => c.offset = num(),
            b"d" => c.delete = ch,
            _ => {}
        }
    }
    Some((c, payload))
}

/// What the terminal must do after a Kitty command.
pub enum KittyOutcome {
    /// Nothing visible (transmit only, delete, chunk in progress).
    None,
    /// Show image `id` at the cursor.
    Place { id: u32 },
}

pub(crate) type DecodedImage = Result<(u32, u32, Vec<u8>), String>;

pub(crate) enum KittyPreparation {
    Complete(KittyOutcome, Option<String>),
    Decode(KittyCommand, Vec<u8>),
}

fn kitty_reply(c: &KittyCommand, msg: &str) -> Option<String> {
    let ok = msg == "OK";
    if (c.id == 0 && c.number == 0) || (ok && c.quiet >= 1) || c.quiet >= 2 {
        return None;
    }
    let mut r = format!("i={}", c.id);
    if c.number != 0 {
        r.push_str(&format!(",I={}", c.number));
    }
    if c.placement_id != 0 {
        r.push_str(&format!(",p={}", c.placement_id));
    }
    Some(format!("{r};{msg}"))
}

impl Store {
    /// Run a Kitty command on the main screen. Returns the outcome and the
    /// reply body (`OK` or an error such as `ENOENT:...`), already filtered by `q=`.
    #[cfg(test)]
    pub fn kitty(&mut self, cmd: KittyCommand, payload: &[u8]) -> (KittyOutcome, Option<String>) {
        self.kitty_on(cmd, payload, false)
    }

    /// [`Store::kitty`] on the main (`alt` false) or alternate screen.
    #[cfg(test)]
    pub fn kitty_on(&mut self, cmd: KittyCommand, payload: &[u8], alt: bool) -> (KittyOutcome, Option<String>) {
        match self.prepare_kitty(cmd, payload, alt) {
            KittyPreparation::Complete(outcome, reply) => (outcome, reply),
            KittyPreparation::Decode(cmd, data) => {
                let decoded = decode_kitty(&cmd, &data);
                self.finish_kitty(cmd, decoded, alt)
            }
        }
    }

    /// Assemble chunks under the grid lock; the returned decode runs without it.
    /// `alt`: the alternate screen is active (scopes deletions).
    pub(crate) fn prepare_kitty(&mut self, cmd: KittyCommand, payload: &[u8], alt: bool) -> KittyPreparation {
        // Abandoned chunked transmission timeout: free buffers if sender stalled > 5s.
        const CHUNKED_TIMEOUT: Duration = Duration::from_secs(5);
        let now = Instant::now();
        if let Some((_, _, last_active)) = self.pending
            && now.duration_since(last_active) > CHUNKED_TIMEOUT {
                self.pending = None;
            }

        // Chunked transmissions: collect until `m=0`; only the first chunk has the keys.
        let (cmd, data) = match self.pending.take() {
            Some((first, mut data, _)) if matches!(first.action, b't' | b'T' | b'q' | b'f') => {
                data.extend_from_slice(payload);
                if data.len() > STORE_LIMIT {
                    return KittyPreparation::Complete(KittyOutcome::None, kitty_reply(&first, "EBIG:chunked transmission exceeds size limit"));
                }
                if cmd.more {
                    self.pending = Some((first, data, now));
                    return KittyPreparation::Complete(KittyOutcome::None, None);
                }
                (first, data)
            }
            _ if cmd.more && matches!(cmd.action, b't' | b'T' | b'q' | b'f') => {
                if payload.len() > STORE_LIMIT {
                    return KittyPreparation::Complete(KittyOutcome::None, kitty_reply(&cmd, "EBIG:chunked transmission exceeds size limit"));
                }
                self.pending = Some((cmd, payload.to_vec(), now));
                return KittyPreparation::Complete(KittyOutcome::None, None);
            }
            _ => (cmd, payload.to_vec()),
        };
        if matches!(cmd.action, b't' | b'T' | b'q' | b'f') {
            let mut cmd = cmd;
            if cmd.action == b'f' {
                let Some(img) = self.images.get(&cmd.id) else {
                    return KittyPreparation::Complete(KittyOutcome::None, kitty_reply(&cmd, "ENOENT:image not found"));
                };
                if cmd.width == 0 { cmd.width = img.width; }
                if cmd.height == 0 { cmd.height = img.height; }
            }
            return KittyPreparation::Decode(cmd, data);
        }
        let (outcome, reply) = self.finish_kitty(cmd, Err(String::new()), alt);
        KittyPreparation::Complete(outcome, reply)
    }

    pub(crate) fn finish_kitty(&mut self, cmd: KittyCommand, decoded: DecodedImage, alt: bool) -> (KittyOutcome, Option<String>) {
        match cmd.action {
            b't' | b'T' | b'q' => match decoded {
                Ok((w, h, rgba)) => {
                    if cmd.action == b'q' {
                        return (KittyOutcome::None, kitty_reply(&cmd, "OK"));
                    }
                    let id = self.insert(cmd.id, w, h, rgba);
                    if let Some(img) = self.images.get_mut(&id) {
                        img.alt = alt;
                    }
                    let out = if cmd.action == b'T' { KittyOutcome::Place { id } } else { KittyOutcome::None };
                    let mut reply_cmd = cmd;
                    if reply_cmd.number != 0 {
                        reply_cmd.id = id;
                    }
                    (out, kitty_reply(&reply_cmd, "OK"))
                }
                Err(e) => (KittyOutcome::None, kitty_reply(&cmd, &e)),
            },
            b'p' => {
                if self.images.contains_key(&cmd.id) {
                    (KittyOutcome::Place { id: cmd.id }, kitty_reply(&cmd, "OK"))
                } else {
                    (KittyOutcome::None, kitty_reply(&cmd, "ENOENT:image not found"))
                }
            }
            b'd' => {
                if !matches!(cmd.delete, 0 | b'a' | b'A' | b'i' | b'I') {
                    return (KittyOutcome::None, kitty_reply(&cmd, "EINVAL:unsupported delete selector"));
                }
                self.kitty_delete(&cmd, alt);
                (KittyOutcome::None, None)
            }
            b'f' => match self.kitty_frame(&cmd, decoded) {
                Ok(()) => (KittyOutcome::None, kitty_reply(&cmd, "OK")),
                Err(e) => (KittyOutcome::None, kitty_reply(&cmd, &e)),
            },
            b'c' => match self.kitty_compose(&cmd) {
                Ok(()) => (KittyOutcome::None, kitty_reply(&cmd, "OK")),
                Err(e) => (KittyOutcome::None, kitty_reply(&cmd, &e)),
            },
            b'a' => match self.kitty_animate(&cmd, Instant::now()) {
                Ok(()) => (KittyOutcome::None, kitty_reply(&cmd, "OK")),
                Err(e) => (KittyOutcome::None, kitty_reply(&cmd, &e)),
            },
            _ => (KittyOutcome::None, kitty_reply(&cmd, "EINVAL:unsupported action")),
        }
    }

    /// `a=d`: lowercase keeps the image data, uppercase frees it too. Kitty
    /// keeps one image store per screen, so only the current screen's
    /// placements go, and only that screen's images that nothing shows any
    /// more are freed.
    fn kitty_delete(&mut self, cmd: &KittyCommand, alt: bool) {
        let (selector, id, pid) = (cmd.delete, cmd.id, cmd.placement_id);
        let hit = |image_id: u32, placement_id: u32| match selector {
            b'i' | b'I' => image_id == id && (pid == 0 || placement_id == pid),
            // Only `a`/`A` (and the omitted default) reach this arm;
            // unsupported selectors are rejected before any mutation.
            _ => true,
        };
        self.placements.retain(|p| p.alt != alt || !hit(p.image_id, p.placement_id));
        self.virtuals.retain(|v| v.alt != alt || !hit(v.image_id, v.placement_id));
        if selector.is_ascii_uppercase() {
            self.free_unshown(alt, (selector == b'I').then_some(id));
        }
    }
}

fn is_temp_dir(dir: &std::path::Path) -> bool {
    let temp_dirs = [
        std::env::temp_dir(),
        std::path::PathBuf::from("/tmp"),
        std::path::PathBuf::from("/var/tmp"),
        #[cfg(target_os = "macos")]
        std::path::PathBuf::from("/private/tmp"),
        #[cfg(target_os = "macos")]
        std::path::PathBuf::from("/private/var/tmp"),
    ];
    let Ok(canonical_dir) = dir.canonicalize() else {
        return false;
    };
    temp_dirs.iter().any(|td| {
        td.canonicalize().is_ok_and(|ctd| canonical_dir.starts_with(&ctd))
    })
}

/// The one reply for every failed `t=f` / `t=t` read or decode. Distinct errors
/// (`ENOENT` vs `EPERM` vs `EINVAL`) let any program that can print to the
/// terminal probe which private files exist (CWE-200).
const FILE_DENIED: &str = "EPERM:access denied";

/// Resolve `.` and `..` without touching the filesystem.
fn lexical_normalize(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn is_sensitive_path(path: &std::path::Path) -> bool {
    for c in path.components() {
        let name = c.as_os_str().to_string_lossy();
        if name.starts_with('.') && name != "." && name != ".." {
            return true;
        }
    }
    // APFS and HFS+ are case-insensitive by default: `/ETC/hosts` is `/etc/hosts`.
    let path_str = if cfg!(target_os = "macos") {
        path.to_string_lossy().to_lowercase()
    } else {
        path.to_string_lossy().into_owned()
    };
    let under = |prefix: &str| {
        path_str.strip_prefix(prefix).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    };
    if ["/etc", "/private/etc", "/dev", "/proc", "/sys", "/var/root", "/root", "/private/var/root"]
        .into_iter()
        .any(under)
    {
        return true;
    }
    if cfg!(target_os = "macos") {
        // Keychains, cookies, mail, messages and app containers.
        if ["/library/keychains", "/private/var/db"].into_iter().any(under) {
            return true;
        }
        if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
            let lib = std::path::Path::new(&home).join("Library").to_string_lossy().to_lowercase();
            if under(&lib) {
                return true;
            }
        }
    }
    false
}

/// Read a `t=f` (file) or `t=t` (temporary file) transmission. Every
/// failure, including a missing file, returns [`FILE_DENIED`].
fn read_file_medium(c: &KittyCommand, raw: Vec<u8>) -> Result<Vec<u8>, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let denied = || FILE_DENIED.to_string();
    let path_str = String::from_utf8(raw).map_err(|_| denied())?;
    let path = std::path::Path::new(&path_str);
    // A relative path would resolve against Litty's own working directory.
    if !path.is_absolute() {
        return Err(denied());
    }
    // Reject sensitive locations before any filesystem access, so that the
    // answer cannot depend on whether the file exists.
    let lexical = lexical_normalize(path);
    if c.medium == b'f' && is_sensitive_path(&lexical) {
        return Err(denied());
    }
    let canonical = path.canonicalize().map_err(|_| denied())?;
    if c.medium == b't' {
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !file_name.contains("tty-graphics-protocol")
            || !path.parent().is_some_and(is_temp_dir)
            || !canonical.parent().is_some_and(is_temp_dir)
        {
            return Err(denied());
        }
    } else if is_sensitive_path(&canonical) {
        return Err(denied());
    }
    // O_NOFOLLOW: the canonical path has no symlinks, so one appearing now is a
    // swap. O_NONBLOCK: never hang on a FIFO. The type and size are checked on
    // the opened descriptor, not on the path (no TOCTOU window).
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&canonical)
        .map_err(|_| denied())?;
    let meta = file.metadata().map_err(|_| denied())?;
    if !meta.is_file() || meta.len() > STORE_LIMIT as u64 {
        return Err(denied());
    }
    let data = read_range(file, c.offset, c.size).map_err(|_| denied())?;
    // Temporary files are deleted once read, but only verified temp files.
    if c.medium == b't' {
        let _ = std::fs::remove_file(&canonical);
    }
    Ok(data)
}

/// Decode a transmission into `(width, height, rgba)`.
pub(crate) fn decode_kitty(c: &KittyCommand, payload: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    if matches!(c.format, 24 | 32) {
        let (w, h) = (c.width, c.height);
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || (w as usize) * (h as usize) > MAX_PIXELS {
            return Err("EINVAL:bad image size".into());
        }
    }
    let raw = base64_decode(payload).ok_or("EINVAL:bad base64")?;
    let data = match c.medium {
        b'd' => raw,
        b'f' | b't' => read_file_medium(c, raw)?,
        b's' => {
            // POSIX shared memory, unlinked once read (as kitty does).
            let name_str = String::from_utf8(raw.clone()).map_err(|_| "EINVAL:bad name")?;
            let valid_prefix = name_str.starts_with("/kitty-")
                || name_str.starts_with("/litty-")
                || name_str.starts_with("/ghostty-")
                || name_str.starts_with("/shm-");
            if !valid_prefix {
                return Err("EPERM:unauthorized shared memory prefix".into());
            }

            let name = std::ffi::CString::new(raw).map_err(|_| "EINVAL:bad name")?;
            // SAFETY: a NUL-terminated name; the descriptor is owned by `file`.
            let fd = unsafe { libc::shm_open(name.as_ptr(), libc::O_RDONLY, 0) };
            if fd < 0 {
                return Err("ENOENT:shared memory not found".into());
            }
            // Mapped, since macOS cannot read() shared memory objects.
            // SAFETY: a read-only mapping dropped before the object is unlinked and closed.
            let data = unsafe {
                let mut st: libc::stat = core::mem::zeroed();
                let res = if libc::fstat(fd, &mut st) != 0 || st.st_size < 0 {
                    Err("EIO:cannot map shared memory".to_string())
                } else {
                    let map_len = st.st_size as usize;
                    if map_len == 0 {
                        Ok(Vec::new())
                    } else {
                        let ptr = libc::mmap(core::ptr::null_mut(), map_len, libc::PROT_READ, libc::MAP_SHARED, fd, 0);
                        if ptr == libc::MAP_FAILED {
                            Err("EIO:cannot map shared memory".to_string())
                        } else {
                            let start = (c.offset as usize).min(map_len);
                            let end = if c.size == 0 { map_len } else { (start + c.size as usize).min(map_len) };
                            let res = if end - start > STORE_LIMIT {
                                Err("EINVAL:too large".to_string())
                            } else {
                                let slice = core::slice::from_raw_parts((ptr as *const u8).add(start), end - start);
                                Ok(slice.to_vec())
                            };
                            libc::munmap(ptr, map_len);
                            res
                        }
                    }
                };
                libc::shm_unlink(name.as_ptr());
                libc::close(fd);
                res
            };
            data?
        }
        _ => return Err("EINVAL:unsupported transmission medium".into()),
    };
    decode_kitty_pixels(c, data).map_err(|error| {
        if matches!(c.medium, b'f' | b't') { FILE_DENIED.to_string() } else { error }
    })
}

/// Decode bytes after reading the medium so file failures share one reply.
fn decode_kitty_pixels(c: &KittyCommand, data: Vec<u8>) -> DecodedImage {
    let data = if c.compression == b'z' {
        super::png::zlib_decompress_with_limit(&data, STORE_LIMIT).map_err(String::from)?
    } else {
        data
    };
    match c.format {
        100 => super::png::decode_png(&data),
        24 | 32 => {
            let (w, h) = (c.width, c.height);
            let bpp = if c.format == 24 { 3 } else { 4 };
            if w == 0
                || h == 0
                || w > MAX_SIDE
                || h > MAX_SIDE
                || (w as usize) * (h as usize) > MAX_PIXELS
                || data.len() < (w * h * bpp) as usize
            {
                return Err("EINVAL:bad image size".into());
            }
            let rgba = if bpp == 4 {
                data[..(w * h * 4) as usize].to_vec()
            } else {
                data[..(w * h * 3) as usize].as_chunks::<3>().0.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect()
            };
            Ok((w, h, rgba))
        }
        _ => Err("EINVAL:unsupported format".into()),
    }
}

/// Put RGBA `src` on `dst`: a copy, or source-over blending (straight alpha).
pub(super) fn compose_pixel(dst: &mut [u8], src: &[u8], replace: bool) {
    if replace {
        dst.copy_from_slice(src);
        return;
    }
    let (sa, da) = (src[3] as u32, dst[3] as u32);
    let out_a = sa + da * (255 - sa) / 255;
    for k in 0..3 {
        if let Some(c) = (src[k] as u32 * sa + dst[k] as u32 * da * (255 - sa) / 255).checked_div(out_a) {
            dst[k] = c.min(255) as u8;
        }
    }
    dst[3] = out_a as u8;
}

/// Read `size` bytes (0 = to the end) from `offset`, within the store limit.
fn read_range(file: std::fs::File, offset: u32, size: u32) -> Result<Vec<u8>, String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = file;
    file.seek(SeekFrom::Start(offset as u64)).map_err(|_| "EINVAL:bad offset")?;
    let limit = if size == 0 { STORE_LIMIT as u64 } else { (size as u64).min(STORE_LIMIT as u64) };
    let mut data = Vec::new();
    file.take(limit).read_to_end(&mut data).map_err(|_| "EIO:cannot read")?;
    Ok(data)
}

/// Standard base64 (padding optional, whitespace ignored).
pub(super) fn base64_decode(s: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for &b in s {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}
