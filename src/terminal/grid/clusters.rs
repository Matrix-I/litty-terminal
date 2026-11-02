//! Grapheme clusters: multi-codepoint graphemes are interned once and cells
//! store `CLUSTER_FLAG | id`. Unreferenced IDs are collected when the table
//! fills or after a history-sized batch of evictions, remapping live cells.

use super::*;

/// Multi-codepoint graphemes cells refer to (see `CLUSTER_FLAG`), interned,
/// plus the scratch buffers the collector reuses so it never allocates per row.
#[derive(Default)]
pub(super) struct ClusterTable {
    /// Interned cluster text, indexed by `CLUSTER_ID`.
    pub(super) texts: Vec<Box<str>>,
    pub(super) ids: std::collections::HashMap<Box<str>, u32>,
    /// Reused buffer for building a cluster.
    pub(super) buf: String,
    /// Collect at most once per history-sized batch of cluster-bearing evictions.
    pub(super) evictions: usize,
    /// Full-table misses to skip before another demand collection. Charging
    /// for the scanned cells also bounds retries when GC frees only one ID.
    pub(super) retry_misses: usize,
    pub(super) marks: Vec<bool>,
    pub(super) remap: Vec<u32>,
}

impl Grid {
    /// The grapheme cluster text of a cell's `codepoint`, if it holds one.
    pub fn cluster(&self, cp: u32) -> Option<&str> {
        if cp & (CLUSTER_FLAG | PH_FLAG) != CLUSTER_FLAG {
            return None;
        }
        self.clusters.texts.get((cp & CLUSTER_ID) as usize).map(|s| &**s)
    }

    /// A cell's character for searching and link detection: a cluster's first
    /// one, blank for empty cells.
    pub fn cell_char(&self, cell: &Cell) -> char {
        match self.cluster(cell.codepoint) {
            Some(text) => text.chars().next().unwrap_or(' '),
            None => char::from_u32(cell.codepoint).filter(|&c| c != '\0').unwrap_or(' '),
        }
    }

    /// Append a cell's text (the whole cluster) to `out`.
    pub fn push_cell_text(&self, cell: &Cell, out: &mut String) {
        match self.cluster(cell.codepoint) {
            Some(text) => out.push_str(text),
            None => out.push(self.cell_char(cell)),
        }
    }

    /// Intern a grapheme cluster string and return its ID (`0..MAX_CLUSTERS`),
    /// garbage-collecting unreferenced clusters if `MAX_CLUSTERS` is reached.
    pub(super) fn intern_cluster(&mut self, text: &str) -> Option<u32> {
        if text.len() > MAX_CLUSTER_BYTES {
            return None;
        }
        if let Some(&id) = self.clusters.ids.get(text) {
            return Some(id);
        }
        if self.clusters.texts.len() >= MAX_CLUSTERS {
            if self.clusters.retry_misses > 0 {
                self.clusters.retry_misses -= 1;
                return None;
            }
            self.gc_clusters();
            if let Some(&id) = self.clusters.ids.get(text) {
                return Some(id);
            }
            if self.clusters.texts.len() >= MAX_CLUSTERS {
                return None;
            }
        }
        let id = self.clusters.texts.len() as u32;
        if id == 0 {
            self.clusters.retry_misses = 0;
        }
        let boxed: Box<str> = text.into();
        self.clusters.texts.push(boxed.clone());
        self.clusters.ids.insert(boxed, id);
        Some(id)
    }

    #[cfg(test)]
    pub(crate) fn cluster_count(&self) -> usize {
        self.clusters.texts.len()
    }

    #[cfg(test)]
    pub(crate) fn seed_dead_clusters_for_bench(&mut self, count: usize) {
        self.clusters.texts.clear();
        self.clusters.ids.clear();
        self.clusters.texts.reserve(count);
        self.clusters.ids.reserve(count);
        for i in 0..count {
            let s: Box<str> = format!("dead_cluster_{i}").into_boxed_str();
            self.clusters.texts.push(s.clone());
            self.clusters.ids.insert(s, i as u32);
        }
    }

