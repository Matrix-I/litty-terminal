//! Shaped output of runs seen before, keyed by their style and text. Platform
//! shaping is the bulk of a frame's work for code-heavy output, and the same
//! runs (`->`, `!=`, `();`) repeat on most rows.
//!
//! Two generations bound the memory: inserts go to the current one, and when
//! it fills it becomes the previous one and a new current one starts. A hit in
//! the previous generation is copied forward, so runs in use survive a flood
//! of one-off runs (timestamps, hex ids) while unused ones age out. Arenas keep
//! their capacity across resets, so a warm cache never allocates.

use std::collections::HashMap;
use std::hash::{BuildHasher, BuildHasherDefault, Hasher, RandomState};

/// Entries per generation.
pub(super) const RUN_CACHE_ENTRIES: usize = 4096;
/// Key bytes (style byte + run text) per generation.
const GENERATION_TEXT: usize = 64 * 1024;
/// `Entry::out_at` for a run whose output is all zeros (nothing substituted),
/// which is every run for a font without ligatures: no words are stored.
const NO_OUT: u32 = u32::MAX;

/// The index is keyed by a hash computed once over the whole key; hashing that
/// `u64` again would double the work per lookup.
#[derive(Default)]
struct PreHashed(u64);

impl Hasher for PreHashed {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(b);
        }
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}

#[derive(Copy, Clone)]
struct Entry {
    key_at: u32,
    key_len: u32,
    out_at: u32,
}

#[derive(Default)]
struct Generation {
    index: HashMap<u64, Entry, BuildHasherDefault<PreHashed>>,
    keys: Vec<u8>,
    out: Vec<u32>,
}

impl Generation {
    fn clear(&mut self) {
        self.index.clear();
        self.keys.clear();
        self.out.clear();
    }

    fn is_full_for(&self, key_len: usize) -> bool {
        self.index.len() >= RUN_CACHE_ENTRIES || self.keys.len() + key_len > GENERATION_TEXT
    }

    /// `None` on a miss; `Some(None)` for a cached all-zero output.
    fn find(&self, hash: u64, key: &[u8]) -> Option<Option<&[u32]>> {
        let e = self.index.get(&hash)?;
        let (at, len) = (e.key_at as usize, e.key_len as usize);
        // Different keys can share a hash; the stored key decides.
        if self.keys[at..at + len] != *key {
            return None;
        }
        Some((e.out_at != NO_OUT).then(|| &self.out[e.out_at as usize..e.out_at as usize + len - 1]))
    }

    fn push(&mut self, hash: u64, key: &[u8], out: &[u32]) {
        let key_at = self.keys.len() as u32;
        self.keys.extend_from_slice(key);
        let out_at = if out.iter().all(|&w| w == 0) {
            NO_OUT
        } else {
            let at = self.out.len() as u32;
            self.out.extend_from_slice(out);
            at
        };
        // A colliding hash replaces the older entry; its arena bytes stay until the reset.
        self.index.insert(hash, Entry { key_at, key_len: key.len() as u32, out_at });
    }
}

pub(super) struct RunCache {
    current: Generation,
    previous: Generation,
    /// Randomly keyed, so terminal output cannot be crafted to collide.
    seed: RandomState,
    /// The key of the run being looked up: a style byte, then its text.
    key: Vec<u8>,
    /// The (size bits, cell width) the entries were shaped at.
    shaped_at: (u32, u32),
}

impl RunCache {
    pub(super) fn new() -> Self {
        RunCache {
            current: Generation::default(),
            previous: Generation::default(),
            seed: RandomState::new(),
            key: Vec::new(),
            shaped_at: (0, 0),
        }
    }

    pub(super) fn clear(&mut self) {
        self.current.clear();
        self.previous.clear();
    }

    /// Drop every entry if shaping now happens at another size or cell width.
    pub(super) fn set_shaped_at(&mut self, px: f32, cell_w: u32) {
        if self.shaped_at != (px.to_bits(), cell_w) {
            self.clear();
            self.shaped_at = (px.to_bits(), cell_w);
        }
    }

    /// Make (`style`, `text`) the key for the next `get_into` / `insert`; returns its hash.
    pub(super) fn load(&mut self, style: u8, text: impl IntoIterator<Item = u8>) -> u64 {
        self.key.clear();
        self.key.push(style);
        self.key.extend(text);
        let mut h = self.seed.build_hasher();
        h.write(&self.key);
        h.finish()
    }

