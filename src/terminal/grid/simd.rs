//! Architecture-specific SIMD fast paths for the terminal grid:
//! - 64-byte / 32-byte printable-ASCII prefix scanning (`0x20..=0x7E`)
//! - Vectorized `u8` → `12-byte Cell` row splatting
//! - Compact 512-bit stack bitset (`BitSet512`, `[u64; 8]`) for zero-heap row
//!   dirty flags and column tab stops

use super::{Cell, is_wide_cp};

/// Fixed 512-bit stack bitset (`[u64; 8]`, 64 bytes) replacing heap-allocated
/// `Vec<bool>` for visible row dirty tracking and column tab stops.
/// Coordinates `>= 512` saturate/clamp safely.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct BitSet512 {
    words: [u64; 8],
}

impl BitSet512 {
    pub const EMPTY: Self = Self { words: [0; 8] };

    /// Bitset with bits `0..len` set to 1.
    #[inline]
    pub fn ones(len: usize) -> Self {
        let mut set = Self::EMPTY;
        let len = len.min(512);
        let full = len / 64;
        for w in &mut set.words[..full] {
            *w = u64::MAX;
        }
        let rem = len % 64;
        if rem > 0 {
            set.words[full] = (1u64 << rem) - 1;
        }
        set
    }

    /// Default 8-column tab stops (`col > 0 && col % 8 == 0`) up to `cols`.
    #[inline]
    pub fn default_tabs(cols: usize) -> Self {
        let mut set = Self::EMPTY;
        let cols = cols.min(512);
        // Every 8th bit set: 0x0101_0101_0101_0101
        const TAB_WORD: u64 = 0x0101_0101_0101_0101;
        let full = cols / 64;
        for w in &mut set.words[..full] {
            *w = TAB_WORD;
        }
        let rem = cols % 64;
        if rem > 0 {
            set.words[full] = TAB_WORD & ((1u64 << rem) - 1);
        }
        // Column 0 is never a tab stop.
        set.words[0] &= !1u64;
        set
    }

    #[inline(always)]
    pub fn set(&mut self, idx: usize, val: bool) {
        if idx < 512 {
            let w = idx >> 6;
            let b = idx & 63;
            let mask = 1u64 << b;
            if val {
                self.words[w] |= mask;
            } else {
                self.words[w] &= !mask;
            }
        }
    }

    #[inline(always)]
    pub fn get(&self, idx: usize) -> bool {
        if idx < 512 {
            (self.words[idx >> 6] >> (idx & 63)) & 1 != 0
        } else {
            false
        }
    }

    #[inline(always)]
    pub fn any(&self) -> bool {
        (self.words[0]
            | self.words[1]
            | self.words[2]
            | self.words[3]
            | self.words[4]
            | self.words[5]
            | self.words[6]
            | self.words[7])
            != 0
    }

    #[inline(always)]
    pub fn clear(&mut self) {
        self.words = [0; 8];
    }

    /// First set bit strictly after `after` and below `limit`, if any.
    #[inline]
    pub fn next_set(&self, after: usize, limit: usize) -> Option<usize> {
        let start = after + 1;
        let limit = limit.min(512);
        if start >= limit {
            return None;
        }
        let mut w = start >> 6;
        let b = start & 63;
        let mut word = self.words[w] & (!0u64 << b);
        loop {
            if word != 0 {
                let idx = (w << 6) + word.trailing_zeros() as usize;
                return (idx < limit).then_some(idx);
            }
            w += 1;
            if (w << 6) >= limit {
                return None;
            }
            word = self.words[w];
        }
    }

    /// Last set bit strictly before `before` and at or above `min_idx`, if any.
    #[inline]
    pub fn prev_set(&self, before: usize, min_idx: usize) -> Option<usize> {
        let before = before.min(512);
        if before <= min_idx || before == 0 {
            return None;
        }
        let last = before - 1;
        let mut w = last >> 6;
        let b = last & 63;
        let mask = if b == 63 { !0u64 } else { (1u64 << (b + 1)) - 1 };
        let mut word = self.words[w] & mask;
        loop {
            if word != 0 {
                let idx = (w << 6) + (63 - word.leading_zeros() as usize);
                return (idx >= min_idx).then_some(idx);
            }
            if w == 0 || (w << 6) <= min_idx {
                return None;
            }
            w -= 1;
            word = self.words[w];
        }
    }
}

