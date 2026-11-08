//! Native / minimal RFC 1950 (zlib) + RFC 1951 (DEFLATE) decompression and
//! a pure-Rust PNG reader used on every platform.

use super::{MAX_SIDE, STORE_LIMIT};

/// Decompress an RFC 1950 zlib stream into at most `limit` bytes.
pub(crate) fn zlib_decompress_with_limit(data: &[u8], limit: usize) -> Result<Vec<u8>, &'static str> {
    if data.len() < 6 {
        return Err("EINVAL:bad zlib data");
    }
    let cmf = data[0];
    let flg = data[1];
    if (cmf & 0x0f) != 8 || (cmf >> 4) > 7 || ((u16::from(cmf) << 8 | u16::from(flg)) % 31) != 0 {
        return Err("EINVAL:bad zlib data");
    }
    if (flg & 0x20) != 0 {
        return Err("EINVAL:bad zlib data");
    }
    let mut bits = BitReader::new(&data[2..]);
    let mut out = Vec::new();
    loop {
        let bfinal = bits.read_bits(1)?;
        let btype = bits.read_bits(2)?;
        match btype {
            0 => {
                bits.align_byte();
                let len = bits.read_u16()? as usize;
                let nlen = bits.read_u16()? as usize;
                if (len ^ 0xffff) != nlen {
                    return Err("EINVAL:bad zlib data");
                }
                if out.len().saturating_add(len) > limit {
                    return Err("EINVAL:bad zlib data");
                }
                let slice = bits.read_bytes(len)?;
                out.extend_from_slice(slice);
            }
            1 => {
                let (lit_tree, dist_tree) = fixed_trees();
                decode_huffman_block(&mut bits, &lit_tree, &dist_tree, &mut out, limit)?;
            }
            2 => {
                let (lit_tree, dist_tree) = read_dynamic_trees(&mut bits)?;
                decode_huffman_block(&mut bits, &lit_tree, &dist_tree, &mut out, limit)?;
            }
            _ => return Err("EINVAL:bad zlib data"),
        }
        if bfinal != 0 {
            break;
        }
    }
    bits.align_byte();
    let expected_adler = bits.read_u32_be()?;
    if adler32(&out) != expected_adler {
        return Err("EINVAL:bad zlib data");
    }
    Ok(out)
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u64,
    bits: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, buf: 0, bits: 0 }
    }

    #[inline]
    fn refill(&mut self) {
        while self.bits <= 56 && self.pos < self.data.len() {
            self.buf |= u64::from(self.data[self.pos]) << self.bits;
            self.pos += 1;
            self.bits += 8;
        }
    }

    #[inline]
    fn read_bits(&mut self, n: u32) -> Result<u32, &'static str> {
        if n == 0 {
            return Ok(0);
        }
        if self.bits < n {
            self.refill();
            if self.bits < n {
                return Err("EINVAL:bad zlib data");
            }
        }
        let val = (self.buf & ((1u64 << n) - 1)) as u32;
        self.buf >>= n;
        self.bits -= n;
        Ok(val)
    }

    fn align_byte(&mut self) {
        let drop = self.bits & 7;
        self.buf >>= drop;
        self.bits -= drop;
    }

    fn read_u16(&mut self) -> Result<u16, &'static str> {
        let lo = self.read_bits(8)? as u16;
        let hi = self.read_bits(8)? as u16;
        Ok(lo | (hi << 8))
    }

    fn read_u32_be(&mut self) -> Result<u32, &'static str> {
        let b0 = self.read_bits(8)?;
        let b1 = self.read_bits(8)?;
        let b2 = self.read_bits(8)?;
        let b3 = self.read_bits(8)?;
        Ok((b0 << 24) | (b1 << 16) | (b2 << 8) | b3)
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], &'static str> {
        debug_assert_eq!(self.bits & 7, 0);
        let rewind = (self.bits / 8) as usize;
        self.pos -= rewind;
        self.bits = 0;
        self.buf = 0;
        if self.pos.saturating_add(len) > self.data.len() {
            return Err("EINVAL:bad zlib data");
        }
        let slice = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Ok(slice)
    }
}

const MAX_BITS: usize = 15;

struct HuffTree {
    counts: [u16; MAX_BITS + 1],
    symbols: [u16; 288],
}

