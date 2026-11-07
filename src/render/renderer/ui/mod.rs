//! Overlay widgets drawn as cell instances on top of the panes. Each builder
//! borrows only the atlas, the queue and the instance slice (`Cells`), never
//! the whole `Renderer`.

use super::{Instance, Renderer};
use crate::render::atlas::Atlas;

pub(super) mod history_popup;
pub(super) mod inspector;
pub(super) mod link_preview;
pub(super) mod preedit;
pub(super) mod scrollbar;
pub(super) mod search_box;
pub(super) mod tab_bar;

/// The window's cell instances and the glyph atlas they index.
pub(in crate::render::renderer) struct Cells<'a> {
    pub(in crate::render::renderer) atlas: &'a mut Atlas,
    pub(in crate::render::renderer) queue: &'a wgpu::Queue,
    pub(in crate::render::renderer) instances: &'a mut [Instance],
    /// Window width in cells: the row stride of `instances`.
    pub(in crate::render::renderer) cols: usize,
}

/// Window rows whose instances changed this frame; `lo..=hi` is uploaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::render::renderer) struct DirtyRows {
    pub(in crate::render::renderer) lo: usize,
    pub(in crate::render::renderer) hi: usize,
}

impl DirtyRows {
    /// Nothing dirty (`lo > hi`).
    pub(in crate::render::renderer) const NONE: Self = Self { lo: usize::MAX, hi: 0 };

    /// Widen the range to include rows `first..=last`.
    #[inline]
    pub(in crate::render::renderer) fn mark(&mut self, first: usize, last: usize) {
        self.lo = self.lo.min(first);
        self.hi = self.hi.max(last);
    }
}

impl Renderer {
    /// Disjoint borrows of the fields every overlay writes through.
    pub(in crate::render::renderer) fn cells(&mut self, cols: usize) -> Cells<'_> {
        Cells { atlas: &mut self.atlas, queue: &self.queue, instances: &mut self.instances, cols }
    }
}

#[cfg(test)]
mod tests {
    use super::DirtyRows;

    #[test]
    fn dirty_rows_mark_widens_range() {
        let mut d = DirtyRows::NONE;
        assert!(d.lo > d.hi, "starts empty");
        d.mark(5, 7);
        d.mark(2, 3);
        assert_eq!(d, DirtyRows { lo: 2, hi: 7 });
    }
}

#[cfg(test)]

#[test]
#[ignore = "requires a GPU adapter; run explicitly on macOS and Linux"]
fn warm_overlay_builders_do_not_allocate() {
    use crate::render::atlas::{Atlas,load_font};
    use crate::terminal::{search::SearchBar,history::{SortMode,MatchResult}};
    let gpu=wgpu::Instance::default();
    let adapter=pollster::block_on(gpu.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("GPU adapter");
    let (device,queue)=pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(),None)).unwrap();
    let mut atlas=Atlas::new(&device,&queue,load_font(&[]),16.0);
    let mut instances=vec![Instance::default();160*40];
    let mut dirty=DirtyRows::NONE;
    let mut cells=Cells{atlas:&mut atlas,queue:&queue,instances:&mut instances,cols:160};
    let titles=vec!["shell".into(),"a very long title that must be truncated inside the allocated tab span ".repeat(3)];
    let shifts=[0,3];
    let view=tab_bar::TabBarView{shifts:&shifts,dragged:Some(1),..tab_bar::TabBarView::new(&titles,0)};
    for _ in 0..3 {tab_bar::build(&mut cells,view);}
    assert_eq!(crate::test_alloc::allocations(||tab_bar::build(&mut cells,view)), 0);
    let mut search=SearchBar::new();search.query="needle".into();search.total=123;search.current=Some(0);
    for _ in 0..3 {search_box::build(&mut cells,2,40,&search,&mut dirty);}
    assert_eq!(crate::test_alloc::allocations(||search_box::build(&mut cells,2,40,&search,&mut dirty)), 0);
    let items:std::collections::VecDeque<String>=(0..9).map(|_|format!("echo {}", "x".repeat(200))).collect();
    let filtered:Vec<MatchResult>=(0..9).map(|i|MatchResult{item_index:i,score:0,match_indices:vec![0,1,2,3]}).collect();
    let mut history=crate::render::HistoryView{is_open:true,query:"echo",selected:0,scroll_offset:0,sort_mode:SortMode::Relevance,items:&items,filtered:&filtered};
    let geometry=history_popup::geometry(160,40).unwrap();
    for _ in 0..3 {history_popup::build(&mut cells,geometry,40,&history,&mut dirty);}
    assert_eq!(crate::test_alloc::allocations(||history_popup::build(&mut cells,geometry,40,&history,&mut dirty)), 0);
    for query in ["", "é日本😀".repeat(40).as_str(), "("] {
        search.query = query.into();
        search.invalid = query == "(";
        search.total = crate::terminal::search::MAX_MATCHES;
        search.current = Some(search.total - 1);
        history.query = query;
        for _ in 0..3 {
            search_box::build(&mut cells,2,40,&search,&mut dirty);
            history_popup::build(&mut cells,geometry,40,&history,&mut dirty);
        }
        assert_eq!(crate::test_alloc::allocations(||search_box::build(&mut cells,2,40,&search,&mut dirty)), 0);
        assert_eq!(crate::test_alloc::allocations(||history_popup::build(&mut cells,geometry,40,&history,&mut dirty)), 0);
    }
    let inspector_lines: Vec<String> = vec!["Terminal Inspector".into(), "Grid      80x24".into(), "Cursor    row 0 col 0".into()];
    for _ in 0..3 { inspector::build(&mut cells, 2, 40, &inspector_lines, &mut dirty); }
    assert_eq!(crate::test_alloc::allocations(|| inspector::build(&mut cells, 2, 40, &inspector_lines, &mut dirty)), 0);
    let link = crate::terminal::grid::Hyperlink { url: "https://example.com/".to_string() + &"a/very/long/path/".repeat(12), row: 0, start_col: 0, end_col: 10, is_path: false };
    for _ in 0..3 { link_preview::build(&mut cells, 40, &link, &mut dirty); }
    assert_eq!(crate::test_alloc::allocations(|| link_preview::build(&mut cells, 40, &link, &mut dirty)), 0);
}