    /// Garbage-collect unreferenced grapheme clusters in `self.clusters.texts` and
    /// `self.clusters.ids`, compacting the table and remapping `CLUSTER_ID` on all
    /// active cells (`0..history + rows` and `alt`).
    pub(crate) fn gc_clusters(&mut self) {
        let total = self.clusters.texts.len();
        if total == 0 {
            return;
        }
        self.clusters.evictions = 0;
        // Retry eventually even after overwrites, erases, resize or leaving
        // the alternate screen, without invalidating on each cell mutation.
        // Existing history/eviction collections can recover capacity sooner.
        self.clusters.retry_misses = ((self.history + self.rows).min(self.cap)
            * self.cols + self.alt.len()).max(total);
        let mut used = std::mem::take(&mut self.clusters.marks);
        used.clear();
        used.resize(total, false);
        let mut used_count = 0usize;

        let mut mark_slice = |slice: &[Cell]| {
            for cell in slice {
                let cp = cell.codepoint;
                if cp & (CLUSTER_FLAG | PH_FLAG) == CLUSTER_FLAG {
                    let id = (cp & CLUSTER_ID) as usize;
                    if let Some(u) = used.get_mut(id)
                        && !*u {
                            *u = true;
                            used_count += 1;
                        }
                }
            }
        };

        let c = self.cols;
        if c > 0 && self.cap > 0 {
            let active_rows = (self.history + self.rows).min(self.cap);
            let start_row = (self.top + self.cap - self.history) % self.cap;
            if active_rows == self.cap {
                mark_slice(&self.cells);
            } else if start_row + active_rows <= self.cap {
                let s = start_row * c;
                let e = (start_row + active_rows) * c;
                mark_slice(&self.cells[s..e]);
            } else {
                let s1 = start_row * c;
                let s2 = (start_row + active_rows - self.cap) * c;
                mark_slice(&self.cells[s1..]);
                mark_slice(&self.cells[..s2]);
            }
        }
        if !self.alt.is_empty() {
            mark_slice(&self.alt);
        }

        if used_count == 0 {
            self.clusters.texts.clear();
            self.clusters.ids.clear();
            self.clusters.marks = used;
            return;
        }
        if used_count == total {
            self.clusters.marks = used;
            return;
        }

        let mut remap = std::mem::take(&mut self.clusters.remap);
        remap.clear();
        remap.resize(total, 0);
        self.clusters.ids.clear();
        let mut old_id = 0;
        let mut new_id = 0;
        self.clusters.texts.retain(|text| {
            let live = used[old_id];
            if live {
                remap[old_id] = new_id;
                self.clusters.ids.insert(text.clone(), new_id);
                new_id += 1;
            }
            old_id += 1;
            live
        });

        let remap_slice = |slice: &mut [Cell]| {
            for cell in slice {
                let cp = cell.codepoint;
                if cp & (CLUSTER_FLAG | PH_FLAG) == CLUSTER_FLAG {
                    let old_id = (cp & CLUSTER_ID) as usize;
                    if let Some(&new_id) = remap.get(old_id) {
                        cell.codepoint = (cp & !CLUSTER_ID) | new_id;
                    }
                }
            }
        };

        if c > 0 && self.cap > 0 {
            let active_rows = (self.history + self.rows).min(self.cap);
            let start_row = (self.top + self.cap - self.history) % self.cap;
            if active_rows == self.cap {
                remap_slice(&mut self.cells);
            } else if start_row + active_rows <= self.cap {
                let s = start_row * c;
                let e = (start_row + active_rows) * c;
                remap_slice(&mut self.cells[s..e]);
            } else {
                let s1 = start_row * c;
                let s2 = (start_row + active_rows - self.cap) * c;
                remap_slice(&mut self.cells[s1..]);
                remap_slice(&mut self.cells[..s2]);
            }
        }
        if !self.alt.is_empty() {
            remap_slice(&mut self.alt);
        }
        self.clusters.marks = used;
        self.clusters.remap = remap;
    }
}
