//! `Grid::feed_bytes` (batched ASCII runs) must leave the grid exactly as the
//! byte-at-a-time `feed_byte` path does, whatever the chunk boundaries.

use super::*;

fn snapshot(g: &Grid) -> impl PartialEq + std::fmt::Debug {
    let mut active_cells = Vec::with_capacity((g.history + g.rows) * g.cols * std::mem::size_of::<Cell>());
    let mut active_wrapped = Vec::with_capacity(g.history + g.rows);
    for i in 0..g.history + g.rows {
        let p = (g.top + g.cap - g.history + i) % g.cap;
        let s = p * g.cols;
        active_cells.extend_from_slice(bytemuck::cast_slice::<Cell, u8>(&g.cells[s..s + g.cols]));
        active_wrapped.push(g.wrapped[p]);
    }
    (
        active_cells,
        active_wrapped,
        g.top,
        g.history,
        (g.cur.row, g.cur.col),
        g.wrap_pending,
        g.dirty,
        g.all_dirty,
        g.extras_dirty,
        g.lines_pushed,
        g.joiner,
        g.response.clone(),
    )
}

/// Clear the "something changed" flags, which start out set, so the
/// tail of a stream proves it sets them itself.
fn reset_dirty(g: &mut Grid) {
    g.extras_dirty = false;
    g.all_dirty = false;
    g.dirty.clear();
}

fn feed_chunked(g: &mut Grid, parser: &mut vte::Parser, data: &[u8], mut next_chunk: impl FnMut() -> usize) {
    let mut rest = data;
    while !rest.is_empty() {
        let (part, tail) = rest.split_at(next_chunk().clamp(1, rest.len()));
        let mut off = 0;
        while off < part.len() {
            off += g.feed_bytes(parser, &part[off..]);
        }
        rest = tail;
    }
}

/// `prefix` is fed byte-wise to both grids, then the dirty flags are
/// cleared and `tail` is fed byte-wise to one grid and, split into
/// pieces of each size in `chunks`, through `feed_bytes` to another.
fn check_with(cols: usize, rows: usize, prefix: &[u8], tail: &[u8], chunks: &[usize]) {
    let mut want = Grid::new(cols, rows);
    let mut pw = vte::Parser::new();
    for &b in prefix {
        want.feed_byte(&mut pw, b);
    }
    reset_dirty(&mut want);
    for &b in tail {
        want.feed_byte(&mut pw, b);
    }
    for &chunk in chunks {
        let mut got = Grid::new(cols, rows);
        let mut pg = vte::Parser::new();
        for &b in prefix {
            got.feed_byte(&mut pg, b);
        }
        reset_dirty(&mut got);
        feed_chunked(&mut got, &mut pg, tail, || chunk);
        assert_eq!(snapshot(&want), snapshot(&got), "{cols}x{rows} chunk={chunk} prefix={prefix:?} tail={tail:?}");
    }
}

const CHUNKS: [usize; 10] = [1, 2, 3, 5, 7, 16, 31, 33, 64, 8192];

fn check(stream: &str) {
    check_with(20, 5, b"", stream.as_bytes(), &CHUNKS);
}

fn check_after(prefix: &str, tail: &str) {
    check_with(20, 5, prefix.as_bytes(), tail.as_bytes(), &CHUNKS);
}

#[test]
fn plain_runs_wrap_scroll_and_fill_the_last_column() {
    let mut s = String::new();
    for i in 0..40 {
        s.push_str(&format!("{i:03} {}\r\n", "x".repeat(i)));
    }
    check(&s);
    check(&"abcdefghij".repeat(30));
    check(&format!("{}\r{}", "a".repeat(20), "b".repeat(20)));
}

