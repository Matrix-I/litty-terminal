//! Grapheme clusters: multi-codepoint graphemes are interned once and cells
//! store `CLUSTER_FLAG | id`. Unreferenced IDs are collected when the table
//! fills or after a history-sized batch of evictions, remapping live cells.

use super::*;

impl Grid {
    /// The grapheme cluster text of a cell's `codepoint`, if it holds one.
    pub fn cluster(&self, cp: u32) -> Option<&str> {
        if cp & (CLUSTER_FLAG | PH_FLAG) != CLUSTER_FLAG {
            return None;
        }
        self.clusters.get((cp & CLUSTER_ID) as usize).map(|s| &**s)
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
        if let Some(&id) = self.cluster_ids.get(text) {
            return Some(id);
        }
        if self.clusters.len() >= MAX_CLUSTERS {
            self.gc_clusters();
            if let Some(&id) = self.cluster_ids.get(text) {
                return Some(id);
            }
            if self.clusters.len() >= MAX_CLUSTERS {
                return None;
            }
        }
        let id = self.clusters.len() as u32;
        let boxed: Box<str> = text.into();
        self.clusters.push(boxed.clone());
        self.cluster_ids.insert(boxed, id);
        Some(id)
    }

    #[cfg(test)]
    pub(crate) fn cluster_count(&self) -> usize {
        self.clusters.len()
    }

    #[cfg(test)]
    pub(crate) fn seed_dead_clusters_for_bench(&mut self, count: usize) {
        self.clusters.clear();
        self.cluster_ids.clear();
        self.clusters.reserve(count);
        self.cluster_ids.reserve(count);
        for i in 0..count {
            let s: Box<str> = format!("dead_cluster_{i}").into_boxed_str();
            self.clusters.push(s.clone());
            self.cluster_ids.insert(s, i as u32);
        }
    }

    /// Garbage-collect unreferenced grapheme clusters in `self.clusters` and
    /// `self.cluster_ids`, compacting the table and remapping `CLUSTER_ID` on all
    /// active cells (`0..history + rows` and `alt`).
    pub(crate) fn gc_clusters(&mut self) {
        let total = self.clusters.len();
        if total == 0 {
            return;
        }
        self.cluster_evictions = 0;
        let mut used = std::mem::take(&mut self.cluster_marks);
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
            self.clusters.clear();
            self.cluster_ids.clear();
            self.cluster_marks = used;
            return;
        }
        if used_count == total {
            self.cluster_marks = used;
            return;
        }

        let mut remap = std::mem::take(&mut self.cluster_remap);
        remap.clear();
        remap.resize(total, 0);
        self.cluster_ids.clear();
        let mut old_id = 0;
        let mut new_id = 0;
        self.clusters.retain(|text| {
            let live = used[old_id];
            if live {
                remap[old_id] = new_id;
                self.cluster_ids.insert(text.clone(), new_id);
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
        self.cluster_marks = used;
        self.cluster_remap = remap;
    }
}