/// Length of the leading run of printable ASCII bytes (`0x20..=0x7E`).
/// Uses explicit `NEON` on `aarch64` and `AVX2`/`SSE2` on `x86_64`.
#[inline]
pub fn printable_ascii_prefix(bytes: &[u8]) -> usize {
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON is mandatory on all aarch64 targets.
        unsafe { printable_ascii_prefix_neon(bytes) }
    }
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: SSE2 is baseline on all x86_64 targets.
        unsafe { printable_ascii_prefix_sse2(bytes) }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        printable_ascii_prefix_scalar(bytes)
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn printable_ascii_prefix_neon(bytes: &[u8]) -> usize {
    use std::arch::aarch64::*;
    let len = bytes.len();
    let ptr = bytes.as_ptr();
    let mut i = 0;

    let v_20 = vdupq_n_u8(0x20);
    let v_5f = vdupq_n_u8(0x5f);

    // 64-byte unrolled loop: 4 NEON registers per iteration.
    while i + 64 <= len {
        // SAFETY: `i + 64 <= len` bounds-checked above.
        unsafe {
            let p = ptr.add(i);
            let b0 = vld1q_u8(p);
            let b1 = vld1q_u8(p.add(16));
            let b2 = vld1q_u8(p.add(32));
            let b3 = vld1q_u8(p.add(48));

            // Unsigned wrap: `b.wrapping_sub(0x20) >= 0x5f` catches `< 0x20` and `>= 0x7f`.
            let c0 = vcgeq_u8(vsubq_u8(b0, v_20), v_5f);
            let c1 = vcgeq_u8(vsubq_u8(b1, v_20), v_5f);
            let c2 = vcgeq_u8(vsubq_u8(b2, v_20), v_5f);
            let c3 = vcgeq_u8(vsubq_u8(b3, v_20), v_5f);

            let any01 = vorrq_u8(c0, c1);
            let any23 = vorrq_u8(c2, c3);
            let any = vorrq_u8(any01, any23);

            if vmaxvq_u8(any) != 0 {
                break;
            }
        }
        i += 64;
    }

    // 16-byte loop for remaining vector blocks.
    while i + 16 <= len {
        // SAFETY: `i + 16 <= len` bounds-checked above.
        unsafe {
            let b0 = vld1q_u8(ptr.add(i));
            let c0 = vcgeq_u8(vsubq_u8(b0, v_20), v_5f);
            if vmaxvq_u8(c0) != 0 {
                // Narrow down within this 16-byte vector via scalar tail.
                break;
            }
        }
        i += 16;
    }

    while i < len && bytes[i].wrapping_sub(0x20) < 0x5f {
        i += 1;
    }
    i
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn printable_ascii_prefix_sse2(bytes: &[u8]) -> usize {
    use std::arch::x86_64::*;
    let len = bytes.len();
    let ptr = bytes.as_ptr();
    let mut i = 0;

    // Signed byte trick: shift `[0x20, 0x7E]` to `[-128, -34]` by adding `-0x80 - 0x20 = -0xA0` (`0x60`),
    // then signed compare `> -34` (`-128 + 0x5f - 1 = -34`).
    let offset = _mm_set1_epi8(0x60);
    let bound = _mm_set1_epi8(-34);

    while i + 64 <= len {
        // SAFETY: `i + 64 <= len` bounds-checked above.
        unsafe {
            let p = ptr.add(i);
            let b0 = _mm_loadu_si128(p as *const __m128i);
            let b1 = _mm_loadu_si128(p.add(16) as *const __m128i);
            let b2 = _mm_loadu_si128(p.add(32) as *const __m128i);
            let b3 = _mm_loadu_si128(p.add(48) as *const __m128i);

            let c0 = _mm_cmpgt_epi8(_mm_add_epi8(b0, offset), bound);
            let c1 = _mm_cmpgt_epi8(_mm_add_epi8(b1, offset), bound);
            let c2 = _mm_cmpgt_epi8(_mm_add_epi8(b2, offset), bound);
            let c3 = _mm_cmpgt_epi8(_mm_add_epi8(b3, offset), bound);

            let any = _mm_or_si128(_mm_or_si128(c0, c1), _mm_or_si128(c2, c3));
            if _mm_movemask_epi8(any) != 0 {
                break;
            }
        }
        i += 64;
    }

    while i + 16 <= len {
        // SAFETY: `i + 16 <= len` bounds-checked above.
        unsafe {
            let b0 = _mm_loadu_si128(ptr.add(i) as *const __m128i);
            let c0 = _mm_cmpgt_epi8(_mm_add_epi8(b0, offset), bound);
            let mask = _mm_movemask_epi8(c0);
            if mask != 0 {
                return i + mask.trailing_zeros() as usize;
            }
        }
        i += 16;
    }

    while i < len && bytes[i].wrapping_sub(0x20) < 0x5f {
        i += 1;
    }
    i
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
#[inline]
fn printable_ascii_prefix_scalar(bytes: &[u8]) -> usize {
    let mut i = 0;
    while i < bytes.len() && bytes[i].wrapping_sub(0x20) < 0x5f {
        i += 1;
    }
    i
}

/// Write an ASCII slice `seg` into `dst` with `(fg, bg)`, returning `true` if
/// any overwritten cell held a non-zero `extra()` index.
#[inline(always)]
pub fn write_ascii_cells(dst: &mut [Cell], seg: &[u8], fg: u32, bg: u32, check_extras: bool) -> bool {
    debug_assert_eq!(dst.len(), seg.len());
    let mut prev_extra = false;
    if check_extras {
        for (cell, &b) in dst.iter_mut().zip(seg) {
            prev_extra |= cell.extra() != 0;
            *cell = Cell { codepoint: b as u32, fg, bg };
        }
    } else {
        for (cell, &b) in dst.iter_mut().zip(seg) {
            *cell = Cell { codepoint: b as u32, fg, bg };
        }
    }
    prev_extra
}

/// Check and blank wide-glyph halves orphaned at the edges of `col..=last`.
#[inline(always)]
pub fn fixup_wide_edges(row: &mut [Cell], col: usize, last: usize, cols: usize, blank: Cell, check_extras: bool) -> bool {
    let mut prev_extra = false;
    if col > 0 && row[col].codepoint == 0 && is_wide_cp(row[col - 1].codepoint) {
        if check_extras {
            prev_extra |= row[col - 1].extra() != 0;
        }
        row[col - 1] = blank;
    }
    if last + 1 < cols && row[last + 1].codepoint == 0 && is_wide_cp(row[last].codepoint) {
        if check_extras {
            prev_extra |= row[last + 1].extra() != 0;
        }
        row[last + 1] = blank;
    }
    prev_extra
}

/// Try to parse a complete semicolon-separated CSI sequence `\x1b[...<final>`
/// at the start of `bytes` into `out`, provided there are no private or
/// intermediate markers (`?`, `>`, `:`, etc.). Returns `None` for more than 16
/// params so `vte` (which keeps 32) handles them.
/// Returns `Some((bytes_consumed, param_count, final_byte))`.
#[inline]
pub fn try_parse_simple_csi(bytes: &[u8], out: &mut [u16; 16]) -> Option<(usize, usize, u8)> {
    if bytes.len() < 3 || bytes[0] != 0x1b || bytes[1] != b'[' {
        return None;
    }
    let b2 = bytes[2];
    // Fast single-byte no-param CSI: `\x1b[m`, `\x1b[K`, `\x1b[H`, `\x1b[J`.
    if matches!(b2, b'm' | b'K' | b'H' | b'f' | b'J' | b'A' | b'B' | b'C' | b'D' | b'G') {
        return Some((3, 0, b2));
    }
    let mut i = 2;
    let mut n = 0usize;
    let mut cur: u32 = 0;
    let limit = bytes.len().min(64);
    while i < limit {
        let b = bytes[i];
        if b.is_ascii_digit() {
            cur = (cur * 10 + (b - b'0') as u32).min(u16::MAX as u32);
            i += 1;
        } else if b == b';' || matches!(b, b'm' | b'K' | b'H' | b'f' | b'J' | b'A' | b'B' | b'C' | b'D' | b'G') {
            // More parameters than `out` holds: let `vte` (32 params) parse it
            // rather than silently dropping the tail.
            *out.get_mut(n)? = cur as u16;
            n += 1;
            if b != b';' {
                return Some((i + 1, n, b));
            }
            cur = 0;
            i += 1;
        } else {
            return None;
        }
    }
    None
}