#[test]
fn colors_links_and_modes_match() {
    check("\x1b[1;31mred bold run\x1b[0m plain \x1b[7minverse text here\x1b[27m\r\nnext line of text");
    check("\x1b]8;;https://a.test\x1b\\linked text run\x1b]8;;\x1b\\ and after the link ends");
    check("\x1b[4hinsert mode text\x1b[1;1H>>>>>>>>>>\x1b[4l more");
    check("\x1b[?7lno autowrap, this text is far wider than the grid\x1b[?7h then wraps again");
    check("\x1b(0lqqqqqqqqqqk\x1b(B back to ascii text");
    check("\x1b[2;4r\x1b[4;1Hscroll region text\r\nmore region text\r\nand more region text\r\nlast");
}

#[test]
fn overwriting_wide_glyphs_clears_the_orphaned_half() {
    for n in 1..=7 {
        check(&format!("漢字漢字漢字\r{}", "a".repeat(n)));
        check(&format!("漢字漢字漢字\x1b[1;2H{}", "a".repeat(n)));
        check(&format!("漢字漢字漢字\x1b[1;{}H{}", 20 - n, "b".repeat(n + 2)));
        // The run ends on the left half of a wide glyph: its right half must go.
        check(&format!("\x1b[1;1H漢字漢字漢字漢字漢字\x1b[1;1H{}", "c".repeat(n + 3)));
        check(&format!("\x1b[1;1H漢字漢字漢字漢字漢字\x1b[1;{}H{}", 2 * n, "d".repeat(4)));
    }
}

#[test]
fn overwritten_hyperlink_cells_flag_the_extras_table() {
    let linked = "\x1b]8;;https://a.test\x1b\\linkedlinkedlinked\x1b]8;;\x1b\\\r\n";
    for n in 4..=12 {
        check_after(linked, &format!("\x1b[1;1H{}", "x".repeat(n)));
        check_after(linked, &format!("\x1b[1;{}H{}", 20 - n + 1, "y".repeat(n)));
    }
    check_after("漢字\x1b]8;;https://a.test\x1b\\漢字\x1b]8;;\x1b\\", "\x1b[1;4Hzzzzzz");
}

#[test]
fn overwriting_a_wrapped_row_clears_its_wrap_flag() {
    let long = "w".repeat(30);
    for n in 4..=20 {
        check_after(&long, &format!("\x1b[1;1H{}", "z".repeat(n)));
        check_after(&long, &format!("\x1b[1;{}H{}", 21 - n, "z".repeat(n)));
    }
}

#[test]
fn dirty_rows_are_marked_by_batched_runs() {
    for row in 1..=5 {
        check_after("", &format!("\x1b[{row};3Habcdefgh"));
    }
    check_after("a\r\nb\r\nc\r\nd\r\ne\r\n", "tail text spanning\r\nseveral rows of the grid");
}

#[test]
fn partial_sequences_and_strings_are_never_batched() {
    check("ab\x1b[31mred\x1b[0m cdefgh\x1b[2");
    check("title\x1b]0;a window title with printable text\x07after the title text");
    check("\x1bP1$rsome dcs payload text\x1b\\after the dcs text");
    check("\x1b_Gi=1;QUJD\x1b\\after the apc text");
    check("\x1b[1;31 mjunk before final\x1b[0m more text here");
    check("é accented text then a plain ascii run follows 漢字 and again ascii text");
}

#[test]
fn esc_terminated_strings_do_not_leave_vte_looking_like_ground() {
    // ESC ends the OSC/DCS string and starts a new sequence: the "20;7"
    // below are CSI parameters, not text, even though dispatch fired.
    check("\x1b]\r\nhello   word \x1b[20;7");
    check("\x1b]0;title\x1b[20;7;m after");
    check("\x1bP1$rpayload\x1b[20;7;m after");
    check("\x1b]8;;https://b.test\x1b\\0123456789 text");
    check("\x1b]8;;https://b.test\x070123456789 text");
    check("\x1b]0;a\x1b[1;22;mmore text after the sequence");
}

