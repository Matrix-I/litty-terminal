//! SIMD substring and byte-set scanners (`NEON` on `aarch64`, `SSE2` on
//! `x86_64`, scalar elsewhere).
//!
//! A literal is found by testing two of its bytes, the rarest by a fixed
//! frequency heuristic, at their offsets across 16 positions per step, then
//! verifying only the lanes that hit. Case-insensitive search folds ASCII
//! letters with `| 0x20`, which is exact for letters: `b | 0x20 == b'a'` only
//! for `b'a'` and `b'A'`.

/// Two probe bytes of a needle: offsets and `(hay | mask) == byte` tests.
#[derive(Clone, Copy, Debug)]
struct Probe {
    i1: usize,
    i2: usize,
    b1: u8,
    m1: u8,
    b2: u8,
    m2: u8,
}

impl Probe {
    fn new(needle: &[u8], ci: bool) -> Self {
        let rarest = |skip: Option<usize>| (0..needle.len()).filter(|&i| Some(i) != skip).min_by_key(|&i| (rank(needle[i]), i));
        let i1 = rarest(None).unwrap_or(0);
        let i2 = rarest(Some(i1)).unwrap_or(i1);
        let test = |b: u8| if ci && b.is_ascii_alphabetic() { (b.to_ascii_lowercase(), 0x20) } else { (b, 0) };
        let ((b1, m1), (b2, m2)) = (test(needle[i1]), test(needle[i2]));
        Probe { i1, i2, b1, m1, b2, m2 }
    }

    #[inline(always)]
    fn hits(&self, hay: &[u8], pos: usize) -> bool {
        (hay[pos + self.i1] | self.m1) == self.b1 && (hay[pos + self.i2] | self.m2) == self.b2
    }
}

/// Rough byte frequency in terminal text; lower is rarer.
pub(super) fn rank(b: u8) -> u8 {
    match b {
        b' ' => 255,
        b'e' | b't' | b'a' | b'o' | b'i' | b'n' | b's' | b'r' | b'h' | b'l' => 220,
        b'a'..=b'z' => 180,
        b'0'..=b'9' => 170,
        b'\n' | b'.' | b',' | b'-' | b'_' | b'/' | b'=' | b':' => 150,
        b'A'..=b'Z' => 110,
        0x80.. => 100,
        _ => 60,
    }
}

/// A precomputed literal search, lowercased when `ci`.
#[derive(Clone, Debug)]
pub(super) struct LiteralFinder {
    needle: Vec<u8>,
    ci: bool,
    probe: Option<Probe>,
    /// One-byte needles scan as a byte set (both cases when `ci`).
    single: Option<ByteSet>,
}

impl LiteralFinder {
    pub(super) fn new(needle: &[u8], ci: bool) -> Self {
        let needle: Vec<u8> = if ci { needle.to_ascii_lowercase() } else { needle.to_vec() };
        let probe = (!needle.is_empty()).then(|| Probe::new(&needle, ci));
        let single = (needle.len() == 1).then(|| {
            let mut bits = [false; 256];
            bits[needle[0] as usize] = true;
            if ci {
                bits[needle[0].to_ascii_uppercase() as usize] = true;
            }
            ByteSet::new(bits)
        });
        LiteralFinder { needle, ci, probe, single }
    }

    pub(super) fn needle(&self) -> &[u8] {
        &self.needle
    }

    /// Offset of the first occurrence of the needle in `hay`.
    #[inline]
    pub(super) fn find(&self, hay: &[u8]) -> Option<usize> {
        if let Some(set) = &self.single {
            return set.find(hay, 0);
        }
        match &self.probe {
            None => Some(0),
            Some(probe) => find_with(hay, &self.needle, self.ci, probe),
        }
    }
}

/// Case-sensitive substring search.
pub fn find_literal_cs(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    find_with(hay, needle, false, &Probe::new(needle, false))
}

/// `true` when `hay` starts with `needle` (`needle` lowercased when `ci`).
#[inline]
pub(super) fn starts_with(hay: &[u8], needle: &[u8], ci: bool) -> bool {
    hay.len() >= needle.len() && verify(&hay[..needle.len()], needle, ci)
}

#[inline(always)]
fn verify(window: &[u8], needle: &[u8], ci: bool) -> bool {
    if ci { window.eq_ignore_ascii_case(needle) } else { window == needle }
}

