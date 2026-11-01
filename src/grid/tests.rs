//! Unit tests for the grid: parser behavior, scrollback ring, selection,
//! clusters, images, prompts and the regressions recorded in ROADMAP.md.

use super::*;
use super::colors::{default_bg, default_fg, palette, parse_x11_color, palette_with};
use super::links::osc8_target;
use super::storage::{MAX_SCROLLBACK, SCROLLBACK, lines_for_bytes};

fn feed(g: &mut Grid, s: &str) {
    let mut p = vte::Parser::new();
    for b in s.bytes() {
        g.feed_byte(&mut p, b);
    }
}

#[test]
fn osc52_preserves_supported_payload_and_rejects_oversize_atomically() {
    let mut g = Grid::new(80, 24);
    for (repeats, terminator) in [(1000, "\x07"), ((1 << 20) / 4, "\x1b\\")] {
        let payload = "YWJj".repeat(repeats);
        feed(&mut g, &format!("\x1b]52;c;{payload}{terminator}"));
        assert_eq!(g.clipboard_request.take().unwrap(), "abc".repeat(repeats));
    }
    for length in [(1 << 20) + 4, MAX_OSC_BYTES + 128] {
        g.clipboard_request = Some("keep".into());
        feed(&mut g, &format!("\x1b]52;c;{}\x07", "A".repeat(length)));
        assert_eq!(g.clipboard_request.as_deref(), Some("keep"));
    }
    g.clipboard_request = None;
    feed(&mut g, "\x1b]52;c;YWJj;ignored\x07");
    assert!(g.clipboard_request.is_none(), "extra fields cannot cause partial success");
    for cancel in ['\x18', '\x1a'] {
        feed(&mut g, &format!("\x1b]52;c;YWJj{cancel}"));
        assert!(g.clipboard_request.is_none(), "CAN/SUB must not dispatch a valid-looking prefix");
    }
    feed(&mut g, "\x1b]52;c;eHl6\x07");
    assert_eq!(g.clipboard_request.as_deref(), Some("xyz"), "parser recovers after cancellation");
}

#[test]
fn osc_timeout_cancels_clipboard_without_dispatching_a_prefix() {
    let mut g = Grid::new(80, 24);
    let mut parser = vte::Parser::new();
    for &byte in b"\x1b]52;c;" { g.feed_byte(&mut parser, byte); }
    g.osc_start = Some(std::time::Instant::now() - std::time::Duration::from_secs(6));
    for byte in "YWJj".repeat(1024).bytes().chain(b"\x1b\\".iter().copied()) { g.feed_byte(&mut parser, byte); }
    assert!(g.clipboard_request.is_none());
    for &byte in b"\x1b]52;c;YWJj\x07" { g.feed_byte(&mut parser, byte); }
    assert_eq!(g.clipboard_request.as_deref(), Some("abc"));
}

#[test]
fn kitty_composition_overflow_returns_protocol_error_through_grid() {
    let mut g = Grid::new(80, 24);
    feed(&mut g, "\x1b_Ga=t,i=1,f=32,s=1,v=1;AAAAAA==\x1b\\");
    let key = g.graphics.image(1).unwrap().key;
    for rectangle in [
        "X=4294967295,w=1,h=1", "Y=4294967295,w=1,h=1",
        "x=4294967295,w=1,h=1", "y=4294967295,w=1,h=1",
        "X=1,w=4294967295,h=1", "Y=1,w=1,h=4294967295",
        "x=1,w=4294967295,h=1", "y=1,w=1,h=4294967295",
        "X=4294967295,Y=4294967295,w=4294967295,h=4294967295",
    ] {
        g.response.clear();
        feed(&mut g, &format!("\x1b_Ga=c,i=1,r=1,c=1,{rectangle}\x1b\\"));
        assert_eq!(g.response, b"\x1b_Gi=1;EINVAL:rectangle out of bounds\x1b\\", "{rectangle}");
        assert_eq!(g.graphics.image(1).unwrap().key, key);
        assert_eq!(&*g.graphics.image(1).unwrap().rgba, &[0; 4]);
    }
    feed(&mut g, "still responsive");
    assert_eq!(g.display_row(0)[0].codepoint, 's' as u32);
}