#[test]
fn invalid_utf8_before_an_ascii_run_is_resynchronized_by_vte() {
    // A lone lead byte or a truncated sequence leaves vte mid-character;
    // the ASCII that follows must go through vte so it can emit U+FFFD.
    let fragments: [&[u8]; 5] = [b"\xc3", b"\xe2\x82", b"\xf0\x9f\x98", b"\x80", b"\xff"];
    for fragment in fragments {
        let mut data = fragment.to_vec();
        data.extend_from_slice(b"abcdefghijklmnopqrstuvwxyz0123456789 more text");
        check_with(20, 5, b"", &data, &CHUNKS);
    }
}

#[test]
fn utf8_text_is_batched_in_ground_state() {
    let mut g = Grid::new(20, 5);
    let mut p = vte::Parser::new();
    g.feed_byte(&mut p, b'a');
    let text = "é漢字 Tiếng 😀";
    assert_eq!(g.feed_bytes(&mut p, text.as_bytes()), text.len(), "the whole run in one call");
    // Not in ground (mid-escape): byte by byte through vte.
    g.feed_byte(&mut p, 0x1b);
    assert_eq!(g.feed_bytes(&mut p, "é".as_bytes()), 1);
}

#[test]
fn utf8_decoder_accepts_exactly_what_std_accepts() {
    let std_char = |b: &[u8]| {
        let len = match b[0] {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => return None,
        };
        let c = std::str::from_utf8(b.get(..len)?).ok()?.chars().next()?;
        (c >= '\u{A0}').then_some(c)
    };
    let mut seqs: Vec<Vec<u8>> = Vec::new();
    for b0 in 0x80..=0xFFu8 {
        seqs.push(vec![b0]);
        for b1 in 0..=0xFFu8 {
            seqs.push(vec![b0, b1]);
            if b0 >= 0xE0 {
                for b2 in [0x00, 0x7F, 0x80, 0x9F, 0xA0, 0xBF, 0xC0, 0xFF] {
                    seqs.push(vec![b0, b1, b2]);
                    if b0 >= 0xF0 {
                        for b3 in [0x7F, 0x80, 0xBF, 0xC0] {
                            seqs.push(vec![b0, b1, b2, b3, b'x']);
                        }
                    }
                }
            }
        }
    }
    for b in &seqs {
        assert_eq!(printable_utf8_char(b), std_char(b), "{b:02x?}");
    }
    for c in ['\u{A0}', 'é', '\u{7FF}', '\u{800}', '漢', '\u{D7FF}', '\u{E000}', '\u{FFFD}', '\u{FFFF}', '\u{10000}', '😀', '\u{10FFFF}'] {
        let mut buf = [0u8; 4];
        assert_eq!(printable_utf8_char(c.encode_utf8(&mut buf).as_bytes()), Some(c));
    }
}

#[test]
fn utf8_text_matches_the_byte_at_a_time_path() {
    check("é ß ñ Tiếng Việt có dấu ─┼─ 漢字かな交じり文 한국어 ok");
    check(&"漢字".repeat(30));
    check("cafe\u{301} nai\u{308}ve e\u{301}\u{302} \u{301}leading mark");
    check("👨\u{200D}👩\u{200D}👧 🇻🇳🇻🇳🇻 ❤\u{FE0F} 👍\u{1F3FD} #\u{FE0F}\u{20E3} 1\u{FE0F}\u{20E3} ok");
    check("\u{A0}\u{A9}\u{AE}\u{AD} © ® \u{200B}\u{2060}\u{FEFF} zero widths");
    // Narrow grids: wide glyphs and clusters at the right edge.
    check_with(3, 3, b"", "漢字漢字 ab漢 a👨\u{200D}👩".as_bytes(), &CHUNKS);
    check_with(1, 2, b"", "漢é字".as_bytes(), &CHUNKS);
}

