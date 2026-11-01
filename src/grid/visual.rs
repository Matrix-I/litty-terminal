//! Bidirectional text: visual order of a display row and logical <-> visual
//! column mapping for the renderer, mouse and cursor (scratch buffers are
//! thread-local, so nothing allocates per frame).

use super::*;

thread_local! {
    static BIDI_SCRATCH: std::cell::RefCell<(Vec<char>, Vec<u8>, Vec<u16>)> =
        const { std::cell::RefCell::new((Vec::new(), Vec::new(), Vec::new())) };
}

impl Grid {
    /// Visual order of display row `r` (see [`crate::bidi::visual_order`]),
    /// with `chars` and `levels` as scratch space. False when the row has no
    /// right-to-left text, or on the alternate screen: full-screen
    /// applications lay out their own bidirectional text.
    pub fn bidi_order(&self, r: usize, chars: &mut Vec<char>, levels: &mut Vec<u8>, order: &mut Vec<u16>) -> bool {
        order.clear();
        let row = self.display_row(r);
        // Nothing below the Hebrew block is right-to-left.
        if self.alt_active
            || !row.iter().any(|c| {
                let cp = c.codepoint;
                if cp & (CLUSTER_FLAG | PH_FLAG) == 0 {
                    cp >= 0x0590
                } else if cp & (CLUSTER_FLAG | PH_FLAG) == CLUSTER_FLAG {
                    self.cluster(cp).is_some_and(|text| text.chars().any(|ch| ch >= '\u{0590}'))
                } else {
                    false
                }
            })
        {
            return false;
        }
        chars.clear();
        chars.extend(row.iter().map(|c| if c.codepoint == 0 { '\0' } else { self.cell_char(c) }));
        crate::bidi::visual_order(chars, levels, order)
    }

    /// The logical column drawn at visual column `col` of display row `r`.
    pub fn logical_col(&self, r: usize, col: usize) -> usize {
        if r >= self.rows + usize::from(self.scroll_fraction > 0.0) {
            return col;
        }
        BIDI_SCRATCH.with(|scratch| {
            let (chars, levels, order) = &mut *scratch.borrow_mut();
            if self.bidi_order(r, chars, levels, order) {
                order.get(col).map_or(col, |&l| l as usize)
            } else {
                col
            }
        })
    }

    /// The visual column the cursor at logical column `col` of display row
    /// `r` is drawn at. After left-to-right text the cursor stays right after
    /// that text, even when the cell under it starts a right-to-left run
    /// (drawn at the run's right end): typing `echo '` before a right-to-left
    /// autosuggestion keeps the cursor next to the quote.
    pub fn cursor_visual_col(&self, r: usize, col: usize) -> usize {
        if r >= self.rows + usize::from(self.scroll_fraction > 0.0) {
            return col;
        }
        BIDI_SCRATCH.with(|scratch| {
            let (chars, levels, order) = &mut *scratch.borrow_mut();
            if self.bidi_order(r, chars, levels, order) {
                let visual = |l: usize| order.iter().position(|&o| o as usize == l);
                if let Some(prev) = col.checked_sub(1).filter(|&p| levels.get(p) == Some(&0))
                    && let Some(v) = visual(prev).map(|v| v + 1).filter(|&v| v < order.len()) {
                        return v;
                    }
                visual(col).unwrap_or(col)
            } else {
                col
            }
        })
    }
}