/// Bounded decimal counters and static labels used by overlay builders.
pub(super) struct Label {
    bytes: [u8; 128],
    len: usize,
}
impl Label {
    pub(super) fn new() -> Self { Self { bytes: [0; 128], len: 0 } }
    pub(super) fn as_str(&self) -> &str { std::str::from_utf8(&self.bytes[..self.len]).expect("formatted UTF-8") }
}
impl std::fmt::Write for Label {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let end = self.len + s.len();
        let dst = self.bytes.get_mut(self.len..end).ok_or(std::fmt::Error)?;
        dst.copy_from_slice(s.as_bytes()); self.len = end; Ok(())
    }
}

/// Borrow at most `count` Unicode scalar values, without collecting them.
pub(super) fn prefix(text: &str, count: usize) -> &str {
    &text[..text.char_indices().nth(count).map_or(text.len(), |(i,_)| i)]
}

/// Lazy preview wrapping preserves `str::lines` semantics and empty lines.
fn preview_lines(text: &str, width: usize) -> impl Iterator<Item = &str> {
    let mut remaining = text;
    std::iter::from_fn(move || {
        if remaining.is_empty() { return None; }
        let current = remaining;
        for (count, (offset, ch)) in current.char_indices().enumerate() {
            if ch == '\n' || (ch == '\r' && current[offset..].starts_with("\r\n")) {
                let consumed = if ch == '\r' { 2 } else { 1 };
                remaining = &current[offset + consumed..];
                return Some(&current[..offset]);
            }
            if width > 0 && count == width {
                remaining = &current[offset..];
                return Some(&current[..offset]);
            }
        }
        remaining = "";
        Some(current)
    })
}

#[cfg(test)]
#[test]
fn borrowed_preview_and_prefix_preserve_character_boundaries() {
    assert_eq!(prefix("é😀x", 2), "é😀");
    assert_eq!(prefix("é", 0), "");
    assert_eq!(preview_lines("é😀x\n\nabc\r\n", 2).collect::<Vec<_>>(), ["é😀", "x", "", "ab", "c"]);
    assert_eq!(preview_lines("", 2).count(), 0);
    assert_eq!(preview_lines("abc", 0).collect::<Vec<_>>(), ["abc"]);
    assert_eq!(crate::test_alloc::allocations(|| {
        assert_eq!(preview_lines("é😀x\n\nabc", 2).take(3).count(), 3);
    }), 0);
}

#[cfg(test)]
#[test]
fn preview_wrapping_matches_original_for_all_short_line_shapes() {
    for width in 1..8 {
        for first in ["", "a", "é😀", "abc", "a\r", "abcde"] {
            for ending in ["", "\n", "\r\n", "\n\n", "\nxy\r\n"] {
                let text = format!("{first}{ending}");
                let mut expected = Vec::new();
                for raw in text.lines() {
                    let mut line = raw;
                    while line.chars().count() > width {
                        let part = prefix(line, width);
                        expected.push(part);
                        line = &line[part.len()..];
                    }
                    expected.push(line);
                }
                assert_eq!(preview_lines(&text, width).collect::<Vec<_>>(), expected, "width={width} text={text:?}");
            }
        }
    }
}