#[inline]
fn find_with(hay: &[u8], needle: &[u8], ci: bool, probe: &Probe) -> Option<usize> {
    if hay.len() < needle.len() {
        return None;
    }
    let limit = hay.len() - needle.len() + 1;
    #[cfg(target_arch = "aarch64")]
    // SAFETY: NEON is mandatory on aarch64.
    let start = unsafe { scan_neon(hay, needle, ci, probe, limit) };
    #[cfg(target_arch = "x86_64")]
    // SAFETY: SSE2 is mandatory on x86_64.
    let start = unsafe { scan_sse2(hay, needle, ci, probe, limit) };
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    let start: Result<usize, usize> = Ok(0);
    match start {
        Err(found) => Some(found),
        Ok(from) => (from..limit).find(|&pos| probe.hits(hay, pos) && verify(&hay[pos..pos + needle.len()], needle, ci)),
    }
}

/// Vector part of the scan: `Err(match)` or `Ok(first position left for the scalar tail)`.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn scan_neon(hay: &[u8], needle: &[u8], ci: bool, p: &Probe, limit: usize) -> Result<usize, usize> {
    use std::arch::aarch64::*;
    let ptr = hay.as_ptr();
    let (v1, k1, v2, k2) = (vdupq_n_u8(p.b1), vdupq_n_u8(p.m1), vdupq_n_u8(p.b2), vdupq_n_u8(p.m2));
    // SAFETY (all loads below): a position `i` with `i + 16 <= limit =
    // hay.len() - needle.len() + 1` loads 16 bytes at `i + offset`, with both
    // offsets `< needle.len()`, so every load stays inside `hay`.
    let candidates = |i: usize| unsafe {
        let a = vceqq_u8(vorrq_u8(vld1q_u8(ptr.add(i + p.i1)), k1), v1);
        let b = vceqq_u8(vorrq_u8(vld1q_u8(ptr.add(i + p.i2)), k2), v2);
        vandq_u8(a, b)
    };
    let check = |eq: uint8x16_t, i: usize| -> Option<usize> {
        // Narrow each 0x00/0xFF lane to a nibble of a 64-bit mask.
        let mut mask = vget_lane_u64(vreinterpret_u64_u8(vshrn_n_u16(vreinterpretq_u16_u8(eq), 4)), 0);
        while mask != 0 {
            let lane = (mask.trailing_zeros() / 4) as usize;
            if verify(&hay[i + lane..i + lane + needle.len()], needle, ci) {
                return Some(i + lane);
            }
            mask &= !(0xF << (lane * 4));
        }
        None
    };
    let mut i = 0;
    while i + 32 <= limit {
        let (lo, hi) = (candidates(i), candidates(i + 16));
        if vmaxvq_u8(vorrq_u8(lo, hi)) != 0
            && let Some(found) = check(lo, i).or_else(|| check(hi, i + 16))
        {
            return Err(found);
        }
        i += 32;
    }
    if i + 16 <= limit {
        if let Some(found) = check(candidates(i), i) {
            return Err(found);
        }
        i += 16;
    }
    Ok(i)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn scan_sse2(hay: &[u8], needle: &[u8], ci: bool, p: &Probe, limit: usize) -> Result<usize, usize> {
    use std::arch::x86_64::*;
    let ptr = hay.as_ptr();
    let splat = |b: u8| _mm_set1_epi8(b as i8);
    let (v1, k1, v2, k2) = (splat(p.b1), splat(p.m1), splat(p.b2), splat(p.m2));
    let mut i = 0;
    while i + 16 <= limit {
        // SAFETY: see `scan_neon`; the same bounds hold.
        let mut mask = unsafe {
            let a = _mm_cmpeq_epi8(_mm_or_si128(_mm_loadu_si128(ptr.add(i + p.i1) as *const __m128i), k1), v1);
            let b = _mm_cmpeq_epi8(_mm_or_si128(_mm_loadu_si128(ptr.add(i + p.i2) as *const __m128i), k2), v2);
            _mm_movemask_epi8(_mm_and_si128(a, b)) as u32
        };
        while mask != 0 {
            let pos = i + mask.trailing_zeros() as usize;
            if verify(&hay[pos..pos + needle.len()], needle, ci) {
                return Err(pos);
            }
            mask &= mask - 1;
        }
        i += 16;
    }
    Ok(i)
}

/// Bytes that may start a match; up to four distinct bytes scan with SIMD.
#[derive(Clone, Debug)]
pub(super) struct ByteSet {
    bits: [u64; 4],
    /// The members, padded by repeating the first, when there are at most four.
    small: Option<[u8; 4]>,
}