    /// Copy the cached output for the loaded key into `out` (one word per text
    /// byte); `false` on a miss, leaving `out` untouched.
    pub(super) fn get_into(&mut self, hash: u64, out: &mut [u32]) -> bool {
        let fill = |out: &mut [u32], words: Option<&[u32]>| match words {
            Some(words) => out.copy_from_slice(words),
            None => out.fill(0),
        };
        if let Some(words) = self.current.find(hash, &self.key) {
            fill(out, words);
            return true;
        }
        let Some(words) = self.previous.find(hash, &self.key) else { return false };
        fill(out, words);
        self.insert(hash, out);
        true
    }

    /// Cache `out` for the loaded key.
    pub(super) fn insert(&mut self, hash: u64, out: &[u32]) {
        debug_assert_eq!(out.len() + 1, self.key.len());
        if self.current.is_full_for(self.key.len()) {
            std::mem::swap(&mut self.current, &mut self.previous);
            self.current.clear();
        }
        self.current.push(hash, &self.key, out);
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.current.index.len() + self.previous.index.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(cache: &mut RunCache, style: u8, text: &str, out: &[u32]) {
        let h = cache.load(style, text.bytes());
        cache.insert(h, out);
    }

    fn get(cache: &mut RunCache, style: u8, text: &str) -> Option<Vec<u32>> {
        let h = cache.load(style, text.bytes());
        let mut out = vec![7; text.len()];
        cache.get_into(h, &mut out).then_some(out)
    }

    #[test]
    fn returns_stored_words_per_style_and_text() {
        let mut c = RunCache::new();
        insert(&mut c, 0, "->", &[0x8000_0020, 0x8000_0021]);
        insert(&mut c, 1, "->", &[0x8000_0040, 0x8000_0041]);
        insert(&mut c, 0, "!=", &[0, 0]);
        assert_eq!(get(&mut c, 0, "->"), Some(vec![0x8000_0020, 0x8000_0021]));
        assert_eq!(get(&mut c, 1, "->"), Some(vec![0x8000_0040, 0x8000_0041]));
        assert_eq!(get(&mut c, 0, "!="), Some(vec![0, 0]), "all-zero output is a hit that writes zeros");
        assert_eq!(get(&mut c, 0, "=>"), None);
        assert_eq!(get(&mut c, 0, "->x"), None);
    }

    #[test]
    fn a_hash_collision_is_a_miss() {
        let mut c = RunCache::new();
        let h = c.load(0, *b"->");
        c.insert(h, &[1, 2]);
        c.load(0, *b"<=");
        let mut out = [0; 2];
        assert!(!c.get_into(h, &mut out), "same hash, other text");
        assert_eq!(out, [0, 0]);
    }

    #[test]
    fn runs_in_use_survive_a_flood_of_new_ones() {
        let mut c = RunCache::new();
        insert(&mut c, 0, "->", &[1, 2]);
        for i in 0..RUN_CACHE_ENTRIES * 3 {
            let run = format!("{i}:");
            insert(&mut c, 0, &run, &vec![0; run.len()]);
            if i % 1000 == 0 {
                assert_eq!(get(&mut c, 0, "->"), Some(vec![1, 2]), "after {i} inserts");
            }
            assert!(c.len() <= 2 * RUN_CACHE_ENTRIES);
        }
        assert_eq!(get(&mut c, 0, "->"), Some(vec![1, 2]));
        assert_eq!(get(&mut c, 0, "5:"), None, "unused runs age out");
    }

    #[test]
    fn key_bytes_are_bounded_per_generation() {
        let mut c = RunCache::new();
        let long = "-".repeat(1000);
        for _ in 0..200 {
            insert(&mut c, 0, &long, &vec![0; 1000]);
            assert!(c.current.keys.len() <= GENERATION_TEXT && c.previous.keys.len() <= GENERATION_TEXT);
        }
    }

    #[test]
    fn a_new_size_drops_entries() {
        let mut c = RunCache::new();
        c.set_shaped_at(28.0, 17);
        insert(&mut c, 0, "->", &[1, 2]);
        c.set_shaped_at(28.0, 17);
        assert!(get(&mut c, 0, "->").is_some());
        c.set_shaped_at(30.0, 17);
        assert_eq!(get(&mut c, 0, "->"), None);
    }

    #[test]
    fn warm_lookups_do_not_allocate() {
        let mut c = RunCache::new();
        insert(&mut c, 0, "->", &[1, 2]);
        let mut out = [0; 2];
        let n = crate::test_alloc::allocations(|| {
            for _ in 0..100 {
                let h = c.load(0, *b"->");
                assert!(c.get_into(h, &mut out));
            }
        });
        assert_eq!(n, 0);
    }
}