#[test]
fn utf8_text_respects_modes_and_line_editor() {
    check("\x1b[4hinsert 漢字 mode é\x1b[1;1H漢字>>\x1b[4l more é");
    check("\x1b[?7lno autowrap 漢字漢字漢字漢字漢字 é é\x1b[?7h 漢字");
    check("\x1b(0lqqé漢k\x1b(B back é");
    check("\x1b]133;B\x07❤\u{FE0F} 👨\u{200D}👩 cafe\u{301}\x1b]133;C\x07 ❤\u{FE0F}");
    check("\x1b[?2027h👨\u{200D}👩\u{200D}👧 é\x1b[?2027l 👨\u{200D}👩");
}

#[test]
fn utf8_control_and_invalid_sequences_go_through_vte() {
    // C1 controls, overlong forms, surrogates, out-of-range and truncated
    // sequences, between and before valid text.
    let fragments: [&[u8]; 9] = [
        b"\xc2\x85", b"\xc2\x9b", b"\xc0\xaf", b"\xe0\x80\xaf", b"\xed\xa0\x80",
        b"\xf4\x90\x80\x80", b"\xf8\x88\x80\x80\x80", b"\xe6\xbc", b"\xf0\x9f\x98",
    ];
    for fragment in fragments {
        for (before, after) in [("é", "漢字 text"), ("abcd", "é"), ("", "ü")] {
            let mut data = before.as_bytes().to_vec();
            data.extend_from_slice(fragment);
            data.extend_from_slice(after.as_bytes());
            check_with(20, 5, b"x", &data, &CHUNKS);
        }
    }
}

#[test]
fn delete_and_control_bytes_end_a_run() {
    check("abcd\x7fefgh ijkl");
    check(&format!("{}\x7f{}", "a".repeat(40), "b".repeat(40)));
    check(&format!("{}\x1f{}", "a".repeat(33), "b".repeat(33)));
    check("abcd\x18efgh\x1aijkl\x07mnop\x08qrst\ttuvw");
    check(&format!("{} {}~{}", "a".repeat(31), "b".repeat(31), "c".repeat(31)));
}

#[test]
fn grapheme_joiner_state_is_respected() {
    for tail in ["abcdefgh", "\u{1F468}\u{200D}abcdefgh", "\u{1F468}\u{200D}\u{1F469}\u{200D}abcdefgh ijkl"] {
        check_after("\x1b[?2027h", &format!("x{tail}\r\nmore ascii text"));
        check_after("", &format!("x{tail}\r\nmore ascii text"));
    }
}