fn line(g: &Grid, r: usize) -> String {
    g.display_row(r)
        .iter()
        .map(|c| char::from_u32(c.codepoint).filter(|&c| c != '\0').unwrap_or(' '))
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[test]
fn ai_bridge_requires_user_arming_and_bounds_payloads() {
    let mut g=Grid::new(20,6);
    feed(&mut g,"\x1b]778;yo;aGVsbG8=\x07");assert!(g.ai.request.is_none());
    g.ai.armed=true;feed(&mut g,"\x1b]778;yo;aGVsbG8=\x07");assert_eq!(g.ai.request.take().as_deref(),Some("hello"));assert!(!g.ai.armed);
    g.ai.armed=true;feed(&mut g,"\x1b]778;yo;%%%\x07");assert!(g.ai.request.is_none());
    g.ai.armed=true;feed(&mut g,"\x1b[?1049h\x1b]778;yo;aGVsbG8=\x07");assert!(g.ai.request.is_none());
}

#[test]
fn precise_scroll_moves_each_pixel_and_only_rebuilds_at_row_boundaries() {
    let mut g = Grid::new(8, 3);
    feed(&mut g, "a\r\nb\r\nc\r\nd\r\ne\r\nf");
    assert!(g.history >= 3);
    for step in 1..=24 {
        g.all_dirty = false;
        assert!(g.scroll_pixels(1.0, 24.0));
        assert!((g.display_offset as f64 - g.scroll_fraction - step as f64 / 24.0).abs() < 1e-9);
        assert_eq!(g.all_dirty, step == 1);
        assert!(g.viewport_dirty);
        // The extra row is backed by real newer content, never an empty strip.
        if g.scroll_fraction > 0.0 { assert_eq!(g.display_row(g.rows)[0].codepoint, b'f' as u32); }
    }
    assert_eq!(g.scroll_fraction, 0.0);
    assert!(g.scroll_pixels(-24.0, 24.0));
    assert_eq!((g.display_offset, g.scroll_fraction), (0, 0.0));
}

#[test]
fn precise_scroll_clamps_reverses_and_hit_tests_partial_rows() {
    let mut g = Grid::new(8, 3);
    feed(&mut g, "a\r\nb\r\nc\r\nd\r\ne\r\nf");
    assert!(!g.scroll_pixels(-4.0, 24.0));
    g.scroll_pixels(2.0, 24.0);
    assert_eq!(g.viewport_row(0.0, 24.0), 0);
    assert_eq!(g.viewport_row(3.0, 24.0), 1);
    assert_eq!(g.viewport_row(71.0, 24.0), 3);
    g.scroll_pixels(100_000.0, 24.0);
    assert_eq!((g.display_offset, g.scroll_fraction), (g.history, 0.0));
    assert!(!g.scroll_pixels(2.0, 24.0));
    g.scroll_pixels(-2.0, 24.0);
    assert!(g.scroll_fraction > 0.0);
    g.scroll_display(0);
    assert_eq!(g.scroll_fraction, 0.0);
    g.scroll_pixels(-2.0, 24.0);
    g.reset_display();
    assert_eq!((g.display_offset, g.scroll_fraction), (0, 0.0));
    feed(&mut g, "\x1b[?1049h");
    assert!(!g.scroll_pixels(2.0, 24.0));
    assert!(!g.scroll_pixels(f64::NAN, 24.0));
}

#[test]
fn precise_scroll_allocates_nothing_and_preserves_cell_layout() {
    let mut g = Grid::new(8, 3);
    feed(&mut g, "a\r\nb\r\nc\r\nd\r\ne\r\nf");
    assert_eq!(std::mem::size_of::<Cell>(), 12);
    let count = crate::test_alloc::allocations(|| {
        for _ in 0..10_000 { g.scroll_pixels(2.0, 24.0); g.scroll_pixels(-2.0, 24.0); }
    });
    assert_eq!(count, 0);
}

#[test]
fn ai_scrollback_joins_wraps_and_excludes_prompt() {
    let mut g=Grid::new(10,6);
    feed(&mut g,"abcdefghijk\r\nsecond\r\n\x1b]133;A\x07$ \x1b]133;B\x07yo help");
    assert_eq!(g.extract_scrollback_lines(1),"second\n");
    assert_eq!(g.extract_scrollback_lines(2),"abcdefghijk\nsecond\n");
    assert_eq!(g.extract_scrollback_lines(0),"");
    feed(&mut g,"\x1b[?1049h");assert!(g.extract_scrollback_lines(200).is_empty());
}

#[test]
fn ai_execution_reads_full_edited_wrapped_input_and_exit() {
    let mut g=Grid::new(10,8);
    feed(&mut g,"\x1b]133;A\x07$ \x1b]133;B\x07printf hello_world");
    feed(&mut g,"\x1b[D\x1b[D");
    assert_eq!(g.full_prompt_input(),"printf hello_world");
    g.ai.track=true;feed(&mut g,"\r\n\x1b]133;C\x07output\r\n\x1b]133;D;7\x07\x1b]133;A\x07$ \x1b]133;B\x07");
    assert_eq!(g.ai.command.as_deref(),Some("printf hello_world"));assert_eq!(g.last_exit,Some(7));assert_eq!(g.prompt_epoch,2);
}

#[test]
fn combining_marks_compose_into_precomposed_chars() {
    let mut g = Grid::new(20, 3);
    // NFD "Việt Nam": e + U+0323 (dot below) + U+0302 (circumflex).
    feed(&mut g, "Vie\u{0323}\u{0302}t Nam");
    assert_eq!(line(&g, 0), "Việt Nam");
    assert_eq!(g.cur.col, 8);

    // Mark at the very start of a line has no base and is dropped.
    feed(&mut g, "\r\n\u{0301}a");
    assert_eq!(line(&g, 1), "a");
}

#[test]
fn combining_mark_at_right_margin_uses_pending_wrap_cell() {
    let mut g = Grid::new(4, 2);
    feed(&mut g, "abce\u{0301}");
    assert_eq!(line(&g, 0), "abcé");
    feed(&mut g, "x");
    assert_eq!(line(&g, 1), "x");
}

#[test]
fn unicode_widths_follow_east_asian_and_emoji_tables() {
    assert_eq!(char_width('a'), 1);
    assert_eq!(char_width('ệ'), 1);
    assert_eq!(char_width('中'), 2);
    assert_eq!(char_width('🚀'), 2);
    assert_eq!(char_width('\u{0301}'), 0);
    assert_eq!(char_width('\u{200D}'), 0);

    let mut g = Grid::new(10, 2);
    feed(&mut g, "🚀x");
    assert_eq!(g.display_row(0)[0].codepoint, '🚀' as u32);
    assert_eq!(g.display_row(0)[1].codepoint, 0);
    assert_eq!(g.display_row(0)[2].codepoint, 'x' as u32);
}

#[test]
fn print_and_wrap() {
    let mut g = Grid::new(5, 3);
    feed(&mut g, "hello world");
    assert_eq!(line(&g, 0), "hello");
    assert_eq!(line(&g, 1), " worl");
    assert_eq!(line(&g, 2), "d");
}

#[test]
fn scroll_into_history() {
    let mut g = Grid::new(4, 2);
    feed(&mut g, "a\r\nb\r\nc\r\nd");
    assert_eq!(line(&g, 0), "c");
    assert_eq!(line(&g, 1), "d");
    g.scroll_display(2);
    assert_eq!(line(&g, 0), "a");
    assert_eq!(line(&g, 1), "b");
}

#[test]
fn csi_cursor_and_erase() {
    let mut g = Grid::new(6, 3);
    feed(&mut g, "abcdef\x1b[2;3HX\x1b[1;1H\x1b[K");
    assert_eq!(line(&g, 0), "");
    assert_eq!(line(&g, 1), "  X");
}

#[test]
fn sgr_colors() {
    let mut g = Grid::new(4, 1);
    feed(&mut g, "\x1b[31;48;2;1;2;3mx");
    let c = g.display_row(0)[0];
    assert_eq!(c.fg, palette(1) | FG_SET);
    assert_eq!(c.bg, 0x0102_0300 | BG_SET);
}

#[test]
fn bold_and_italic_are_stored_in_fg_flags() {
    let mut g = Grid::new(8, 1);
    feed(&mut g, "\x1b[1ma\x1b[3mb\x1b[22mc\x1b[0;31;1md");
    let row = g.display_row(0);
    assert_eq!(row[0].fg, FG_BOLD);
    assert_eq!(row[1].fg, FG_BOLD | FG_ITALIC);
    assert_eq!(row[2].fg, FG_ITALIC);
    assert_eq!(row[3].fg, palette(1) | FG_SET | FG_BOLD);
}

#[test]
fn sgr_underline_styles_and_strikethrough() {
    let mut g = Grid::new(10, 1);
    feed(&mut g, "\x1b[4ma\x1b[4:3mb\x1b[21mc\x1b[4:0md\x1b[4:4;9me\x1b[29mf\x1b[4:5mg\x1b[24mh");
    let style = |c: &Cell| (c.bg & BG_UNDERLINE_MASK, c.bg & BG_STRIKE != 0);
    let row: Vec<_> = g.display_row(0)[..8].iter().map(style).collect();
    assert_eq!(
        row,
        [(1, false), (3, false), (2, false), (0, false), (4, true), (4, false), (5, false), (0, false)]
    );
}

#[test]
fn sgr_58_sets_underline_color_without_touching_colors() {
    let mut g = Grid::new(8, 1);
    feed(&mut g, "\x1b[4;31;58;2;255;0;16ma\x1b[58:5:2mb\x1b[59mc\x1b[58:2::1:2:3;0md");
    let row = g.display_row(0);
    assert_eq!(row[0].fg & !FG_EXTRA, palette(1) | FG_SET);
    assert_eq!(row[0].bg & !BG_EXTRA, BG_UNDERLINE);
    assert_ne!(row[0].extra(), 0);
    assert_eq!(g.underline_color(&row[0]), Some(0xFF00_1000));
    assert_eq!(g.underline_color(&row[1]), Some(palette(2)));
    assert_eq!(g.underline_color(&row[2]), None);
    assert_eq!(g.underline_color(&row[3]), None, "SGR 0 resets the underline color");
    assert_eq!(row[3], Cell { codepoint: 'd' as u32, fg: 0, bg: 0 });
}

#[test]
fn osc8_hyperlink_spans_its_cells() {
    let mut g = Grid::new(20, 1);
    feed(&mut g, "\x1b]8;id=x;https://example.com/a;b\x1b\\link\x1b]8;;\x1b\\ plain");
    let link = g.find_hyperlink_at(0, 2, None).expect("osc 8 link");
    assert_eq!((link.start_col, link.end_col), (0, 3));
    assert_eq!(link.url, "https://example.com/a;b");
    assert!(!link.is_path);
    assert_eq!(g.find_hyperlink_at(0, 6, None), None);
}

#[test]
fn osc8_survives_sgr_reset_and_adjacent_links_stay_apart() {
    let mut g = Grid::new(20, 1);
    feed(&mut g, "\x1b]8;;http://a.test\x07a\x1b[0;1mb\x1b]8;;http://b.test\x07c\x1b]8;;\x07d");
    let a = g.find_hyperlink_at(0, 1, None).expect("link a");
    assert_eq!((a.start_col, a.end_col, a.url.as_str()), (0, 1, "http://a.test"));
    let b = g.find_hyperlink_at(0, 2, None).expect("link b");
    assert_eq!((b.start_col, b.end_col, b.url.as_str()), (2, 2, "http://b.test"));
    assert_eq!(g.find_hyperlink_at(0, 3, None), None);
}

#[test]
fn osc8_targets() {
    assert_eq!(osc8_target("file://host/tmp/a%20b.txt"), Some(("/tmp/a b.txt".into(), true)));
    assert_eq!(osc8_target("HTTPS://x.test"), Some(("HTTPS://x.test".into(), false)));
    assert_eq!(osc8_target("mailto:me@x.test"), Some(("mailto:me@x.test".into(), false)));
    assert_eq!(osc8_target("javascript:alert(1)"), None);
    assert_eq!(osc8_target("-a Calculator"), None);
    let mut g = Grid::new(10, 1);
    feed(&mut g, "\x1b]8;;x-evil://run\x1b\\click\x1b]8;;\x1b\\");
    assert_eq!(g.find_hyperlink_at(0, 1, None), None);
}

#[test]
fn extra_slots_are_reclaimed_once_cells_stop_using_them() {
    let mut g = Grid::new(4, 2);
    for i in 0..600 {
        feed(&mut g, &format!("\x1b]8;;https://e.test/{i}\x1b\\x\x1b]8;;\x1b\\\r"));
    }
    let link = g.find_hyperlink_at(0, 0, None).expect("latest link");
    assert_eq!(link.url, "https://e.test/599");
    assert!(g.extras.len() <= 256);
}

#[test]
fn kitty_keyboard_stack_per_screen() {
    let mut g = Grid::new(10, 3);
    feed(&mut g, "\x1b[?u");
    assert_eq!(std::mem::take(&mut g.response), b"\x1b[?0u");
    feed(&mut g, "\x1b[>1u\x1b[>11u");
    assert_eq!(g.kitty_keyboard_flags(), 11);
    feed(&mut g, "\x1b[=4;2u");
    assert_eq!(g.kitty_keyboard_flags(), 15);
    feed(&mut g, "\x1b[=1;3u\x1b[?u");
    assert_eq!(std::mem::take(&mut g.response), b"\x1b[?14u");
    feed(&mut g, "\x1b[<u");
    assert_eq!(g.kitty_keyboard_flags(), 1);

    // The alternate screen has its own stack.
    feed(&mut g, "\x1b[?1049h");
    assert_eq!(g.kitty_keyboard_flags(), 0);
    feed(&mut g, "\x1b[>31u");
    assert_eq!(g.kitty_keyboard_flags(), 31);
    feed(&mut g, "\x1b[?1049l");
    assert_eq!(g.kitty_keyboard_flags(), 1);

    // Popping more than was pushed leaves the flags at zero.
    feed(&mut g, "\x1b[<5u");
    assert_eq!(g.kitty_keyboard_flags(), 0);
}

#[test]
fn kitty_stack_is_bounded_and_does_not_restore_the_cursor() {
    let mut g = Grid::new(10, 3);
    feed(&mut g, "\x1b[2;3H\x1b[s\x1b[1;1H");
    for i in 1..=12 {
        feed(&mut g, &format!("\x1b[>{i}u"));
    }
    assert_eq!((g.cur.row, g.cur.col), (0, 0), "CSI > u is not DECRC");
    feed(&mut g, "\x1b[<8u");
    assert_eq!(g.kitty_keyboard_flags(), 4);
    feed(&mut g, "\x1b[<u");
    assert_eq!(g.kitty_keyboard_flags(), 0, "oldest entries were dropped");
    feed(&mut g, "\x1b[u");
    assert_eq!((g.cur.row, g.cur.col), (1, 2));
}

#[test]
fn scrollback_limit_is_bytes_per_width() {
    assert_eq!(lines_for_bytes(usize::MAX, 80), SCROLLBACK);
    assert_eq!(lines_for_bytes(80 * 12 * 100, 80), 100);
    assert_eq!(lines_for_bytes(80 * 12 * 100, 160), 50);
    assert_eq!(lines_for_bytes(0, 80), 0);
    assert_eq!(lines_for_bytes(usize::MAX - 1, 1), MAX_SCROLLBACK);
}

#[test]
fn select_all_spans_scrollback_and_skips_blank_edges() {
    let mut g = Grid::new(6, 3);
    feed(&mut g, "\r\none\r\ntwo\r\nthree\r\nfour");
    g.select_all();
    assert_eq!(g.selected_text().as_deref(), Some("one\ntwo\nthree\nfour"));
    g.clear_history();
    feed(&mut g, "\x1b[2J\x1b[H");
    g.select_all();
    assert_eq!(g.selected_text(), None, "nothing to select on an empty screen");
}

#[test]
fn shift_click_extends_from_the_far_end() {
    let mut g = Grid::new(10, 3);
    feed(&mut g, "abcdefghij\r\nklmnopqrst\r\nuvwxyz");
    assert!(!g.extend_selection(0, 0), "no selection to extend");
    g.set_selection(Some(Selection { start: (1, 2), end: (1, 4) }));
    // After the selection: the start stays.
    assert!(g.extend_selection(2, 1));
    assert_eq!(g.selected_text().as_deref(), Some("mnopqrst\nuv"));
    // Before it: the far end (now 2,1) stays the anchor.
    assert!(g.extend_selection(0, 8));
    assert_eq!(g.selected_text().as_deref(), Some("ij\nklmnopqrst\nuv"));
    // Dragging on keeps that anchor.
    g.set_selection_end(1, 0);
    assert_eq!(g.selected_text().as_deref(), Some("klmnopqrst\nuv"));
}

#[test]
fn parses_x11_color_specs() {
    assert_eq!(parse_x11_color(b"rgb:ff/80/00"), Some(0xFF80_0000));
    assert_eq!(parse_x11_color(b"rgb:ffff/8080/0000"), Some(0xFF80_0000));
    assert_eq!(parse_x11_color(b"rgb:f/8/0"), Some(0xFF88_0000));
    assert_eq!(parse_x11_color(b"#1e1e2e"), Some(0x1E1E_2E00));
    assert_eq!(parse_x11_color(b"#f80"), Some(0xF080_0000), "XParseColor: most significant bits");
    assert_eq!(parse_x11_color(b"#ffff80800000"), Some(0xFF80_0000));
    assert_eq!(parse_x11_color(b"red"), Some(0xFF00_0000));
    assert_eq!(parse_x11_color(b"Slate Blue"), Some(0x6A5A_CD00));
    assert_eq!(parse_x11_color(b"nope"), None);
    assert_eq!(parse_x11_color(b"rgb:ff/80"), None);
    assert_eq!(parse_x11_color(b"#12345"), None);
}

#[test]
fn kitty_and_sixel_images_are_placed_at_the_cursor() {
    let mut g = Grid::new(20, 5);
    g.cell_px = (10, 20);
    // 30x40 px RGBA (all zero) = 3x2 cells; base64 of 4800 zero bytes.
    let payload = "A".repeat(6400);
    feed(&mut g, &format!("ab\x1b_Ga=T,f=32,s=30,v=40,i=1;{payload}\x1b\\"));
    assert_eq!(String::from_utf8(std::mem::take(&mut g.response)).unwrap(), "\x1b_Gi=1;OK\x1b\\");
    let p = &g.graphics.placements[0];
    assert_eq!((p.line, p.col, p.cols, p.rows), (0, 2, 3, 2));
    assert_eq!((g.cur.row, g.cur.col), (1, 5), "cursor after the image's last column, on its last row");
    assert_eq!(line(&g, 0), "ab", "the APC string is not printed");

    let mut drawn = Vec::new();
    g.visible_images(&mut drawn);
    assert_eq!((drawn.len(), drawn[0].row, drawn[0].col), (1, 0, 2));

    // Scrolling moves it up with the text, then out of view.
    feed(&mut g, "\r\n\n\n\n");
    drawn.clear();
    g.visible_images(&mut drawn);
    assert_eq!(drawn[0].row, -1);

    // Sixel: 6 px tall = 1 row at 20 px cells; cursor goes below it.
    feed(&mut g, "\x1b[H\x1bPq#0;2;100;0;0#0~~~\x1b\\");
    assert_eq!(g.graphics.placements.len(), 2);
    assert_eq!((g.cur.row, g.cur.col), (1, 0));

    // A full reset forgets everything; DA1 advertises Sixel.
    feed(&mut g, "\x1bc\x1b[c");
    assert!(g.graphics.placements.is_empty());
    assert_eq!(g.response, b"\x1b[?62;4;22c");
}

/// Visible text of row `r` with clusters spelled out and wide halves skipped.
fn cluster_line(g: &Grid, r: usize) -> String {
    let mut out = String::new();
    for c in g.display_row(r).iter().filter(|c| c.codepoint != 0) {
        g.push_cell_text(c, &mut out);
    }
    out
}

#[test]
fn grapheme_clusters_in_mode_2027() {
    let mut g = Grid::new(20, 3);
    g.modes.grapheme_mode = true;
    // ZWJ family, flags (two indicators each), skin tone: two cells apiece.
    feed(&mut g, "👨\u{200D}👩\u{200D}👧|🇻🇳🇺🇸|👍🏽|");
    assert_eq!(cluster_line(&g, 0), "👨\u{200D}👩\u{200D}👧|🇻🇳🇺🇸|👍🏽|");
    let row = g.display_row(0);
    assert!(row[0].codepoint & CLUSTER_FLAG != 0 && is_wide_cp(row[0].codepoint));
    assert_eq!((row[2].codepoint, g.cluster(row[3].codepoint)), ('|' as u32, Some("🇻🇳")));
    assert!(is_wide_cp(row[3].codepoint));
    assert_eq!(g.cluster(row[5].codepoint), Some("🇺🇸"));
    assert_eq!(g.cur.col, 11);

    // VS16 widens a text-default symbol; the cursor moves past both halves.
    feed(&mut g, "\r\n❤\u{FE0F}x#\u{FE0F}\u{20E3}y");
    let row = g.display_row(1);
    assert_eq!((g.cluster(row[0].codepoint), row[2].codepoint), (Some("❤\u{FE0F}"), 'x' as u32));
    assert_eq!((g.cluster(row[3].codepoint), row[5].codepoint), (Some("#\u{FE0F}\u{20E3}"), 'y' as u32));

    // Copy keeps the whole sequence; search sees one char per cell.
    g.select_all();
    assert_eq!(g.selected_text().unwrap(), "👨\u{200D}👩\u{200D}👧|🇻🇳🇺🇸|👍🏽|\n❤\u{FE0F}x#\u{FE0F}\u{20E3}y");
    assert_eq!(g.cell_char(&g.display_row(0)[0]), '👨');

    // Mode 2027 off: every emoji keeps its own cells (ZWJ folds into the one before).
    feed(&mut g, "\x1b[?2027l\x1b[?2027$p\r\n👨\u{200D}👩|");
    assert_eq!(std::mem::take(&mut g.response), b"\x1b[?2027;2$y");
    assert_eq!(g.cluster(g.display_row(2)[0].codepoint), Some("👨\u{200D}"));
    assert_eq!(g.display_row(2)[4].codepoint, '|' as u32);
}

#[test]
fn rtl_rows_map_between_visual_and_logical_columns() {
    let mut g = Grid::new(6, 1);
    feed(&mut g, "a אבג");
    // Drawn as "a גבא": visual column 2 shows logical column 4 (ג).
    assert_eq!((g.logical_col(0, 2), g.cursor_visual_col(0, 4)), (4, 2));
    assert_eq!(g.logical_col(0, 0), 0);
    // A cursor right after left-to-right text stays next to it, not at
    // the right end of the run its cell starts.
    assert_eq!(g.cursor_visual_col(0, 2), 2);
    // After a right-to-left letter it follows its own cell.
    assert_eq!(g.cursor_visual_col(0, 3), 3);
    // Typing `echo '` before a right-to-left autosuggestion.
    let mut g = Grid::new(14, 1);
    feed(&mut g, "echo 'سلام'\x1b[7G");
    assert_eq!(g.cursor_pos().map(|(x, _)| g.cursor_visual_col(0, x)), Some(6));
    // Full-screen applications get the cells as they are.
    feed(&mut g, "\x1b[?1049h\x1b[Hאב");
    assert_eq!(g.logical_col(0, 0), 0);
}

#[test]
fn marks_without_a_precomposed_form_become_clusters() {
    let mut g = Grid::new(10, 1);
    g.modes.grapheme_mode = false;
    feed(&mut g, "q\u{0303}e\u{0301}z");
    let row = g.display_row(0);
    assert_eq!(g.cluster(row[0].codepoint), Some("q\u{0303}"));
    assert_eq!(row[1].codepoint, 'é' as u32, "precomposed when possible");
    assert_eq!((row[2].codepoint, g.cur.col), ('z' as u32, 3));
    assert_eq!(cluster_line(&g, 0), "q\u{0303}éz");
}

#[test]
fn command_line_input_keeps_per_codepoint_widths() {
    // At the prompt (OSC 133 B..C) zsh counts 👍🏽 as 4 columns and ❤️ as 1.
    let mut g = Grid::new(20, 2);
    g.modes.grapheme_mode = true;
    feed(&mut g, "\x1b]133;A\x07$ \x1b]133;B\x07👍🏽❤\u{FE0F}x");
    assert_eq!(g.cur.col, 2 + 4 + 1 + 1);
    assert_eq!(g.cluster(g.display_row(0)[6].codepoint), Some("❤\u{FE0F}"));
    // Command output clusters as usual.
    feed(&mut g, "\r\n\x1b]133;C\x07👍🏽❤\u{FE0F}x");
    assert_eq!(g.cur.col, 2 + 2 + 1);
    // Narrow-by-default pictographs are two columns there too (zsh's table).
    let mut g = Grid::new(20, 2);
    feed(&mut g, "\x1b]133;A\x07$ \x1b]133;B\x07🏳\u{FE0F}x");
    assert_eq!(g.cur.col, 2 + 2 + 1);
    assert_eq!(g.cluster(g.display_row(0)[2].codepoint), Some("🏳\u{FE0F}"));
    // A flag: two regional indicators of two columns each at the prompt,
    // one two-column cluster in the output.
    let mut g = Grid::new(20, 2);
    g.modes.grapheme_mode = true;
    feed(&mut g, "\x1b]133;A\x07$ \x1b]133;B\x07🇻🇳");
    assert_eq!(g.cur.col, 2 + 4);
    feed(&mut g, "\r\n\x1b]133;C\x07🇻🇳");
    assert_eq!(g.cur.col, 2);
}

#[cfg(target_os = "macos")]
#[test]
fn spacing_marks_follow_the_system_wcwidth() {
    // zsh on macOS counts "कि नमस्ते" as 6 columns (U+093F is zero-width
    // there); any other count desyncs the line editor's cursor.
    let mut g = Grid::new(20, 1);
    feed(&mut g, "कि नमस्ते");
    assert_eq!(g.cur.col, 6);
    assert_eq!(g.cluster(g.display_row(0)[0].codepoint), Some("कि"));
}

#[test]
fn kitty_unicode_placeholders_show_virtual_placements() {
    let mut g = Grid::new(10, 3);
    g.cell_px = (10, 20);
    // 20x40 px image, virtual placement in a 2x2 cell box (20x40 px): each
    // cell row shows half of it.
    let payload = "A".repeat(4267);
    feed(&mut g, &format!("\x1b_Ga=t,f=32,s=20,v=40,i=7,q=2;{payload}\x1b\\\x1b_Ga=p,U=1,i=7,c=2,r=2,q=2\x1b\\"));
    assert!(g.graphics.placements.is_empty(), "nothing placed at the cursor");
    assert_eq!((g.cur.row, g.cur.col), (0, 0), "a virtual placement does not move the cursor");
    let d = |i: usize| char::from_u32(crate::graphics::PLACEHOLDER_DIACRITICS[i]).unwrap();
    // Row 0 with explicit row + column, the next cell inherits; row 1 by
    // 256-color index 7 with only a row diacritic.
    feed(&mut g, &format!("\x1b[38;2;0;0;7m\u{10EEEE}{}{}\u{10EEEE}\r\n", d(0), d(0)));
    feed(&mut g, &format!("\x1b[38;5;7mx\u{10EEEE}{}\u{10EEEE}\x1b[0m", d(1)));
    assert_eq!(line(&g, 0), "", "placeholders have no text");
    let mut drawn = Vec::new();
    g.visible_images(&mut drawn);
    assert_eq!(drawn.len(), 2);
    assert_eq!((drawn[0].row, drawn[0].col, drawn[0].cols), (0, 0, 2));
    assert_eq!(drawn[0].src, [0.0, 0.0, 20.0, 20.0]);
    assert_eq!(drawn[0].px, Some([0.0, 0.0, 20.0, 20.0]));
    assert_eq!((drawn[1].row, drawn[1].col), (1, 1));
    assert_eq!(drawn[1].src, [0.0, 20.0, 20.0, 20.0], "second image row");

    // A wider box keeps the aspect ratio: the image is centered horizontally.
    feed(&mut g, "\x1b_Ga=p,U=1,i=7,c=4,r=2,q=2\x1b\\");
    drawn.clear();
    g.visible_images(&mut drawn);
    assert_eq!(drawn[0].px, Some([10.0, 0.0, 10.0, 20.0]), "starts half a cell into the box");

    // Deleting the image removes the virtual placement.
    feed(&mut g, "\x1b_Ga=d,d=I,i=7\x1b\\");
    drawn.clear();
    g.visible_images(&mut drawn);
    assert!(drawn.is_empty());
}

#[test]
fn osc4_recolors_text_already_on_screen() {
    let mut g = Grid::new(8, 1);
    feed(&mut g, "\x1b[31mr\x1b[42mg\x1b[0mn");
    feed(&mut g, "\x1b]4;1;#00ff00\x07");
    let row = g.display_row(0);
    assert_eq!(row[0].fg, 0x00FF_0000 | FG_SET, "red text is now green");
    assert_eq!(row[1].bg & !0xFF, palette(2), "other colors untouched");
    assert_eq!(row[2].fg, 0, "default color untouched");
    feed(&mut g, "\x1b]104;1\x07");
    assert_eq!(g.display_row(0)[0].fg, palette(1) | FG_SET, "reset brings it back");
}

#[test]
fn osc4_sets_queries_and_resets_the_palette() {
    let mut g = Grid::new(8, 1);
    feed(&mut g, "\x1b]4;1;rgb:12/34/56;200;#abcdef\x07\x1b[31mx\x1b[38;5;200my");
    let row = g.display_row(0);
    assert_eq!(row[0].fg, 0x1234_5600 | FG_SET);
    assert_eq!(row[1].fg, 0xABCD_EF00 | FG_SET);
    feed(&mut g, "\x1b]4;1;?;2;?\x1b\\");
    let reply = format!(
        "\x1b]4;1;rgb:1212/3434/5656\x1b\\\x1b]4;2;rgb:{0:02x}{0:02x}/{1:02x}{1:02x}/{2:02x}{2:02x}\x1b\\",
        palette(2) >> 24,
        (palette(2) >> 16) & 0xFF,
        (palette(2) >> 8) & 0xFF
    );
    assert_eq!(String::from_utf8(std::mem::take(&mut g.response)).unwrap(), reply);
    feed(&mut g, "\x1b]104;1\x07\x1b[31mz");
    assert_eq!(g.display_row(0)[2].fg, palette(1) | FG_SET);
    assert_eq!(palette_with(&g.palette_override, 200), 0xABCD_EF00, "only index 1 was reset");
    feed(&mut g, "\x1b]104\x07");
    assert!(g.palette_override.is_empty());
}

#[test]
fn osc_10_11_12_set_query_and_reset_dynamic_colors() {
    let mut g = Grid::new(8, 1);
    feed(&mut g, "\x1b]10;?\x07");
    let fg = default_fg();
    let expected = format!(
        "\x1b]10;rgb:{0:02x}{0:02x}/{1:02x}{1:02x}/{2:02x}{2:02x}\x07",
        fg >> 24,
        (fg >> 16) & 0xFF,
        (fg >> 8) & 0xFF
    );
    assert_eq!(String::from_utf8(std::mem::take(&mut g.response)).unwrap(), expected);

    // `10;fg;bg` sets fg and then bg; `11;?;?` queries bg and cursor.
    feed(&mut g, "\x1b]10;#102030;#405060\x07\x1b]12;rgb:ff/00/00\x07");
    assert_eq!((g.fg_color(), g.bg_color(), g.cursor_color()), (0x1020_3000, 0x4050_6000, Some(0xFF00_0000)));
    feed(&mut g, "\x1b]11;?;?\x07");
    assert_eq!(
        std::mem::take(&mut g.response),
        b"\x1b]11;rgb:4040/5050/6060\x07\x1b]12;rgb:ffff/0000/0000\x07"
    );

    // Inverse video resolves the overridden defaults.
    feed(&mut g, "\x1b[7mx");
    assert_eq!(g.display_row(0)[0].fg & !0xFF, 0x4050_6000);

    feed(&mut g, "\x1b]110\x07\x1b]111\x07\x1b]112\x07");
    assert_eq!((g.fg_color(), g.bg_color(), g.cursor_color()), (default_fg(), default_bg(), None));

    feed(&mut g, "\x1b]11;#000000\x07\x1bc");
    assert_eq!(g.bg_color(), default_bg(), "RIS drops OSC color overrides");
}

#[test]
fn modify_other_keys_set_query_and_reset() {
    let mut g = Grid::new(8, 1);
    feed(&mut g, "\x1b[>4;2m");
    assert_eq!(g.modes.modify_other_keys, 2);
    feed(&mut g, "\x1b[?4m");
    assert_eq!(std::mem::take(&mut g.response), b"\x1b[>4;2m");
    feed(&mut g, "\x1b[>4m");
    assert_eq!(g.modes.modify_other_keys, 0);
    feed(&mut g, "\x1b[>4;1m\x1b[>4n");
    assert_eq!(g.modes.modify_other_keys, 0);
    feed(&mut g, "\x1b[>4;9m");
    assert_eq!(g.modes.modify_other_keys, 2, "clamped");
    assert_eq!(g.display_row(0)[0], Cell::default(), "not mistaken for SGR");
}

#[test]
fn notifications_progress_pointer_and_bell() {
    let mut g = Grid::new(8, 1);
    feed(&mut g, "\x1b]9;build done; 3 warnings\x07");
    assert_eq!(g.notification.take(), Some((String::new(), "build done; 3 warnings".into())));
    feed(&mut g, "\x1b]777;notify;Tests;all \x1b[31mpassed\x1b\\");
    assert_eq!(g.notification.take(), Some(("Tests".into(), "all ".into())));

    feed(&mut g, "\x1b]9;4;1;45\x07");
    assert_eq!(g.progress, Some((1, 45)));
    assert!(std::mem::take(&mut g.title_changed));
    feed(&mut g, "\x1b]9;4;3\x07");
    assert_eq!(g.progress, Some((3, 0)));
    feed(&mut g, "\x1b]9;4;2;250\x07");
    assert_eq!(g.progress, Some((2, 100)));
    feed(&mut g, "\x1b]9;4;0\x07");
    assert_eq!(g.progress, None);

    feed(&mut g, "\x1b]22;pointer\x07");
    assert_eq!(g.pointer_shape, "pointer");
    // The ESC inside the OSC 777 body above ended it; the rest printed as text.
    assert_eq!(line(&g, 0), "passed");
    feed(&mut g, "a\x07b");
    assert!(g.bell);
    assert_eq!(line(&g, 0), "passedab");
}

#[test]
fn xtgettcap_and_decrqss() {
    let mut g = Grid::new(8, 4);
    // "TN" and "Co" known, "zz" unknown, "Tc" boolean.
    feed(&mut g, "\x1bP+q544E;436F;7A7A;5463\x1b\\");
    let reply = String::from_utf8(std::mem::take(&mut g.response)).unwrap();
    assert_eq!(
        reply,
        "\x1bP1+r544E=787465726D2D323536636F6C6F72\x1b\\\x1bP1+r436F=323536\x1b\\\x1bP0+r7A7A\x1b\\\x1bP1+r5463\x1b\\"
    );

    feed(&mut g, "\x1b[1;3;4:3;38;2;1;2;3m\x1bP$qm\x1b\\");
    assert_eq!(String::from_utf8(std::mem::take(&mut g.response)).unwrap(), "\x1bP1$r0;1;3;4:3;38;2;1;2;3m\x1b\\");
    feed(&mut g, "\x1b[2;3r\x1bP$qr\x1b\\\x1b[5 q\x1bP$q q\x1b\\\x1bP$qx\x1b\\");
    assert_eq!(
        String::from_utf8(std::mem::take(&mut g.response)).unwrap(),
        "\x1bP1$r2;3r\x1b\\\x1bP1$r5 q\x1b\\\x1bP0$r\x1b\\"
    );
    assert_eq!(line(&g, 0), "", "DCS payloads are not printed");

    // Non-hex bytes or invalid length queries must be ignored and never echoed
    feed(&mut g, "\x1bP+qevil;\rwhoami\r;544;invalid_hex\x1b\\");
    assert!(g.response.is_empty(), "non-hex and invalid length queries must not be echoed back");
}

#[test]
fn pixel_mouse_color_scheme_and_window_size_reports() {
    let mut g = Grid::new(80, 24);
    g.cell_px = (9, 18);
    feed(&mut g, "\x1b[?1016h\x1b[?2031h\x1b[?1016$p\x1b[?2027$p");
    assert!(g.modes.mouse_sgr_pixels && g.modes.color_scheme_reports);
    assert_eq!(std::mem::take(&mut g.response), b"\x1b[?1016;1$y\x1b[?2027;1$y");
    set_color_scheme_dark(false);
    feed(&mut g, "\x1b[?996n");
    assert_eq!(std::mem::take(&mut g.response), b"\x1b[?997;2n");
    set_color_scheme_dark(true);
    feed(&mut g, "\x1b[14t\x1b[16t\x1b[18t");
    assert_eq!(std::mem::take(&mut g.response), b"\x1b[4;432;720t\x1b[6;18;9t\x1b[8;24;80t");
}

#[test]
fn prompt_marks_jump_and_select_output() {
    let mut g = Grid::new(20, 3);
    let cmd = |g: &mut Grid, name: &str, out: &str| {
        feed(g, &format!("\x1b]133;A\x07$ \x1b]133;B\x07{name}\r\n\x1b]133;C\x07{out}\x1b]133;D;0\x07"));
    };
    cmd(&mut g, "one", "1a\r\n1b\r\n");
    cmd(&mut g, "two", "2a\r\n2b\r\n2c\r\n");
    feed(&mut g, "\x1b]133;A\x07$ ");
    assert_eq!(g.prompts.len(), 3);

    assert_eq!(g.select_last_output().as_deref(), Some("2a\n2b\n2c"));
    assert!(g.display_offset > 0, "scrolled up to show the output start");

    g.display_offset = 0;
    assert!(g.jump_to_prompt(-1));
    assert_eq!(line(&g, 0), "$ two");
    assert!(g.jump_to_prompt(-1));
    assert_eq!(line(&g, 0), "$ one");
    assert!(!g.jump_to_prompt(-1), "no older prompt");
    assert!(g.jump_to_prompt(1));
    assert_eq!(line(&g, 0), "$ two");
    assert!(g.jump_to_prompt(1));
    assert!(g.jump_to_prompt(1), "past the last prompt goes live");
    assert_eq!(g.display_offset, 0);
}

#[test]
fn prompt_rows_above_cursor_counts_the_current_multiline_prompt() {
    let mut g = Grid::new(30, 6);
    feed(&mut g, "\x1b]133;A\x07~\r\n$ \x1b]133;B\x07");
    assert_eq!(g.prompt_rows_above_cursor(), 1);

    let mut single = Grid::new(30, 6);
    feed(&mut single, "\x1b]133;A\x07$ \x1b]133;B\x07");
    assert_eq!(single.prompt_rows_above_cursor(), 0);

    // A prompt that already produced output is not the current one.
    let mut done = Grid::new(30, 6);
    feed(&mut done, "\x1b]133;A\x07~\r\n$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07out\r\n");
    assert_eq!(done.prompt_rows_above_cursor(), 0);
}

#[test]
fn prompt_marks_survive_reflow() {
    let mut g = Grid::new(20, 3);
    let cmd = |g: &mut Grid, name: &str, out: &str| {
        feed(g, &format!("\x1b]133;A\x07$ \x1b]133;B\x07{name}\r\n\x1b]133;C\x07{out}\x1b]133;D;0\x07"));
    };
    cmd(&mut g, "one", "a long output line!!\r\n");
    cmd(&mut g, "two", "2a\r\n2b\r\n");
    feed(&mut g, "\x1b]133;A\x07$ ");

    // Narrower: the long line wraps, so every later line moves down.
    g.resize(10, 3);
    assert_eq!(g.prompts.len(), 3);
    assert_eq!(g.select_last_output().as_deref(), Some("2a\n2b"));
    g.display_offset = 0;
    assert!(g.jump_to_prompt(-1));
    assert_eq!(line(&g, 0), "$ two");
    assert!(g.jump_to_prompt(-1));
    assert_eq!(line(&g, 0), "$ one");

    // Wider again: back to one row per line.
    g.resize(30, 3);
    assert_eq!(g.select_last_output().as_deref(), Some("2a\n2b"));
    g.display_offset = 0;
    assert!(g.jump_to_prompt(-1));
    assert_eq!(line(&g, 0), "$ two");
    assert!(g.jump_to_prompt(-1));
    assert_eq!(line(&g, 0), "$ one");
}

#[test]
fn mouse_and_focus_modes() {
    let mut g = Grid::new(8, 2);
    assert_eq!(g.modes.mouse_tracking, MouseTracking::Off);
    feed(&mut g, "\x1b[?1002h\x1b[?1006h\x1b[?1004h");
    assert_eq!(g.modes.mouse_tracking, MouseTracking::Drag);
    assert!(g.modes.mouse_sgr && g.modes.focus_events);
    feed(&mut g, "\x1b[?1002$p");
    assert_eq!(g.response, b"\x1b[?1002;1$y");
    feed(&mut g, "\x1b[?1002l\x1bc");
    assert_eq!(g.modes.mouse_tracking, MouseTracking::Off);
    assert!(!g.modes.mouse_sgr && !g.modes.focus_events);
}

#[test]
fn tab_stops_set_clear_and_move() {
    let mut g = Grid::new(30, 1);
    feed(&mut g, "\tA");
    assert_eq!(g.display_row(0)[8].codepoint, 'A' as u32);
    feed(&mut g, "\r\x1b[3g\x1b[5G\x1bH\r\tB");
    assert_eq!(g.display_row(0)[4].codepoint, 'B' as u32);
    feed(&mut g, "\x1b[Z");
    assert_eq!(g.cur.col, 4);
    feed(&mut g, "\x1b[3g\r\x1b[2I");
    assert_eq!(g.cur.col, 29);
}

#[test]
fn insert_mode_shifts_text_right() {
    let mut g = Grid::new(8, 1);
    feed(&mut g, "abc\r\x1b[4hX\x1b[4lY");
    assert_eq!(line(&g, 0), "XYbc");
    let mut g = Grid::new(8, 1);
    feed(&mut g, "abc\r\x1b[4hXY");
    assert_eq!(line(&g, 0), "XYabc");
}

#[test]
fn origin_mode_positions_relative_to_scroll_region() {
    let mut g = Grid::new(10, 10);
    feed(&mut g, "\x1b[3;6r\x1b[?6h");
    assert_eq!(g.cur.row, 2);
    feed(&mut g, "\x1b[2;1H");
    assert_eq!(g.cur.row, 3);
    feed(&mut g, "\x1b[99;1H");
    assert_eq!(g.cur.row, 5);
    feed(&mut g, "\x1b[6n");
    assert_eq!(g.response, b"\x1b[4;1R");
    feed(&mut g, "\x1b[?6l");
    assert_eq!(g.cur.row, 0);
}

#[test]
fn osc_7_52_and_133() {
    let mut g = Grid::new(20, 2);
    feed(&mut g, "\x1b]7;file://host/Users/me/My%20Dir\x07");
    assert_eq!(g.reported_cwd.as_deref(), Some("/Users/me/My Dir"));
    assert!(g.title_changed);

    feed(&mut g, "\x1b]52;c;aGVsbG8gVmnhu4d0\x07");
    assert_eq!(g.clipboard_request.as_deref(), Some("hello Việt"));
    g.clipboard_request = None;
    feed(&mut g, "\x1b]52;c;?\x07");
    assert!(g.clipboard_request.is_none());

    // Prompt "my-prompt>>" with the input start marked by OSC 133;B.
    feed(&mut g, "\x1b]133;A\x07my-prompt>>\x1b]133;B\x07ls -la");
    assert_eq!(g.current_prompt_input(), "ls -la");
    assert_eq!(g.prompt_command_range().0, 11);
}

#[test]
fn is_wide_cp_matches_char_width_for_every_codepoint() {
    for cp in 0..=0x10FFFF {
        let expected = char::from_u32(cp).is_some_and(|c| char_width(c) == 2);
        assert_eq!(is_wide_cp(cp), expected, "U+{cp:04X}");
    }
}

#[test]
fn overwriting_half_of_a_wide_char_clears_the_other_half() {
    let mut g = Grid::new(6, 1);
    feed(&mut g, "中\rx");
    assert_eq!(g.display_row(0)[1].codepoint, 0);
    assert_eq!(line(&g, 0), "x");
    let mut g = Grid::new(6, 1);
    feed(&mut g, "中\x1b[2Gy");
    assert_eq!(g.display_row(0)[0].codepoint, 0);
    assert_eq!(line(&g, 0), " y");
}

#[test]
fn printable_ascii_keeps_insert_wrap_and_line_drawing() {
    let mut g = Grid::new(5, 2);
    feed(&mut g, "abcd\r\x1b[4hX\x1b[4l");
    assert_eq!(line(&g, 0), "Xabcd");
    // The last column defers the wrap to the next character.
    feed(&mut g, "\x1b[1;5Hyz");
    assert_eq!((line(&g, 0), line(&g, 1)), ("Xabcy".to_string(), "z".to_string()));
    // DEC line drawing maps letters before the ASCII fast path.
    feed(&mut g, "\x1b(0q\x1b(Bq");
    assert_eq!(line(&g, 1), "z─q");
}

#[test]
fn only_continuations_join_the_previous_grapheme() {
    let mut g = Grid::new(10, 1);
    g.modes.grapheme_mode = true;
    // After a ZWJ a non-pictographic character starts its own cell, and
    // the joiner does not carry over to the emoji after it.
    feed(&mut g, "👨\u{200D}中👩");
    let row = g.display_row(0);
    assert_eq!(g.cluster(row[0].codepoint), Some("👨\u{200D}"));
    assert_eq!((row[2].codepoint, row[4].codepoint), ('中' as u32, '👩' as u32));
}

#[test]
fn emoji_clusters_copy_back_exactly() {
    let s = "😀 👍🏽 👨‍👩‍👧 🇻🇳 ❤️ #️⃣ 🏳️‍🌈";
    let mut g = Grid::new(40, 3);
    g.modes.grapheme_mode = true;
    feed(&mut g, &format!("\x1b]133;A\x07$ \x1b]133;B\x07echo '{s}'\r\n\x1b]133;C\x07{s}\r\n"));
    g.set_selection(Some(Selection { start: (1, 0), end: (1, 39) }));
    assert_eq!(g.selected_text().as_deref(), Some(s));
}

#[test]
fn selection_follows_text_when_output_scrolls() {
    let mut g = Grid::new(10, 3);
    feed(&mut g, "one\r\ntwo\r\nthree");
    g.set_selection(Some(Selection { start: (1, 0), end: (1, 2) }));
    assert_eq!(g.selected_text().as_deref(), Some("two"));
    feed(&mut g, "\r\nfour");
    // "two" moved up to row 0, and the selection moved with it.
    assert!(g.is_selected(0, 0) && !g.is_selected(1, 0));
    assert_eq!(g.selected_text().as_deref(), Some("two"));
    g.scroll_display(1);
    assert!(g.is_selected(1, 0));
    assert_eq!(g.selected_text().as_deref(), Some("two"));
}

#[test]
fn selection_joins_soft_wrapped_lines() {
    let mut g = Grid::new(4, 3);
    feed(&mut g, "abcdefg\r\nxy");
    g.set_selection(Some(Selection { start: (0, 0), end: (2, 3) }));
    assert_eq!(g.selected_text().as_deref(), Some("abcdefg\nxy"));
}

#[test]
fn search_focuses_nearest_match_and_scrolls_to_it() {
    let mut g = Grid::new(10, 3);
    for i in 0..10 {
        feed(&mut g, &format!("line{i}\r\n"));
    }
    let re = regex::Regex::new(r"line[13]").unwrap();
    let matches = g.search_text().find(&re);
    assert_eq!(matches.len(), 2);
    g.set_search_matches(matches);
    // Bottom-most match is focused first and scrolled into view.
    assert_eq!(g.search_status(), (Some(1), 2));
    assert!((0..g.rows).any(|r| line(&g, r) == "line3"));
    let r = (0..g.rows).find(|&r| line(&g, r) == "line3").unwrap();
    assert_eq!(g.search_hits(r).at(0), SearchHit::Current);
    assert_eq!(g.search_hits(r).at(5), SearchHit::None);

    g.search_step(true);
    assert_eq!(g.search_status(), (Some(0), 2));
    let r = (0..g.rows).find(|&r| line(&g, r) == "line1").unwrap();
    assert_eq!(g.search_hits(r).at(4), SearchHit::Current);
    g.search_step(true);
    assert_eq!(g.search_status(), (Some(1), 2), "wraps around");

    g.clear_search();
    assert_eq!(g.search_status(), (None, 0));
}

#[test]
fn resize_reflows_wrapped_lines() {
    let mut g = Grid::new(10, 4);
    feed(&mut g, "0123456789abcde\r\n$ ");
    assert_eq!(line(&g, 0), "0123456789");
    assert_eq!(line(&g, 1), "abcde");

    g.resize(20, 4);
    assert_eq!(line(&g, 0), "0123456789abcde");
    assert_eq!(line(&g, 1), "$");
    assert_eq!((g.cur.row, g.cur.col), (1, 2));

    g.resize(5, 4);
    assert_eq!(line(&g, 0), "01234");
    assert_eq!(line(&g, 1), "56789");
    assert_eq!(line(&g, 2), "abcde");
    assert_eq!(line(&g, 3), "$");
    assert_eq!((g.cur.row, g.cur.col), (3, 2));

    g.resize(4, 2);
    // Older rows go to scrollback; the cursor line stays visible.
    assert_eq!(line(&g, 1), "$");
    assert_eq!(g.cur.row, 1);
    g.scroll_display(g.history as isize);
    assert_eq!(line(&g, 0), "0123");
}

#[test]
fn resize_reflow_keeps_wide_chars_whole() {
    let mut g = Grid::new(6, 2);
    feed(&mut g, "ab中文");
    g.resize(3, 3);
    assert_eq!(line(&g, 0), "ab");
    assert_eq!(g.display_row(1)[0].codepoint, '中' as u32);
    assert_eq!(g.display_row(2)[0].codepoint, '文' as u32);
}

#[test]
fn alt_screen_restores() {
    let mut g = Grid::new(4, 2);
    feed(&mut g, "main\x1b[?1049hALT");
    assert_eq!(line(&g, 0), "ALT");
    feed(&mut g, "\x1b[?1049l");
    assert_eq!(line(&g, 0), "main");
}

#[test]
fn scroll_region() {
    let mut g = Grid::new(3, 4);
    feed(&mut g, "1\r\n2\r\n3\r\n4\x1b[2;3r\x1b[3;1H\n");
    assert_eq!(line(&g, 0), "1");
    assert_eq!(line(&g, 1), "3");
    assert_eq!(line(&g, 2), "");
    assert_eq!(line(&g, 3), "4");
}

#[test]
fn resize_keeps_cursor_line() {
    let mut g = Grid::new(4, 4);
    feed(&mut g, "a\r\nb\r\nc\r\nd");
    g.resize(4, 2);
    assert_eq!(line(&g, 0), "c");
    assert_eq!(line(&g, 1), "d");
    g.resize(4, 4);
    assert_eq!(line(&g, 0), "a");
    assert_eq!(line(&g, 3), "d");
}

#[test]
fn dsr_reply() {
    let mut g = Grid::new(10, 5);
    feed(&mut g, "\x1b[3;4H\x1b[6n");
    assert_eq!(g.response, b"\x1b[3;4R");
}

#[test]
fn selection_and_line_copy() {
    let mut g = Grid::new(80, 5);
    feed(
        &mut g,
        " ~/Desktop/personal/litty-terminal \u{e0b0} \u{e0a0} main \u{00b1} \u{e0b0}\r\n\
         $ cd Desktop/personal/litty-terminal\r\n\
         $ ./target/release/litty",
    );
    // Double-click on prompt line selects only ~/Desktop/personal/litty-terminal
    assert_eq!(
        g.select_smart_line(0, 10),
        "~/Desktop/personal/litty-terminal"
    );
    assert_eq!(
        g.select_smart_line(0, 0),
        "~/Desktop/personal/litty-terminal"
    );
    // Double-click on command line selects full line minus "$ "
    assert_eq!(
        g.select_smart_line(1, 15),
        "cd Desktop/personal/litty-terminal"
    );
    assert_eq!(
        g.select_smart_line(1, 2),
        "cd Desktop/personal/litty-terminal"
    );
    // Double-click on ./target/release/litty line selects ./target/release/litty
    assert_eq!(g.select_smart_line(2, 10), "./target/release/litty");
    // Triple-click selects raw full line
    assert_eq!(g.select_line(1), "$ cd Desktop/personal/litty-terminal");
}

#[test]
fn double_click_selects_block_then_line() {
    let mut g = Grid::new(40, 3);
    feed(&mut g, "$ git log --oneline 中文x\r\n");
    assert_eq!(g.select_block(0, 8), "log");
    assert!(g.is_block_selected(0, 6));
    assert!(!g.is_block_selected(0, 2), "a different block");
    assert_eq!(g.select_block(0, 12), "--oneline");
    // A wide glyph and its placeholder cell stay in one block.
    assert_eq!(g.select_block(0, 21), "中文x");
    // A blank cell falls back to the smart line.
    assert_eq!(g.select_block(0, 5), "git log --oneline 中文x");
    assert!(!g.is_block_selected(0, 8));
    g.set_selection(None);
    assert!(!g.is_block_selected(0, 8));
}

#[test]
fn url_detection() {
    let mut g = Grid::new(80, 5);
    feed(
        &mut g,
        "Docs at https://ghostty.org/docs and repo (http://github.com/ghostty-org).\r\n\
         ~/Desktop/personal/litty-terminal \u{e0b0} main\r\n\
         $ cd ./src",
    );
    let link1 = g.find_hyperlink_at(0, 15, None).expect("find https url");
    assert_eq!(link1.url, "https://ghostty.org/docs");
    assert_eq!(link1.start_col, 8);
    assert_eq!(link1.end_col, 31);
    assert!(!link1.is_path);

    let link2 = g.find_hyperlink_at(0, 50, None).expect("find http url");
    assert_eq!(link2.url, "http://github.com/ghostty-org");
    assert!(!link2.is_path);

    // Path detection on prompt
    let path1 = g.find_hyperlink_at(1, 10, None).expect("find path");
    assert_eq!(path1.url, "~/Desktop/personal/litty-terminal");
    assert_eq!(path1.start_col, 0);
    assert_eq!(path1.end_col, 32);
    assert!(path1.is_path);

    // Relative path detection
    let path2 = g.find_hyperlink_at(2, 6, None).expect("find relative path");
    assert_eq!(path2.url, "./src");
    assert!(path2.is_path);

    assert!(g.find_hyperlink_at(0, 2, None).is_none());
}

#[test]
fn dynamic_ring_buffer_growth_and_scrolling() {
    let mut g = Grid::new(20, 4);
    assert!(g.cap <= 64, "initial cap should be lean, was {}", g.cap);

    for i in 0..150 {
        feed(&mut g, &format!("line {:03}\r\n", i));
    }
    assert_eq!(g.history, 147);
    assert!(g.cap >= g.history + g.rows);

    // Scroll back to line 0 and verify contents
    g.scroll_display(g.history as isize);
    assert_eq!(line(&g, 0), "line 000");

    // Reset, resize, and scroll back to line 0 again
    g.reset_display();
    g.resize(30, 8);
    g.scroll_display(g.history as isize);
    assert_eq!(line(&g, 0), "line 000");
}

#[test]
fn cursor_movement_on_click() {
    let mut g = Grid::new(80, 5);
    feed(&mut g, "$ cargo build");
    // Prompt is "$ " (cols 0..1), command is "cargo build" (cols 2..12), cursor is at col 13
    assert_eq!(g.cur.row, 0);
    assert_eq!(g.cur.col, 13);
    assert_eq!(g.prompt_command_range(), (2, 13));

    // Click at col 4 (letter 'r' in cargo): moves left by 9 columns
    let left_bytes = g.calculate_cursor_movement_to(0, 4).expect("move left");
    assert_eq!(left_bytes, b"\x1b[D".repeat(9));

    // Click before prompt at col 0: clamped to col 2 (start of command), moves left by 11
    let clamp_left = g.calculate_cursor_movement_to(0, 0).expect("clamp to start");
    assert_eq!(clamp_left, b"\x1b[D".repeat(11));

    // Click at col 50: clamped to col 13 (end of command) -> already at col 13 -> None
    assert!(g.calculate_cursor_movement_to(0, 50).is_none());

    // Now move cursor to col 2 (start of cargo)
    g.cur.col = 2;
    // Click at col 8 (start of build): moves right by 6
    let right_bytes = g.calculate_cursor_movement_to(0, 8).expect("move right");
    assert_eq!(right_bytes, b"\x1b[C".repeat(6));

    // Click on different row returns None
    assert!(g.calculate_cursor_movement_to(1, 8).is_none());

    // Alt screen active returns None
    g.alt_active = true;
    assert!(g.calculate_cursor_movement_to(0, 8).is_none());
    g.alt_active = false;

    // Scrolled into history returns None
    g.display_offset = 1;
    assert!(g.calculate_cursor_movement_to(0, 8).is_none());
    g.display_offset = 0;

    // Wide character test
    let mut g_emoji = Grid::new(80, 5);
    feed(&mut g_emoji, "❯ 🦀 test");
    // "❯ " (cols 0..1), 🦀 (cols 2..3, width 2), " test" (cols 4..8), cursor at col 9
    assert_eq!(g_emoji.cur.col, 9);
    // Click at col 2 on 🦀: 5 characters between col 2 and 9 (test=4, space=1, 🦀=1, dummy cell skipped)
    let emoji_click = g_emoji.calculate_cursor_movement_to(0, 2).expect("emoji move");
    assert_eq!(emoji_click, b"\x1b[D".repeat(6));

    // Click at dummy cell 3 on 🦀: snaps to col 2 and also moves 6 chars left
    let dummy_click = g_emoji.calculate_cursor_movement_to(0, 3).expect("dummy snap");
    assert_eq!(dummy_click, b"\x1b[D".repeat(6));
}

#[test]
fn delete_before_and_after_cursor() {
    let mut g = Grid::new(80, 5);
    feed(&mut g, "$ cargo build");
    // Prompt ends at 2, cursor is at 13
    assert_eq!(g.cur.col, 13);

    // At end of line: delete before cursor deletes 11 characters ("cargo build")
    assert_eq!(g.calculate_delete_before_cursor(), Some(vec![0x7f; 11]));
    // At end of line: nothing to delete after cursor
    assert_eq!(g.calculate_delete_after_cursor(), None);

    // Move cursor to middle (col 7, after "cargo ")
    g.cur.col = 7;
    // Deletes 5 characters ("cargo ") before cursor
    assert_eq!(g.calculate_delete_before_cursor(), Some(vec![0x7f; 5]));
    // Deletes text after cursor ("build") via Ctrl+K
    assert_eq!(g.calculate_delete_after_cursor(), Some(vec![0x0b]));

    // Move cursor to start of command (col 2)
    g.cur.col = 2;
    // Nothing before cursor (prompt is protected)
    assert_eq!(g.calculate_delete_before_cursor(), None);
    // Deletes text after cursor via Ctrl+K
    assert_eq!(g.calculate_delete_after_cursor(), Some(vec![0x0b]));

    // In alt screen mode
    g.alt_active = true;
    assert_eq!(g.calculate_delete_before_cursor(), Some(vec![0x15])); // Ctrl+U
    assert_eq!(g.calculate_delete_after_cursor(), Some(vec![0x0b]));  // Ctrl+K
}

#[test]
fn clear_screen_and_history_clears_clusters() {
    let mut g = Grid::new(20, 5);
    feed(&mut g, "❤️\r\n👨‍👩‍👧‍👦\r\n");
    assert!(!g.clusters.is_empty(), "clusters recorded");
    assert!(!g.cluster_ids.is_empty());

    g.clear_screen_and_history();
    assert!(g.clusters.is_empty(), "clusters cleared on clear_screen_and_history");
    assert!(g.cluster_ids.is_empty());

    feed(&mut g, "🏳️‍🌈\r\n");
    assert!(!g.clusters.is_empty());

    g.full_reset();
    assert!(g.clusters.is_empty(), "clusters cleared on full_reset");
    assert!(g.cluster_ids.is_empty());
}

#[test]
fn clear_screen_and_history_trims_extras() {
    let mut g = Grid::new(20, 5);
    feed(&mut g, "\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\\r\n");
    assert!(g.extras.len() > 1, "extra slot allocated for hyperlink");

    g.clear_screen_and_history();
    assert_eq!(g.extras.len(), 1, "extras trimmed to [None] on clear_screen_and_history");

    feed(&mut g, "\x1b]8;;https://another.org\x1b\\link2\x1b]8;;\x1b\\\r\n");
    assert!(g.extras.len() > 1);

    g.full_reset();
    assert_eq!(g.extras.len(), 1, "extras trimmed on full_reset");
}

#[test]
fn clear_screen_and_history_prunes_images() {
    let mut g = Grid::new(20, 5);
    let id = g.graphics.insert(0, 10, 10, vec![0; 400]);
    g.graphics.place(crate::graphics::Placement {
        image_id: id,
        placement_id: 1,
        line: 0,
        col: 0,
        cols: 2,
        rows: 2,
        src: [0, 0, 10, 10],
        z: 0,
        alt: false,
        fit: false,
    });
    assert!(g.graphics.image(id).is_some());
    assert_eq!(g.graphics.placements.len(), 1);

    g.clear_screen_and_history();
    assert!(g.graphics.placements.is_empty(), "placements cleared");
    assert!(g.graphics.image(id).is_none(), "auto-assigned image freed");
}

#[test]
fn clear_screen_and_history_prunes_virtual_placements() {
    let mut g = Grid::new(20, 5);
    let id = g.graphics.insert(0, 10, 10, vec![0; 400]);
    g.graphics.virtuals.push(crate::graphics::VirtualPlacement {
        image_id: id,
        placement_id: 1,
        cols: 2,
        rows: 2,
        z: 0,
        alt: false,
    });
    assert_eq!(g.graphics.virtuals.len(), 1);
    assert!(g.graphics.image(id).is_some());

    g.clear_screen_and_history();
    assert!(g.graphics.virtuals.is_empty(), "virtuals cleared");
    assert!(g.graphics.image(id).is_none(), "auto-assigned image freed after virtual cleared");
}

#[test]
fn alt_screen_exit_prunes_virtual_placements_and_frees_auto_ids() {
    let mut g = Grid::new(20, 5);
    let main_id = g.graphics.insert(0, 10, 10, vec![0; 400]);
    g.graphics.virtuals.push(crate::graphics::VirtualPlacement {
        image_id: main_id,
        placement_id: 1,
        cols: 2,
        rows: 2,
        z: 0,
        alt: false,
    });

    // Switch to alternate screen
    g.set_alt(true, false);
    let alt_id = g.graphics.insert(0, 10, 10, vec![0; 400]);
    g.place_virtual(alt_id, 3, 3, 0, 2);
    assert_eq!(g.graphics.virtuals.len(), 2);
    assert!(g.graphics.image(main_id).is_some());
    assert!(g.graphics.image(alt_id).is_some());

    // Exit alternate screen
    g.set_alt(false, false);
    assert_eq!(g.graphics.virtuals.len(), 1, "alt virtual placement pruned");
    assert_eq!(g.graphics.virtuals[0].image_id, main_id, "main virtual placement preserved");
    assert!(g.graphics.image(main_id).is_some(), "main image retained");
    assert!(g.graphics.image(alt_id).is_none(), "alt auto-id image freed");
}

#[test]
fn clear_history_trims_scrollback_extras_and_reclaims_slots() {
    let mut g = Grid::new(20, 3);
    feed(&mut g, "\x1b]8;;http://scrollback.test\x07link\x1b]8;;\x07\n\n\n\n");
    assert!(g.history > 0);
    assert!(g.extras.len() > 1);
    assert!(g.extras[1].is_some());

    // clear_history should discard the scrollback rows and free unreferenced extras
    g.clear_history();
    assert_eq!(g.history, 0);
    assert_eq!(g.extras.len(), 1, "trailing unused extras trimmed on clear_history");
}

#[test]
fn unclosed_apc_stream_cleans_up_on_reset_and_clear() {
    let mut g = Grid::new(20, 3);
    let mut parser = vte::Parser::new();
    // Send start of APC: ESC _
    g.feed_byte(&mut parser, 0x1b);
    g.feed_byte(&mut parser, b'_');
    for &b in b"m=1;a=t;payload" {
        g.feed_byte(&mut parser, b);
    }
    assert_eq!(g.apc_state, 2);
    assert!(!g.apc_buf.is_empty());
    assert!(g.apc_start.is_some());

    g.clear_screen_and_history();
    assert_eq!(g.apc_state, 0);
    assert!(g.apc_buf.is_empty());
    assert!(g.apc_start.is_none());

    // Test full reset clears APC too
    g.feed_byte(&mut parser, 0x1b);
    g.feed_byte(&mut parser, b'_');
    g.feed_byte(&mut parser, b'x');
    assert_eq!(g.apc_state, 2);
    g.full_reset();
    assert_eq!(g.apc_state, 0);
    assert!(g.apc_buf.is_empty());
    assert!(g.apc_start.is_none());
}

#[test]
fn dcs_stream_cleans_up_on_reset_and_clear() {
    let mut g = Grid::new(20, 3);
    feed(&mut g, "\x1bP$q\"p");
    assert!(g.dcs.is_some());
    assert!(!g.dcs_buf.is_empty());

    g.clear_screen_and_history();
    assert!(g.dcs.is_none());
    assert!(g.dcs_buf.is_empty());
    assert!(g.dcs_start.is_none());
}

#[test]
fn scrollback_eviction_prunes_dead_virtual_placements() {
    let mut g = Grid::new(20, 2);
    g.scrollback = 2;
    g.cap = 4;
    g.cells.truncate(4 * 20);
    // Insert auto-id image
    let id = g.graphics.insert(0, 10, 10, vec![0; 400]);
    assert!(id >= (1 << 24));
    g.place_virtual(id, 2, 1, 0, 0);
    let d = |i: usize| char::from_u32(crate::graphics::PLACEHOLDER_DIACRITICS[i]).unwrap();
    let msb = (id >> 24) as usize;
    let r = ((id >> 16) & 0xFF) as u8;
    let g_col = ((id >> 8) & 0xFF) as u8;
    let b = (id & 0xFF) as u8;
    feed(&mut g, &format!("\x1b[38;2;{r};{g_col};{b}m\u{10EEEE}{}{}{}\x1b[0m", d(0), d(0), d(msb)));
    assert_eq!(g.graphics.virtuals.len(), 1);
    assert!(g.graphics.image(id).is_some());

    // Scroll lines:
    // row 0: placeholder
    // \n: row 1
    // \n: scroll 1 (history = 1)
    // \n: scroll 2 (history = 2, scrollback is now full)
    feed(&mut g, "\n\n\n");
    assert_eq!(g.history, 2);
    assert_eq!(g.graphics.virtuals.len(), 1);
    assert!(g.graphics.image(id).is_some());

    // Scroll 1 more line: scrollback limit is 2, so the oldest row (holding placeholder) is evicted!
    feed(&mut g, "\n");
    assert_eq!(g.graphics.virtuals.len(), 0, "dead virtual placement pruned after row evicted");
    assert!(g.graphics.image(id).is_none(), "auto-id image freed when virtual placement dies");
}

#[test]
fn clear_history_prunes_scrollback_only_virtual_placements() {
    let mut g = Grid::new(20, 2);
    let id = g.graphics.insert(0, 10, 10, vec![0; 400]);
    g.place_virtual(id, 2, 1, 0, 0);
    let d = |i: usize| char::from_u32(crate::graphics::PLACEHOLDER_DIACRITICS[i]).unwrap();
    let msb = (id >> 24) as usize;
    let r = ((id >> 16) & 0xFF) as u8;
    let g_col = ((id >> 8) & 0xFF) as u8;
    let b = (id & 0xFF) as u8;
    feed(&mut g, &format!("\x1b[38;2;{r};{g_col};{b}m\u{10EEEE}{}{}{}\x1b[0m\r\n\r\n\r\n", d(0), d(0), d(msb)));
    assert!(g.history > 0);
    assert_eq!(g.graphics.virtuals.len(), 1);
    assert!(g.graphics.image(id).is_some());

    g.clear_history();
    assert_eq!(g.graphics.virtuals.len(), 0, "virtual placement pruned on clear_history");
    assert!(g.graphics.image(id).is_none(), "auto-id image freed on clear_history");
}

#[test]
fn alt_screen_linefeed_scroll_prunes_stale_placements() {
    let mut g = Grid::new(20, 4);
    g.set_alt(true, false);
    assert!(g.alt_active);

    // Insert auto-id image and place it on row 0 covering 2 rows
    let id = g.graphics.insert(0, 20, 40, vec![0; 20 * 40 * 4]);
    g.place_image(id, [0, 0, 20, 40], 2, 2, 0, 1, true, false);
    assert_eq!(g.graphics.placements.len(), 1);
    assert_eq!(g.graphics.placements[0].rows, 2);
    assert_eq!(g.graphics.placements[0].line, g.lines_pushed);

    // Scroll up by 1 line on the alternate screen (rows 0..=3)
    g.scroll_up_in(0, 3, 1, true);
    assert_eq!(g.graphics.placements.len(), 1);
    assert_eq!(g.graphics.placements[0].rows, 1, "top row clipped");
    assert_eq!(g.graphics.placements[0].line, g.lines_pushed);
    assert!(g.graphics.image(id).is_some());

    // Scroll up by 1 more line: placement completely scrolled off top of alt screen
    g.scroll_up_in(0, 3, 1, true);
    assert!(g.graphics.placements.is_empty(), "placement pruned after scrolling off alt screen");
    assert!(g.graphics.image(id).is_none(), "auto-id image freed after placement pruned");
}

#[test]
fn test_bidi_order_no_false_trigger_on_clusters_and_placeholders() {
    let mut g = Grid::new(30, 4);
    let mut chars = Vec::new();
    let mut levels = Vec::new();
    let mut order = Vec::new();

    // 1. Plain ASCII with grapheme cluster (e + combining acute accent)
    feed(&mut g, "hello e\u{301} world\r\n");
    let has_bidi = g.bidi_order(0, &mut chars, &mut levels, &mut order);
    assert!(!has_bidi, "ASCII with grapheme cluster must not trigger BiDi order");

    // 2. Wide emoji cluster (heart with VS16)
    feed(&mut g, "emoji \u{2764}\u{fe0f} test\r\n");
    let has_bidi = g.bidi_order(1, &mut chars, &mut levels, &mut order);
    assert!(!has_bidi, "emoji cluster must not trigger BiDi order");

    // 3. Image placeholder cell
    let id = g.graphics.insert(0, 10, 10, vec![0; 400]);
    g.place_virtual(id, 2, 1, 0, 0);
    let d = |i: usize| char::from_u32(crate::graphics::PLACEHOLDER_DIACRITICS[i]).unwrap();
    let msb = (id >> 24) as usize;
    let r = ((id >> 16) & 0xFF) as u8;
    let g_col = ((id >> 8) & 0xFF) as u8;
    let b = (id & 0xFF) as u8;
    feed(&mut g, &format!("\x1b[38;2;{r};{g_col};{b}m\u{10EEEE}{}{}{}\x1b[0m\r\n", d(0), d(0), d(msb)));
    let has_bidi = g.bidi_order(2, &mut chars, &mut levels, &mut order);
    assert!(!has_bidi, "image placeholder must not trigger BiDi order");

    // 4. Real RTL Hebrew text MUST trigger BiDi
    feed(&mut g, "english \u{05E9}\u{05DC}\u{05D5}\u{05DD} hebrew");
    let has_bidi = g.bidi_order(3, &mut chars, &mut levels, &mut order);
    assert!(has_bidi, "Hebrew text MUST trigger BiDi order");
    assert!(!order.is_empty(), "order map must be generated for BiDi row");
}

#[test]
fn test_saturated_extras_performance_and_reclamation() {
    let mut g = Grid::new(80, 24);
    let t0 = std::time::Instant::now();
    // Feed 1000 distinct hyperlinks into the grid (exceeds 255 capacity)
    for i in 0..1000 {
        feed(&mut g, &format!("\x1b]8;;https://example.com/item/{i}\x1b\\item_{i}\x1b]8;;\x1b\\\r\n"));
    }
    let dur = t0.elapsed();
    assert!(dur < std::time::Duration::from_millis(500), "1000 hyperlinks must not freeze (took {:?})", dur);
    assert!(g.extras.len() <= 256, "extras table must not exceed 256 entries");

    // 2. Test reclamation with scrollback eviction
    let mut g2 = Grid::new(40, 2);
    g2.scrollback = 2;
    g2.cap = 4;
    for i in 0..500 {
        feed(&mut g2, &format!("\x1b]8;;https://reclaim.test/{i}\x1b\\link_{i}\x1b]8;;\x1b\\\r\n"));
    }
    assert!(g2.extras.len() <= 256);
    let link = g2.find_hyperlink_at(0, 0, None).expect("link on row 0 should be found");
    assert!(link.url.starts_with("https://reclaim.test/"));
}

#[test]
fn test_kitty_apc_and_sixel_stream_ingestion() {
    let mut g = Grid::new(80, 24);
    let mut parser = vte::Parser::new();

    // 1. Kitty APC stream: 128KB chunk
    let mut apc_data = Vec::with_capacity(128 * 1024 + 16);
    apc_data.extend_from_slice(b"\x1b_Gf=32,s=100,v=100,m=1;");
    apc_data.resize(128 * 1024, b'A');
    apc_data.extend_from_slice(b"\x1b\\");

    let t0 = std::time::Instant::now();
    for &b in &apc_data {
        g.feed_byte(&mut parser, b);
    }
    let dur_apc = t0.elapsed();
    assert!(dur_apc < std::time::Duration::from_millis(100), "128KB Kitty APC took too long: {:?}", dur_apc);

    // 2. Sixel DCS stream: 128KB chunk
    let mut sixel_data = Vec::with_capacity(128 * 1024 + 16);
    sixel_data.extend_from_slice(b"\x1bPq\"1;1;100;100#0;2;0;0;0");
    sixel_data.resize(128 * 1024, b'~');
    sixel_data.extend_from_slice(b"\x1b\\");

    let t1 = std::time::Instant::now();
    for &b in &sixel_data {
        g.feed_byte(&mut parser, b);
    }
    let dur_sixel = t1.elapsed();
    assert!(dur_sixel < std::time::Duration::from_millis(100), "128KB Sixel took too long: {:?}", dur_sixel);
}

#[test]
fn test_saturated_extras_reclamation_on_alt_screen_and_edits() {
    let mut g = Grid::new(80, 24);
    // Fill 255 extras
    for i in 1..=255 {
        feed(&mut g, &format!("\x1b]8;;https://example.com/item/{i}\x1b\\x\x1b]8;;\x1b\\"));
    }
    assert_eq!(g.extras.len(), 256);

    // Overwrite cell (0, 0) with a wide character replacing the extra
    feed(&mut g, "\x1b[1;1H\u{4E16}\x1b[0m");
    assert!(g.extras_dirty, "extras_dirty must be marked on cell overwrite");

    // Now feed a brand new URL; it should reclaim the freed slot
    feed(&mut g, "\x1b[1;10H\x1b]8;;https://example.com/new_item\x1b\\NEW\x1b]8;;\x1b\\");
    assert!(g.extras.len() <= 256);
    let link = g.find_hyperlink_at(0, 10, None).expect("new link must be interned into reclaimed slot");
    assert_eq!(&*link.url, "https://example.com/new_item");

    // Test alt-screen exit reclamation
    feed(&mut g, "\x1b[?1049h"); // enter alt
    assert!(g.alt_active);
    feed(&mut g, "\x1b[?1049l"); // exit alt
    assert!(!g.alt_active);

    // Test character deletion (DCH) and insertion (ICH)
    feed(&mut g, "\x1b[1;10H\x1b[1P"); // delete character with link
    assert!(g.extras_dirty, "extras_dirty must be set after DCH");
}

#[test]
fn test_osc104_color_swap_aliasing_and_batched_reset() {
    let mut g = Grid::new(80, 24);
    let p1 = palette(1) & !0xFF;
    let p2 = palette(2) & !0xFF;

    // Override palette index 1 with palette(2)'s RGB and index 2 with palette(1)'s RGB (color swap!)
    feed(
        &mut g,
        &format!(
            "\x1b]4;1;rgb:{:02x}/{:02x}/{:02x}\x1b\\\x1b]4;2;rgb:{:02x}/{:02x}/{:02x}\x1b\\",
            (p2 >> 24) & 0xFF,
            (p2 >> 16) & 0xFF,
            (p2 >> 8) & 0xFF,
            (p1 >> 24) & 0xFF,
            (p1 >> 16) & 0xFF,
            (p1 >> 8) & 0xFF
        ),
    );

    // Write cell (0,0) with index 1 (currently p2) and cell (0,1) with index 2 (currently p1)
    feed(&mut g, "\x1b[31mA\x1b[32mB\x1b[0m");
    assert_eq!(g.display_row(0)[0].fg & !0xFF, p2);
    assert_eq!(g.display_row(0)[1].fg & !0xFF, p1);

    // Now send OSC 104 (full palette reset).
    // Cell (0,0) must revert to p1 and cell (0,1) must revert to p2 without aliasing both to p2!
    feed(&mut g, "\x1b]104\x1b\\");
    assert_eq!(
        g.display_row(0)[0].fg & !0xFF,
        p1,
        "cell(0,0) with swapped palette 1 must reset to p1, not alias to p2"
    );
    assert_eq!(
        g.display_row(0)[1].fg & !0xFF,
        p2,
        "cell(0,1) with swapped palette 2 must reset to p2"
    );
}

fn populated_cluster_history() -> Grid {
    let mut g = Grid::new(80, 50);
    // Keep this benchmark independent of process-global config tests.
    g.scrollback = 5000;
    let mut parser = vte::Parser::new();
    for i in 0..5200 {
        let base = char::from_u32(0x4e00 + i % 5000).unwrap();
        for b in format!("{base}\u{301}\r\n").bytes() { g.feed_byte(&mut parser, b); }
    }
    assert_eq!(g.history, 5000);
    g.gc_clusters();
    assert!(g.clusters.len() >= 4096);
    g
}

#[test]
fn cluster_eviction_gc_is_amortized_and_preserves_live_text() {
    let mut g = populated_cluster_history();
    let start_count = g.clusters.len();
    let start_evictions = g.cluster_evictions;
    let allocations = crate::test_alloc::allocations(|| {
        for _ in 0..200 { g.scroll_up_in(0, g.rows - 1, 1, true); }
    });
    assert_eq!(allocations, 0, "scrolling must not allocate a mark vector per row");
    assert_eq!(g.clusters.len(), start_count, "no collection during a small eviction batch");
    assert_eq!(g.cluster_evictions, start_evictions + 200);
    // Finish a full eviction interval. GC must reclaim dead clusters and
    // remap the surviving cells, even after wrapping the ring.
    for _ in 0..4800 { g.scroll_up_in(0, g.rows - 1, 1, true); }
    assert!(g.clusters.len() < start_count);
    assert_eq!(g.cluster_evictions, 0);
    for i in 0..50 {
        let row = (g.top + g.cap - g.history + i) % g.cap;
        let cp = g.cells[row * g.cols].codepoint;
        let base = char::from_u32((0x4e00 + (5151 + i) % 5000) as u32).unwrap();
        if i < 49 { assert_eq!(g.cluster(cp), Some(format!("{base}\u{301}").as_str())); }
    }
    let marks = g.cluster_marks.as_ptr();
    g.gc_clusters();
    assert_eq!(g.cluster_marks.as_ptr(), marks, "mark storage is reused");
}

#[test]
#[ignore = "manual performance measurement"]
fn benchmark_cluster_scrollback_eviction() {
    let mut g = populated_cluster_history();
    let start = std::time::Instant::now();
    for _ in 0..200 { g.scroll_up_in(0, g.rows - 1, 1, true); }
    let amortized = start.elapsed();
    let mut legacy = populated_cluster_history();
    let start = std::time::Instant::now();
    for _ in 0..200 {
        legacy.scroll_up_in(0, legacy.rows - 1, 1, true);
        legacy.gc_clusters();
    }
    eprintln!("Cluster history, 200 evictions: amortized {amortized:?}; full GC per row {:?}", start.elapsed());
}

#[test]
fn test_grapheme_cluster_gc_on_clear_history_and_eviction() {
    let mut g = Grid::new(40, 4);
    // Feed multi-mark clusters into history rows (base + 2 combining marks won't NFC-precompose into 1 char)
    for i in 0..20 {
        let mark1 = char::from_u32(0x0300 + (i % 16)).unwrap();
        let mark2 = char::from_u32(0x0310 + (i % 16)).unwrap();
        feed(&mut g, &format!("a{mark1}{mark2}_{i}\r\n"));
    }
    assert!(g.clusters.len() >= 10, "clusters should be populated (got {})", g.clusters.len());

    // Write a distinct active cluster on the current screen row
    feed(&mut g, "z\u{0308}\u{0304}");
    let active_row = g.cur.row;
    let cp_before = g.display_row(active_row)[0].codepoint;
    assert_ne!(cp_before & CLUSTER_FLAG, 0, "active cell must be a cluster");
    assert_eq!(g.cluster(cp_before), Some("z\u{0308}\u{0304}"));

    // Clear history -> triggers gc_clusters()
    g.clear_history();
    assert!(
        g.clusters.len() <= 4,
        "unreferenced history clusters must be garbage-collected (remaining: {})",
        g.clusters.len()
    );
    let cp_after = g.display_row(active_row)[0].codepoint;
    assert_ne!(cp_after & CLUSTER_FLAG, 0);
    assert_eq!(
        g.cluster(cp_after),
        Some("z\u{0308}\u{0304}"),
        "active cluster ID must be remapped and preserve its text after GC"
    );

    // Verify new clusters can still be interned and looked up accurately
    feed(&mut g, "q\u{0301}\u{0327}");
    let cp_new = g.display_row(active_row)[1].codepoint;
    assert_eq!(g.cluster(cp_new), Some("q\u{0301}\u{0327}"));
}

#[test]
fn test_hyperlink_candidate_lock_free_resolution() {
    let mut g = Grid::new(80, 24);
    feed(&mut g, "Check ./Cargo.toml and https://example.com/docs here\r\n");

    // Candidate for implicit path ("./Cargo.toml") extracted without filesystem I/O
    let cand_path = g
        .find_hyperlink_candidate_at(0, 8)
        .expect("should extract implicit path candidate");
    let resolved_path = cand_path
        .resolve(Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))))
        .expect("./Cargo.toml should resolve against CARGO_MANIFEST_DIR");
    assert!(resolved_path.is_path);
    assert_eq!(&*resolved_path.url, "./Cargo.toml");

    // Candidate for explicit URL ("https://example.com/docs")
    let cand_url = g
        .find_hyperlink_candidate_at(0, 28)
        .expect("should extract URL candidate");
    let resolved_url = cand_url.resolve(None).expect("URL should resolve without CWD");
    assert_eq!(&*resolved_url.url, "https://example.com/docs");
}