impl ByteSet {
    pub(super) fn new(members: [bool; 256]) -> Self {
        let mut bits = [0u64; 4];
        let list: Vec<u8> = (0..=255u8).filter(|&b| members[b as usize]).collect();
        list.iter().for_each(|&b| bits[(b >> 6) as usize] |= 1 << (b & 63));
        let small = (1..=4).contains(&list.len()).then(|| std::array::from_fn(|i| *list.get(i).unwrap_or(&list[0])));
        ByteSet { bits, small }
    }

    pub(super) fn len(&self) -> usize {
        self.bits.iter().map(|w| w.count_ones() as usize).sum()
    }

    #[inline(always)]
    pub(super) fn contains(&self, b: u8) -> bool {
        (self.bits[(b >> 6) as usize] >> (b & 63)) & 1 != 0
    }

    /// First index `>= from` holding a member.
    #[inline]
    pub(super) fn find(&self, hay: &[u8], from: usize) -> Option<usize> {
        let tail = hay.get(from..)?;
        let mut start = 0;
        #[cfg(target_arch = "aarch64")]
        if let Some(small) = &self.small {
            // SAFETY: NEON is mandatory on aarch64.
            match unsafe { scan_set_neon(tail, small) } {
                Err(found) => return Some(from + found),
                Ok(rest) => start = rest,
            }
        }
        #[cfg(target_arch = "x86_64")]
        if let Some(small) = &self.small {
            // SAFETY: SSE2 is mandatory on x86_64.
            match unsafe { scan_set_sse2(tail, small) } {
                Err(found) => return Some(from + found),
                Ok(rest) => start = rest,
            }
        }
        tail[start..].iter().position(|&b| self.contains(b)).map(|i| from + start + i)
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn scan_set_neon(hay: &[u8], set: &[u8; 4]) -> Result<usize, usize> {
    use std::arch::aarch64::*;
    let ptr = hay.as_ptr();
    let v = set.map(|b| vdupq_n_u8(b));
    // A one-byte set (padded with copies) needs a single compare per chunk.
    let one = set.iter().all(|&b| b == set[0]);
    // SAFETY: callers pass `i` with `i + 16 <= hay.len()`.
    let hits = |i: usize| unsafe {
        let chunk = vld1q_u8(ptr.add(i));
        if one {
            vceqq_u8(chunk, v[0])
        } else {
            vorrq_u8(vorrq_u8(vceqq_u8(chunk, v[0]), vceqq_u8(chunk, v[1])), vorrq_u8(vceqq_u8(chunk, v[2]), vceqq_u8(chunk, v[3])))
        }
    };
    let first = |eq: uint8x16_t| {
        let mask = vget_lane_u64(vreinterpret_u64_u8(vshrn_n_u16(vreinterpretq_u16_u8(eq), 4)), 0);
        (mask != 0).then(|| (mask.trailing_zeros() / 4) as usize)
    };
    let mut i = 0;
    while i + 64 <= hay.len() {
        let eq = [hits(i), hits(i + 16), hits(i + 32), hits(i + 48)];
        if vmaxvq_u8(vorrq_u8(vorrq_u8(eq[0], eq[1]), vorrq_u8(eq[2], eq[3]))) != 0 {
            for (k, chunk) in eq.into_iter().enumerate() {
                if let Some(lane) = first(chunk) {
                    return Err(i + 16 * k + lane);
                }
            }
        }
        i += 64;
    }
    while i + 16 <= hay.len() {
        if let Some(lane) = first(hits(i)) {
            return Err(i + lane);
        }
        i += 16;
    }
    Ok(i)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn scan_set_sse2(hay: &[u8], set: &[u8; 4]) -> Result<usize, usize> {
    use std::arch::x86_64::*;
    let ptr = hay.as_ptr();
    let v = set.map(|b| _mm_set1_epi8(b as i8));
    let mut i = 0;
    while i + 16 <= hay.len() {
        // SAFETY: `i + 16 <= hay.len()`.
        let mask = unsafe {
            let chunk = _mm_loadu_si128(ptr.add(i) as *const __m128i);
            let eq = _mm_or_si128(
                _mm_or_si128(_mm_cmpeq_epi8(chunk, v[0]), _mm_cmpeq_epi8(chunk, v[1])),
                _mm_or_si128(_mm_cmpeq_epi8(chunk, v[2]), _mm_cmpeq_epi8(chunk, v[3])),
            );
            _mm_movemask_epi8(eq) as u32
        };
        if mask != 0 {
            return Err(i + mask.trailing_zeros() as usize);
        }
        i += 16;
    }
    Ok(i)
}
