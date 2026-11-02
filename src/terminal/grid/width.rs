//! Display width of characters (Unicode East Asian Width + emoji presentation,
//! macOS `wcwidth` for spacing marks) and the per-cell wide check used while printing.

use super::*;

/// Display width of a character: 0 (combining / zero-width), 1, or 2 (CJK / emoji),
/// following the Unicode East Asian Width and emoji presentation tables.
pub fn char_width(c: char) -> usize {
    use unicode_width::UnicodeWidthChar;
    // Regional indicators have emoji presentation: two columns each, as in
    // zsh, kitty and Ghostty (a flag pair joins into one two-column cluster).
    if (0x1F1E6..=0x1F1FF).contains(&(c as u32)) {
        return 2;
    }
    let w = c.width().unwrap_or(0).min(2);
    #[cfg(target_os = "macos")]
    if w == 1 && is_spacing_mark_range(c as u32) && macos_wcwidth(c) == Some(0) {
        return 0;
    }
    w
}

/// Scripts with spacing combining marks (Indic, Southeast Asian, Brahmic).
#[cfg(target_os = "macos")]
pub(super) fn is_spacing_mark_range(cp: u32) -> bool {
    matches!(cp, 0x0900..=0x0DFF | 0x0F00..=0x109F | 0x1700..=0x1CFF | 0xA800..=0xABFF | 0x11000..=0x11FFF)
}

/// What macOS's own `wcwidth` says. Shells (zsh, bash, readline) count with it, so
/// for spacing marks such as Devanagari U+093F, which it treats as zero-width,
/// disagreeing would leave the line editor's cursor columns off.
#[cfg(target_os = "macos")]
pub(super) fn macos_wcwidth(c: char) -> Option<usize> {
    use std::sync::OnceLock;
    unsafe extern "C" {
        fn wcwidth_l(wc: libc::wchar_t, loc: libc::locale_t) -> libc::c_int;
    }
    static LOCALE: OnceLock<usize> = OnceLock::new();
    // SAFETY: newlocale gets a valid NUL-terminated name and a null base; the
    // locale is never freed, so the pointer stays valid for wcwidth_l.
    let loc = *LOCALE.get_or_init(|| unsafe {
        libc::newlocale(libc::LC_CTYPE_MASK, c"en_US.UTF-8".as_ptr(), std::ptr::null_mut()) as usize
    });
    if loc == 0 {
        return None;
    }
    // SAFETY: loc is a live locale from newlocale.
    let w = unsafe { wcwidth_l(c as libc::wchar_t, loc as libc::locale_t) };
    usize::try_from(w).ok()
}

/// Smallest codepoint `char_width` reports as double-width (Hangul Jamo).
pub(super) const FIRST_WIDE: u32 = 0x1100;

/// Whether a cell's codepoint is a double-width character. Called for every
/// printed character, so ASCII and other narrow text skip the width tables.
#[inline]
pub fn is_wide_cp(cp: u32) -> bool {
    if cp & (CLUSTER_FLAG | PH_FLAG) == CLUSTER_FLAG {
        return cp & CLUSTER_WIDE != 0;
    }
    // Fast-reject common narrow ranges above FIRST_WIDE (0x1100):
    // Vietnamese Latin Extended Additional (0x1EA0..=0x1EF9),
    // Box Drawing and Block Elements (0x2500..=0x259F).
    if (0x1EA0..=0x1EF9).contains(&cp) || (0x2500..=0x259F).contains(&cp) {
        return false;
    }
    cp >= FIRST_WIDE && char::from_u32(cp).is_some_and(|c| char_width(c) == 2)
}

/// Extended pictographic (emoji) codepoints that join a ZWJ sequence.
pub(super) fn is_pictographic(cp: u32) -> bool {
    matches!(cp, 0x00A9 | 0x00AE | 0x203C | 0x2049 | 0x2122 | 0x2139 | 0x2194..=0x21AA | 0x231A..=0x23FF
        | 0x24C2 | 0x25AA..=0x25FE | 0x2600..=0x27BF | 0x2934 | 0x2935 | 0x2B05..=0x2B55 | 0x3030 | 0x303D
        | 0x3297 | 0x3299 | 0x1F000..=0x1FAFF)
}