impl HuffTree {
    fn from_lengths(lengths: &[u8]) -> Result<Self, &'static str> {
        let mut counts = [0u16; MAX_BITS + 1];
        for &len in lengths {
            if len as usize > MAX_BITS {
                return Err("EINVAL:bad zlib data");
            }
            if len > 0 {
                counts[len as usize] += 1;
            }
        }
        let mut offsets = [0u16; MAX_BITS + 1];
        let mut total = 0u16;
        for bits in 1..=MAX_BITS {
            offsets[bits] = total;
            total += counts[bits];
        }
        let mut symbols = [0u16; 288];
        for (sym, &len) in lengths.iter().enumerate() {
            if len > 0 {
                let idx = offsets[len as usize] as usize;
                offsets[len as usize] += 1;
                symbols[idx] = sym as u16;
            }
        }
        Ok(Self { counts, symbols })
    }

    #[inline]
    fn decode(&self, bits: &mut BitReader<'_>) -> Result<u16, &'static str> {
        let mut code = 0i32;
        let mut first = 0i32;
        let mut index = 0i32;
        for len in 1..=MAX_BITS {
            code |= bits.read_bits(1)? as i32;
            let count = self.counts[len] as i32;
            if code - count < first {
                return Ok(self.symbols[(index + (code - first)) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("EINVAL:bad zlib data")
    }
}

fn fixed_trees() -> (HuffTree, HuffTree) {
    let mut lit = [0u8; 288];
    lit[..144].fill(8);
    lit[144..256].fill(9);
    lit[256..280].fill(7);
    lit[280..288].fill(8);
    let dist = [5u8; 32];
    (HuffTree::from_lengths(&lit).unwrap(), HuffTree::from_lengths(&dist).unwrap())
}

fn read_dynamic_trees(bits: &mut BitReader<'_>) -> Result<(HuffTree, HuffTree), &'static str> {
    let hlit = bits.read_bits(5)? as usize + 257;
    let hdist = bits.read_bits(5)? as usize + 1;
    let hclen = bits.read_bits(4)? as usize + 4;
    if hlit > 286 || hdist > 30 {
        return Err("EINVAL:bad zlib data");
    }
    const CL_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
    let mut cl_lengths = [0u8; 19];
    for i in 0..hclen {
        cl_lengths[CL_ORDER[i]] = bits.read_bits(3)? as u8;
    }
    let cl_tree = HuffTree::from_lengths(&cl_lengths)?;
    let total = hlit + hdist;
    let mut lengths = [0u8; 318];
    let mut i = 0;
    while i < total {
        let sym = cl_tree.decode(bits)?;
        match sym {
            0..=15 => {
                lengths[i] = sym as u8;
                i += 1;
            }
            16 => {
                if i == 0 {
                    return Err("EINVAL:bad zlib data");
                }
                let prev = lengths[i - 1];
                let rep = bits.read_bits(2)? as usize + 3;
                if i + rep > total {
                    return Err("EINVAL:bad zlib data");
                }
                lengths[i..i + rep].fill(prev);
                i += rep;
            }
            17 => {
                let rep = bits.read_bits(3)? as usize + 3;
                if i + rep > total {
                    return Err("EINVAL:bad zlib data");
                }
                i += rep;
            }
            18 => {
                let rep = bits.read_bits(7)? as usize + 11;
                if i + rep > total {
                    return Err("EINVAL:bad zlib data");
                }
                i += rep;
            }
            _ => return Err("EINVAL:bad zlib data"),
        }
    }
    let lit_tree = HuffTree::from_lengths(&lengths[..hlit])?;
    let dist_tree = HuffTree::from_lengths(&lengths[hlit..total])?;
    Ok((lit_tree, dist_tree))
}

fn decode_huffman_block(
    bits: &mut BitReader<'_>,
    lit_tree: &HuffTree,
    dist_tree: &HuffTree,
    out: &mut Vec<u8>,
    limit: usize,
) -> Result<(), &'static str> {
    const LEN_BASE: [u16; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
        131, 163, 195, 227, 258,
    ];
    const LEN_EXTRA: [u8; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
    ];
    const DIST_BASE: [u16; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
        2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
    ];
    const DIST_EXTRA: [u8; 30] = [
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
        13, 13,
    ];

    loop {
        let sym = lit_tree.decode(bits)?;
        if sym < 256 {
            if out.len() >= limit {
                return Err("EINVAL:bad zlib data");
            }
            out.push(sym as u8);
        } else if sym == 256 {
            break;
        } else {
            let len_idx = (sym - 257) as usize;
            if len_idx >= LEN_BASE.len() {
                return Err("EINVAL:bad zlib data");
            }
            let length = LEN_BASE[len_idx] as usize + bits.read_bits(LEN_EXTRA[len_idx] as u32)? as usize;
            let dist_sym = dist_tree.decode(bits)? as usize;
            if dist_sym >= DIST_BASE.len() {
                return Err("EINVAL:bad zlib data");
            }
            let distance = DIST_BASE[dist_sym] as usize + bits.read_bits(DIST_EXTRA[dist_sym] as u32)? as usize;
            if distance == 0 || distance > out.len() {
                return Err("EINVAL:bad zlib data");
            }
            if out.len().saturating_add(length) > limit {
                return Err("EINVAL:bad zlib data");
            }
            out.reserve(length);
            for _ in 0..length {
                let b = out[out.len() - distance];
                out.push(b);
            }
        }
    }
    Ok(())
}

pub(crate) fn adler32(data: &[u8]) -> u32 {
    let mut s1 = 1u32;
    let mut s2 = 0u32;
    for chunk in data.chunks(5552) {
        for &b in chunk {
            s1 += u32::from(b);
            s2 += s1;
        }
        s1 %= 65521;
        s2 %= 65521;
    }
    (s2 << 16) | s1
}

#[cfg(test)]
pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if (crc & 1) != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// Decode a PNG image into `(width, height, rgba)`.
///
/// Validates the PNG header, `IHDR` bounds, and APNG `fcTL` canvas invariants
/// first, then decodes every standard color type and bit depth (palette and
/// `tRNS` expansion, 16-bit samples stripped to their high byte, Adam7) with
/// the built-in RFC 2083 reader, so both platforms produce the same pixels.
pub fn decode_png(data: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if data.len() < 33 || &data[..8] != PNG_SIG || &data[12..16] != b"IHDR" {
        return Err("EINVAL:bad PNG".into());
    }
    let iihdr_len = u32::from_be_bytes([data[8], data[9], data[10], data[11]]) as usize;
    if iihdr_len != 13 {
        return Err("EINVAL:bad PNG".into());
    }
    let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    let bit_depth = data[24];
    let color_type = data[25];
    let compression = data[26];
    let filter = data[27];
    let interlace = data[28];

    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return Err("EINVAL:bad image size".into());
    }
    if (w as usize).saturating_mul(h as usize).saturating_mul(4) > STORE_LIMIT {
        return Err("EINVAL:image too large".into());
    }
    if compression != 0 || filter != 0 || interlace > 1 {
        return Err("EINVAL:bad PNG".into());
    }
    let format = Format::new(color_type, bit_depth).ok_or("EINVAL:bad PNG")?;

    // Walk chunks to validate APNG sub-frame bounds and collect IDAT stream.
    let mut pos = 8usize;
    let mut idat = Vec::new();
    let mut plte: &[u8] = &[];
    let mut trns: &[u8] = &[];
    let mut saw_iend = false;
    while pos + 12 <= data.len() {
        let len = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        let ctype = &data[pos + 4..pos + 8];
        let cdata_start = pos + 8;
        let Some(cdata_end) = cdata_start.checked_add(len) else {
            return Err("EINVAL:bad PNG".into());
        };
        if cdata_end + 4 > data.len() {
            return Err("EINVAL:bad PNG".into());
        }
        let cdata = &data[cdata_start..cdata_end];
        match ctype {
            b"fcTL" => {
                if cdata.len() < 26 {
                    return Err("EINVAL:bad PNG".into());
                }
                let fw = u32::from_be_bytes([cdata[4], cdata[5], cdata[6], cdata[7]]);
                let fh = u32::from_be_bytes([cdata[8], cdata[9], cdata[10], cdata[11]]);
                if (fw, fh) != (w, h) {
                    return Err("EINVAL:bad PNG".into());
                }
            }
            b"PLTE" => plte = cdata,
            b"tRNS" => trns = cdata,
            b"IDAT" => {
                if idat.len().saturating_add(cdata.len()) > STORE_LIMIT {
                    return Err("EINVAL:image too large".into());
                }
                idat.extend_from_slice(cdata);
            }
            b"IEND" => {
                saw_iend = true;
                break;
            }
            _ => {}
        }
        pos = cdata_end + 4;
    }
    if !saw_iend || idat.is_empty() {
        return Err("EINVAL:bad PNG".into());
    }
    let palette = Palette::new(&format, plte, trns).ok_or("EINVAL:bad PNG")?;
    decode_png_scanlines(w as usize, h as usize, &format, &palette, interlace == 1, &idat)
        .map(|rgba| (w, h, rgba))
}

/// Adam7 pass origins and steps: `(x0, y0, dx, dy)`.
const ADAM7: [(usize, usize, usize, usize); 7] =
    [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];
const NO_INTERLACE: [(usize, usize, usize, usize); 1] = [(0, 0, 1, 1)];

/// Sample layout of one `IHDR` color type / bit depth pair.
struct Format {
    color_type: u8,
    depth: u8,
    channels: usize,
}

impl Format {
    /// The combinations allowed by the PNG specification, table 11.1.
    fn new(color_type: u8, depth: u8) -> Option<Self> {
        let channels = match (color_type, depth) {
            (0, 1 | 2 | 4 | 8 | 16) | (3, 1 | 2 | 4 | 8) => 1,
            (2, 8 | 16) => 3,
            (4, 8 | 16) => 2,
            (6, 8 | 16) => 4,
            _ => return None,
        };
        Some(Self { color_type, depth, channels })
    }

    fn bits_per_pixel(&self) -> usize {
        self.channels * usize::from(self.depth)
    }

    /// Filter distance: bytes per complete pixel, rounded up to one.
    fn filter_bpp(&self) -> usize {
        self.bits_per_pixel().div_ceil(8)
    }

    fn row_bytes(&self, width: usize) -> Option<usize> {
        Some(width.checked_mul(self.bits_per_pixel())?.div_ceil(8))
    }

    /// Sample `i` (in channel order) of a reconstructed scanline, unscaled.
    #[inline]
    fn sample(&self, line: &[u8], i: usize) -> u16 {
        match self.depth {
            8 => u16::from(line[i]),
            16 => u16::from_be_bytes([line[2 * i], line[2 * i + 1]]),
            d => {
                let bit = i * usize::from(d);
                let shift = 8 - usize::from(d) - bit % 8;
                u16::from(line[bit / 8] >> shift) & ((1 << d) - 1)
            }
        }
    }

    /// Scale an unscaled sample to 8 bits (high byte for 16-bit samples).
    #[inline]
    fn to_u8(&self, v: u16) -> u8 {
        match self.depth {
            16 => (v >> 8) as u8,
            8 => v as u8,
            d => (u32::from(v) * 255 / ((1 << d) - 1)) as u8,
        }
    }
}

/// `PLTE` entries and the `tRNS` alpha table or color key for one image.
struct Palette {
    rgba: Vec<[u8; 4]>,
    /// Gray (`[g, -, -]`) or RGB transparent key in unscaled samples.
    key: Option<[u16; 3]>,
}

impl Palette {
    /// `None` when an indexed image has no valid `PLTE`. A malformed `tRNS`
    /// is ignored, as libpng does, instead of rejecting the whole image.
    fn new(format: &Format, plte: &[u8], trns: &[u8]) -> Option<Self> {
        let be = |i: usize| u16::from_be_bytes([trns[2 * i], trns[2 * i + 1]]);
        match format.color_type {
            3 => {
                let entries = plte.len() / 3;
                // Entries beyond `1 << depth` are unreachable, so a long
                // palette is harmless (libpng treats it as a benign error).
                if !plte.len().is_multiple_of(3) || entries == 0 {
                    return None;
                }
                let alpha = if trns.len() <= entries { trns } else { &[] };
                let rgba = plte
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .enumerate()
                    .map(|(i, &[r, g, b])| [r, g, b, alpha.get(i).copied().unwrap_or(255)])
                    .collect();
                Some(Self { rgba, key: None })
            }
            0 => Some(Self { rgba: Vec::new(), key: (trns.len() == 2).then(|| [be(0), 0, 0]) }),
            2 => Some(Self { rgba: Vec::new(), key: (trns.len() == 6).then(|| [be(0), be(1), be(2)]) }),
            _ => Some(Self { rgba: Vec::new(), key: None }),
        }
    }
}

fn decode_png_scanlines(
    w: usize,
    h: usize,
    format: &Format,
    palette: &Palette,
    interlaced: bool,
    idat: &[u8],
) -> Result<Vec<u8>, String> {
    let passes: &[(usize, usize, usize, usize)] = if interlaced { &ADAM7 } else { &NO_INTERLACE };
    let pass_size = |&(x0, y0, dx, dy): &(usize, usize, usize, usize)| {
        ((w.saturating_sub(x0)).div_ceil(dx), (h.saturating_sub(y0)).div_ceil(dy))
    };
    let mut expected_raw = 0usize;
    for pass in passes {
        let (pw, ph) = pass_size(pass);
        if pw > 0 && ph > 0 {
            let stride = format.row_bytes(pw).ok_or("EINVAL:bad PNG")? + 1;
            expected_raw = stride.checked_mul(ph).and_then(|n| n.checked_add(expected_raw)).ok_or("EINVAL:bad PNG")?;
        }
    }
    // Raw samples may be twice the RGBA8 output (16-bit), plus filter bytes.
    let raw_limit = STORE_LIMIT * if format.depth == 16 { 2 } else { 1 } + 7 * h;
    if expected_raw > raw_limit {
        return Err("EINVAL:image too large".into());
    }
    let raw = zlib_decompress_with_limit(idat, expected_raw).map_err(|_| "EINVAL:bad PNG")?;
    if raw.len() != expected_raw {
        return Err("EINVAL:bad PNG".into());
    }

    let bpp = format.filter_bpp();
    let mut rgba = vec![0u8; w * h * 4];
    let mut rows = raw.as_slice();
    let mut prev_row = Vec::new();
    let mut cur_row = Vec::new();
    for pass @ &(x0, y0, dx, dy) in passes {
        let (pw, ph) = pass_size(pass);
        if pw == 0 || ph == 0 {
            continue;
        }
        let row_bytes = format.row_bytes(pw).ok_or("EINVAL:bad PNG")?;
        prev_row.clear();
        prev_row.resize(row_bytes, 0);
        cur_row.resize(row_bytes, 0);
        for py in 0..ph {
            let (src, rest) = rows.split_at(row_bytes + 1);
            rows = rest;
            unfilter(src[0], &src[1..], &prev_row, &mut cur_row, bpp)?;
            let y = y0 + py * dy;
            if dx == 1 && format.color_type == 6 && format.depth == 8 {
                // Common case: the reconstructed row already is RGBA.
                rgba[y * w * 4..][..w * 4].copy_from_slice(&cur_row);
                core::mem::swap(&mut prev_row, &mut cur_row);
                continue;
            }
            for px in 0..pw {
                let at = (y * w + x0 + px * dx) * 4;
                rgba[at..at + 4].copy_from_slice(&pixel(format, palette, &cur_row, px)?);
            }
            core::mem::swap(&mut prev_row, &mut cur_row);
        }
    }
    Ok(rgba)
}

/// Reverse one scanline filter (RFC 2083 §6) into `out`.
fn unfilter(filter: u8, line: &[u8], prev: &[u8], out: &mut [u8], bpp: usize) -> Result<(), String> {
    for i in 0..line.len() {
        let a = if i >= bpp { out[i - bpp] } else { 0 };
        let b = prev[i];
        let c = if i >= bpp { prev[i - bpp] } else { 0 };
        out[i] = match filter {
            0 => line[i],
            1 => line[i].wrapping_add(a),
            2 => line[i].wrapping_add(b),
            3 => line[i].wrapping_add(((u16::from(a) + u16::from(b)) >> 1) as u8),
            4 => line[i].wrapping_add(paeth(a, b, c)),
            _ => return Err("EINVAL:bad PNG".into()),
        };
    }
    Ok(())
}

/// RGBA of pixel `x` in a reconstructed scanline.
#[inline]
fn pixel(format: &Format, palette: &Palette, line: &[u8], x: usize) -> Result<[u8; 4], String> {
    let c = format.channels;
    let s = |i: usize| format.sample(line, x * c + i);
    let px = match format.color_type {
        // An index past the palette is black, as in libpng.
        3 => palette.rgba.get(usize::from(s(0))).copied().unwrap_or([0, 0, 0, 255]),
        0 => {
            let g = s(0);
            let v = format.to_u8(g);
            [v, v, v, if palette.key == Some([g, 0, 0]) { 0 } else { 255 }]
        }
        2 => {
            let rgb = [s(0), s(1), s(2)];
            let [r, g, b] = rgb.map(|v| format.to_u8(v));
            [r, g, b, if palette.key == Some(rgb) { 0 } else { 255 }]
        }
        4 => {
            let v = format.to_u8(s(0));
            [v, v, v, format.to_u8(s(1))]
        }
        _ => [s(0), s(1), s(2), s(3)].map(|v| format.to_u8(v)),
    };
    Ok(px)
}

#[inline]
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i32::from(a) + i32::from(b) - i32::from(c);
    let pa = (p - i32::from(a)).abs();
    let pb = (p - i32::from(b)).abs();
    let pc = (p - i32::from(c)).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Encode an 8-bit RGB or RGBA buffer as a valid PNG (using stored DEFLATE blocks) for tests.
#[cfg(test)]
pub(crate) fn encode_test_png(w: u32, h: u32, color_type: u8, pixels: &[u8]) -> Vec<u8> {
    let bpp: usize = match color_type {
        2 => 3,
        6 => 4,
        _ => panic!("unsupported test color_type"),
    };
    assert_eq!(pixels.len(), (w as usize) * (h as usize) * bpp);
    let mut raw = Vec::with_capacity((w as usize * bpp + 1) * h as usize);
    for row in pixels.chunks((w as usize) * bpp) {
        raw.push(0); // Filter: None
        raw.extend_from_slice(row);
    }
    let z = zlib_store(&raw);
    let mut out = Vec::from(&b"\x89PNG\r\n\x1a\n"[..]);
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&w.to_be_bytes());
    ihdr[4..8].copy_from_slice(&h.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = color_type;
    write_png_chunk(&mut out, b"IHDR", &ihdr);
    write_png_chunk(&mut out, b"IDAT", &z);
    write_png_chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
pub(crate) fn write_png_chunk(out: &mut Vec<u8>, ctype: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(ctype);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

#[cfg(test)]
pub(crate) fn zlib_store(raw: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let chunks: Vec<&[u8]> = if raw.is_empty() { vec![&[]] } else { raw.chunks(65535).collect() };
    for (i, chunk) in chunks.iter().enumerate() {
        let last = u8::from(i + 1 == chunks.len());
        out.push(last);
        let len = chunk.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&adler32(raw).to_be_bytes());
    out
}

/// Compress `total_zeros` zero bytes into a minimal valid RFC 1950 zlib stream
/// using Fixed Huffman RLE (symbol 0 + length-258 distance-1 repeats).
#[cfg(test)]
pub(crate) fn zlib_compress_zeros(total_zeros: usize) -> Vec<u8> {
    struct BitWriter {
        out: Vec<u8>,
        buf: u32,
        bits: u32,
    }
    impl BitWriter {
        fn new(out: Vec<u8>) -> Self {
            Self { out, buf: 0, bits: 0 }
        }
        fn write_bits(&mut self, val: u32, n: u32) {
            self.buf |= val << self.bits;
            self.bits += n;
            while self.bits >= 8 {
                self.out.push(self.buf as u8);
                self.buf >>= 8;
                self.bits -= 8;
            }
        }
        fn write_huff_msb(&mut self, code: u32, n: u32) {
            for i in (0..n).rev() {
                self.write_bits((code >> i) & 1, 1);
            }
        }
        fn finish(mut self) -> Vec<u8> {
            if self.bits > 0 {
                self.out.push(self.buf as u8);
            }
            self.out
        }
    }

    let mut bw = BitWriter::new(vec![0x78, 0x01]);
    bw.write_bits(1, 1); // BFINAL = 1
    bw.write_bits(1, 2); // BTYPE = 01 (Fixed Huffman)
    if total_zeros > 0 {
        // Literal 0: Fixed Huffman code is 0x30 (8 bits)
        bw.write_huff_msb(0x30, 8);
        let mut rem = total_zeros - 1;
        while rem >= 258 {
            // Length 258 = symbol 285 -> Fixed Huffman code 0xc5 (8 bits)
            bw.write_huff_msb(0xc5, 8);
            // Distance 1 = symbol 0 -> Fixed Huffman code 0x00 (5 bits)
            bw.write_huff_msb(0x00, 5);
            rem -= 258;
        }
        for _ in 0..rem {
            bw.write_huff_msb(0x30, 8);
        }
    }
    // End of block: symbol 256 -> Fixed Huffman code 0x00 (7 bits)
    bw.write_huff_msb(0x00, 7);
    let mut out = bw.finish();
    // Adler32 of `total_zeros` zero bytes: s1 = 1, s2 = total_zeros % 65521
    let adler = (((total_zeros % 65521) as u32) << 16) | 1;
    out.extend_from_slice(&adler.to_be_bytes());
    out
}