/// Small deterministic xorshift so failures are reproducible.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn random_token(r: &mut Rng) -> Vec<u8> {
    const WORDS: [&str; 8] = ["hello", "the quick brown fox ", "x", "0123456789", "   ", "abcdefghijklmnopqrstuvwxyzabcdefghijklmnop", "~!@#$%^&*()", "word "];
    const FINALS: [char; 12] = ['m', 'H', 'J', 'K', 'h', 'l', 'r', 'A', 'B', 'C', 'D', 'G'];
    const RAW: [&[u8]; 5] = [b"\xc3", b"\xe2\x82", b"\x80", b"\xff", b"\xf0\x9f"];
    const ESCS: [&str; 7] = ["\x1b", "\x1b[", "\x1b[3", "\x1b]", "\x1b_", "\x1bP", "\x1b\\"];
    let text: String = match r.below(30) {
        0..=9 => {
            let mut s = String::new();
            for _ in 0..1 + r.below(4) {
                s.push_str(WORDS[r.below(WORDS.len())]);
            }
            s
        }
        10..=11 => "\r\n".into(),
        12 => "\r".into(),
        13 => "\n".into(),
        14 => ["\t", "\x08", "\x7f", "\x07", "\x18", "\x1a", "\x1f", "\0"][r.below(8)].into(),
        15..=17 => {
            let mut s = String::from("\x1b[");
            for _ in 0..r.below(4) {
                s.push_str(&r.below(40).to_string());
                s.push(';');
            }
            s.push(FINALS[r.below(FINALS.len())]);
            s
        }
        18 => ["\x1b[?7l", "\x1b[?7h", "\x1b[4h", "\x1b[4l", "\x1b[?2027h", "\x1b[?2027l", "\x1b[7m", "\x1b[0m", "\x1b[1;31m"][r.below(9)].into(),
        19 => ["\x1b(0", "\x1b(B", "\x1bM", "\x1bD", "\x1bE", "\x1b7", "\x1b8"][r.below(7)].into(),
        20 => ["\x1b]8;;https://a.test\x1b\\", "\x1b]8;;\x1b\\", "\x1b]0;title text here\x07", "\x1b]8;;https://b.test\x07"][r.below(4)].into(),
        21 => ["漢字", "é", "ß", "─┼─", "Tiếng", "\u{301}", "\u{200D}", "👨\u{200D}👩", "🇻🇳", "❤\u{FE0F}"][r.below(10)].into(),
        // Raw invalid UTF-8 fragments, as a binary `cat` would produce.
        22 => return RAW[r.below(RAW.len())].to_vec(),
        23 => ESCS[r.below(ESCS.len())].into(),
        24 => "\x1b_Gi=1;QUJD\x1b\\".into(),
        25 => "\x1bP1$rpayload text\x1b\\".into(),
        _ => WORDS[r.below(WORDS.len())].into(),
    };
    text.into_bytes()
}

fn random_stream(r: &mut Rng, min_tokens: usize, extra_tokens: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for _ in 0..min_tokens + r.below(extra_tokens) {
        out.extend(random_token(r));
    }
    out
}

#[test]
fn random_streams_match_the_byte_at_a_time_path() {
    let mut r = Rng(0x9E37_79B9_7F4A_7C15);
    let iters: u64 = std::env::var("LITTY_FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(4000);
    for iter in 0..iters {
        let cols = 2 + r.below(23);
        let rows = 2 + r.below(7);
        let prefix = random_stream(&mut r, 0, 8);
        let tail = random_stream(&mut r, 4, 50);
        let mut want = Grid::new(cols, rows);
        let mut pw = vte::Parser::new();
        for &b in &prefix {
            want.feed_byte(&mut pw, b);
        }
        reset_dirty(&mut want);
        for &b in &tail {
            want.feed_byte(&mut pw, b);
        }
        let mut got = Grid::new(cols, rows);
        let mut pg = vte::Parser::new();
        for &b in &prefix {
            got.feed_byte(&mut pg, b);
        }
        reset_dirty(&mut got);
        let mut cr = Rng(iter + 1);
        feed_chunked(&mut got, &mut pg, &tail, || 1 + cr.below(48));
        assert_eq!(snapshot(&want), snapshot(&got), "iter={iter} {cols}x{rows} prefix={prefix:?} tail={tail:?}");
    }
}

/// Steady state of a live terminal: the ring is at capacity, so printing
/// batched runs, colored text and scrolling must not touch the allocator.
#[test]
fn steady_state_batched_runs_allocate_nothing() {
    let mut grid = Grid::new(80, 24);
    let mut parser = vte::Parser::new();
    let line = "\x1b[1;32mok\x1b[0m plain ascii run long enough to be batched \x1b[38;5;208mcolor\x1b[0m and more plain text after it\r\n";
    let feed = |g: &mut Grid, p: &mut vte::Parser| {
        let mut off = 0;
        while off < line.len() {
            off += g.feed_bytes(p, &line.as_bytes()[off..]);
        }
    };
    for _ in 0..30_000 {
        feed(&mut grid, &mut parser);
    }
    let allocations = crate::test_alloc::allocations(|| {
        for _ in 0..1_000 {
            feed(&mut grid, &mut parser);
        }
    });
    assert_eq!(allocations, 0, "steady-state batched printing must not allocate");
}
