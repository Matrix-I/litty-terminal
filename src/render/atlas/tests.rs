#[cfg(target_os = "macos")]
use super::coretext::{rasterize_color, rasterize_text_cluster};
use super::fonts::parse_font;
use super::run_cache;
use super::shaper::{GlyphPiece, Shaper};
use super::sprites::draw_builtin_sprite;
use super::*;

#[test]
fn test_cell_size_pt_values() {
    let fonts = load_font(&["SFMono Nerd Font".to_string()]);
    let (cw, ch) = Atlas::cell_size_pt(&fonts, 15.0);
    println!("cell_size_pt for 15.0: cw = {cw}, ch = {ch}");
    println!("For 143x41: inner_w = {}, inner_h = {}", 143.0 * cw + 8.0, 41.0 * ch + 8.0);
    assert!(cw > 0.0 && ch > 0.0);
}

#[test]
fn blit_fit_scales_premultiplies_and_centers() {
    // 4x2 opaque red over half-transparent blue, into an 8x8 canvas: 2x scale
    // would overflow the 0.95 margin, so it is ~1.9x and vertically centered.
    let mut src = Vec::new();
    for i in 0..8 {
        src.extend_from_slice(if i % 4 < 2 { &[255, 0, 0, 255] } else { &[0, 0, 255, 128] });
    }
    let mut dst = vec![0u8; 8 * 8 * 4];
    blit_fit(&src, 4, 2, &mut dst, 8, 8);
    let px = |x: usize, y: usize| &dst[(y * 8 + x) * 4..][..4];
    assert_eq!(px(0, 0), [0, 0, 0, 0], "transparent margin above");
    assert_eq!(px(0, 3), [255, 0, 0, 255]);
    assert_eq!(px(7, 3), [0, 0, 128, 128], "premultiplied");
}

#[cfg(target_os = "macos")]
#[test]
fn color_emoji_rasterizes_in_color() {
    let (w, h) = (36, 34);
    let mut buf = vec![0u8; (w * h * 4) as usize];
    assert!(rasterize_color(&mut buf, w, h, 28.0, "😀"), "😀 from Apple Color Emoji");
    assert!(buf.as_chunks::<4>().0.iter().any(|p| p[3] > 200 && (p[0] as i32 - p[2] as i32).abs() > 60), "colored, not gray");
    buf.fill(0);
    assert!(rasterize_color(&mut buf, w, h, 28.0, "🇻🇳"), "a flag");
}

#[cfg(target_os = "macos")]
#[test]
fn text_clusters_render_through_coretext() {
    let (w, h) = (18, 34);
    let mut buf = vec![0u8; (w * h) as usize];
    assert!(rasterize_text_cluster(&mut buf, w, h, 26, 28.0, "\u{0915}\u{093F}"), "Devanagari ki");
    let ink_rows = (0..h as usize).filter(|y| buf[y * w as usize..][..w as usize].iter().any(|&v| v > 0)).count();
    assert!(ink_rows > 10, "a real glyph, not a dot");
}

/// Needs a font with programming ligatures, e.g.
/// `LITTY_LIGATURE_FONT=JetBrainsMono-Regular.ttf cargo test ligatures`.
#[test]
fn ligatures_shape_into_glyph_pieces() {
    let Ok(path) = std::env::var("LITTY_LIGATURE_FONT") else { return };
    let _features = FEATURES_LOCK.lock();
    set_font_features(&[]);
    let fonts = FontSet { primary: parse_font(&path, 0, true).unwrap(), fallbacks: Vec::new() };
    let row: Vec<crate::terminal::grid::Cell> =
        "ab -> cd != ef".chars().map(|c| crate::terminal::grid::Cell { codepoint: c as u32, ..Default::default() }).collect();
    let mut out = Vec::new();
    let mut on = Shaper::new(Vec::new());
    assert!(on.shape_row(&fonts, 28.0, 17, &row, &mut out).0);
    assert_eq!(&out[..2], [0, 0], "plain words are not shaped");
    assert!(out[3] != 0 && out[4] != 0, "`->` is substituted: {out:x?}");
    assert!(out[9] != 0 && out[10] != 0, "`!=` is substituted: {out:x?}");
    assert_eq!(out[5], 0);
    // JetBrains Mono draws `->` as one glyph over both cells, pen in the second.
    assert!(on.pieces.iter().all(|p| p.span == 2), "{:?}", on.pieces);

    let off = ["-calt", "-liga"].map(|f| parse_font_feature(f).unwrap()).to_vec();
    set_font_features(&off);
    let fonts_off = FontSet { primary: parse_font(&path, 0, true).unwrap(), fallbacks: Vec::new() };
    assert!(!Shaper::new(off).shape_row(&fonts_off, 28.0, 17, &row, &mut out).0, "font-feature = -calt disables them");
    set_font_features(&[]);
}

/// `load_font` holds `FONT_CACHE` while `parse_font` reads `FONT_FEATURES`;
/// `set_font_features` must not hold `FONT_FEATURES` while it waits for `FONT_CACHE`.
#[test]
fn set_font_features_does_not_deadlock_with_load_font() {
    let _features = FEATURES_LOCK.lock();
    let cache = super::fonts::FONT_CACHE.lock();
    let setter = std::thread::spawn(|| set_font_features(&[parse_font_feature("-calt").unwrap()]));
    // Give the setter the chance to take FONT_FEATURES and block on FONT_CACHE.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while !shaper::FONT_FEATURES.is_locked() && !setter.is_finished() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    // What `parse_font` does inside `load_font`.
    let read = shaper::FONT_FEATURES.try_lock_for(std::time::Duration::from_secs(2)).map(|f| f.clone());
    drop(cache);
    setter.join().unwrap();
    set_font_features(&[]);
    assert!(read.is_some(), "set_font_features held FONT_FEATURES while waiting for FONT_CACHE");
}

#[test]
fn font_feature_parser_handles_standard_syntax() {
    assert_eq!(parse_font_feature("calt"), Ok(FontFeature { tag: *b"calt", value: 1 }));
    assert_eq!(parse_font_feature("+calt"), Ok(FontFeature { tag: *b"calt", value: 1 }));
    assert_eq!(parse_font_feature("-calt"), Ok(FontFeature { tag: *b"calt", value: 0 }));
    assert_eq!(parse_font_feature("ss01"), Ok(FontFeature { tag: *b"ss01", value: 1 }));
    assert_eq!(parse_font_feature("liga=0"), Ok(FontFeature { tag: *b"liga", value: 0 }));
    assert_eq!(parse_font_feature("cv02=3"), Ok(FontFeature { tag: *b"cv02", value: 3 }));
    assert_eq!(parse_font_feature("kern=off"), Ok(FontFeature { tag: *b"kern", value: 0 }));
    assert_eq!(parse_font_feature("kern=on"), Ok(FontFeature { tag: *b"kern", value: 1 }));
    assert!(parse_font_feature("").is_err());
    assert!(parse_font_feature("toolong=1").is_err());
    assert!(parse_font_feature("-calt=1").is_err());
    assert!(parse_font_feature("calt=abc").is_err());
}

#[cfg(target_os = "macos")]
#[test]
fn primary_font_rasterizes_ascii_and_indexed_glyphs() {
    let fonts = load_font(&[]);
    let (cw, ch) = (16, 32);
    let mut buf = vec![0u8; (cw * ch) as usize];
    fonts.primary.rasterize_char(&mut buf, cw, ch, 24, 28.0, 'M', false);
    assert!(buf.iter().any(|&v| v > 0), "primary font must rasterize 'M'");
    let gid = fonts.primary.lookup_glyph_index('A');
    assert!(gid > 0);
    let (_, _, bw, bh) = fonts.primary.glyph_bounds(gid, 28.0);
    assert!(bw > 0 && bh > 0);
    buf.fill(0);
    fonts.primary.draw_indexed_glyph(&mut buf, cw as usize, ch as usize, 0.0, 24, 28.0, gid);
    assert!(buf.iter().any(|&v| v > 0), "indexed glyph must draw");
    buf.fill(0);
    assert!(fonts.primary.rasterize_cluster_primary(&mut buf, cw as u32, ch as u32, 24, 28.0, "e\u{0301}"));
}

#[cfg(target_os = "macos")]
const MENLO: &str = "/System/Library/Fonts/Menlo.ttc";

/// `ch`'s glyph id, looked up on a throwaway font so the font under test
/// starts with no cached sizes.
#[cfg(target_os = "macos")]
fn menlo_gid(ch: char) -> u16 {
    parse_font(MENLO, 0, true).unwrap().lookup_glyph_index(ch)
}

/// Shaping asks for nominal glyph ids between sized calls at the atlas size;
/// a lookup at another size would replace the one cached font every time.
#[cfg(target_os = "macos")]
#[test]
fn glyph_lookup_keeps_the_cached_font_size() {
    let gid = menlo_gid('A');
    // Its own font: `load_font` shares one across tests that use other sizes.
    let font = parse_font(MENLO, 0, true).unwrap();
    assert!(font.glyph_bounds(gid, 28.0).2 > 0);
    assert!(font.lookup_glyph_index('>') > 0);
    assert_eq!(font.cached_sizes(), [28.0]);
}

/// Windows on a Retina and a 1x display share one font at two sizes.
#[cfg(target_os = "macos")]
#[test]
fn sized_fonts_keep_several_sizes() {
    let gid = menlo_gid('A');
    let font = parse_font(MENLO, 0, true).unwrap();
    for px in [28.0, 14.0, 28.0, 14.0] {
        assert!(font.glyph_bounds(gid, px).2 > 0);
    }
    assert_eq!(font.cached_sizes(), [14.0, 28.0]);
    for px in [10.0, 11.0, 12.0, 13.0, 15.0] {
        let _ = font.glyph_bounds(gid, px);
    }
    assert_eq!(font.cached_sizes().len(), 4, "bounded");
}

#[test]
fn metric_adjust_parses_pixels_and_percent() {
    assert_eq!(MetricAdjust::parse("2"), Ok(MetricAdjust::Px(2)));
    assert_eq!(MetricAdjust::parse("-1"), Ok(MetricAdjust::Px(-1)));
    assert_eq!(MetricAdjust::parse("10%"), Ok(MetricAdjust::Percent(10.0)));
    assert!(MetricAdjust::parse("big").is_err());
    assert_eq!(MetricAdjust::Px(2).apply(17), 19);
    assert_eq!(MetricAdjust::Percent(20.0).apply(20), 24);
    assert_eq!(MetricAdjust::Px(-50).apply(17), 1, "never below one pixel");
}

#[test]
fn powerline_and_sprites_render_non_empty() {
    let mut buf = vec![0u8; 16 * 32];
    // U+E0B0 right arrow, U+E0A0 git branch, U+2500 box horizontal, U+2588 full block, U+28FF braille
    for cp in [0xE0B0, 0xE0A0, 0xE0A2, 0xE0B2, 0xE0B4, 0xE0B6, 0x2500, 0x256D, 0x2588, 0x28FF] {
        buf.fill(0);
        assert!(draw_builtin_sprite(&mut buf, 16, 32, cp), "sprite {cp:#X} not handled");
        assert!(buf.iter().any(|&b| b > 0), "sprite {cp:#X} produced blank bitmap");
    }
}

#[test]
fn powerline_right_arrow_touches_left_edge_seamlessly() {
    let (cw, ch) = (16u32, 32u32);
    let mut buf = vec![0u8; (cw * ch) as usize];
    assert!(draw_builtin_sprite(&mut buf, cw, ch, 0xE0B0));
    // Middle rows at x=0 must be 100% solid (255) so there is zero gap with the previous cell.
    for y in 2..(ch - 2) {
        assert_eq!(buf[(y * cw) as usize], 255, "left edge gap at row {y}");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn macos_coretext_fallback_renders_missing_menlo_glyphs() {
    let mut buf = vec![0u8; 16 * 32];
    // U+279C (➜) or CJK '中' is handled via CoreText fallback on macOS
    assert!(macos_coretext_rasterize(&mut buf, 16, 32, 24, 28.0, '中'));
    assert!(buf.iter().any(|&b| b > 0));
}

#[cfg(target_os = "macos")]
#[test]
fn loads_sfmono_nerd_font_successfully() {
    let set = load_font(&["SFMono Nerd Font".to_string()]);
    assert!(set.primary.lookup_glyph_index('A') > 0);
}

#[test]
fn test_dynamic_atlas_growth_and_shaper_cap() {
    let _gpu = crate::render::backend::TEST_GPU.lock();
    let (device, queue) = crate::render::backend::shared_device_and_queue();

    let fonts = load_font(&[]);
    let mut atlas = Atlas::new(&device, &queue, fonts, 28.0);
    let initial_size = atlas.size;
    let initial_slots = atlas.n_slots;
    assert_eq!(initial_size, 512);

    // Verify wide_slots allocates two distinct slots (left and right) in one pass
    atlas.begin_frame();
    let (sl, sr) = atlas.glyph_slots(&queue, '中' as u32, 2, GlyphStyle::default());
    assert_ne!(sl, BLANK_SLOT);
    assert_ne!(sr, BLANK_SLOT);
    assert_ne!(sl, sr);
    // Second lookup in same frame hits cache and returns identical slots
    assert_eq!(atlas.glyph_slots(&queue, '中' as u32, 2, GlyphStyle::default()), (sl, sr));

    // Exhaust all slots in the same frame so alloc_slot evicts an active slot and sets grow_needed
    for i in 0..initial_slots + 16 {
        let cp = 0x4E00 + i;
        let _ = atlas.slot(&queue, cp);
    }
    assert!(atlas.take_evicted(), "same-frame overflow must mark atlas evicted");
    assert!(atlas.grow_if_needed(&device, &queue), "atlas must grow from 512 to 1024");
    assert_eq!(atlas.size, 1024);
    assert!(atlas.n_slots > initial_slots);

    // Verify Shaper pieces cap at 4096 triggers reset_pieces and evicts GLYPH_PIECE keys
    for i in 0..4096u16 {
        let piece = GlyphPiece { gid: i, span: 2, origin: 0, bold: false, italic: false };
        atlas.shaper.piece_ids.insert(piece, i as u32);
        atlas.shaper.pieces.push(piece);
    }
    let dummy_key = GLYPH_PIECE | (5 << 5) | 1;
    let piece_slot = atlas.alloc_slot(dummy_key);
    assert_eq!(atlas.map.get(&dummy_key).copied(), Some(piece_slot));

    let mut out = Vec::new();
    let row = vec![crate::terminal::grid::Cell { codepoint: b'a' as u32, ..Default::default() }];
    atlas.shape_row(&row, &mut out);
    assert!(atlas.shaper.pieces.is_empty(), "shaper pieces must reset at 4096 cap");
    assert!(!atlas.map.contains_key(&dummy_key), "GLYPH_PIECE slot keys must be evicted on shaper reset");
    assert!(atlas.take_evicted(), "shaper reset must flag atlas eviction");
}

#[test]
fn synthesized_line_metrics_give_a_sane_cell_height() {
    let lm = synthesized_line_metrics(20.0);
    assert!(lm.ascent > 0.0 && lm.descent < 0.0);
    assert_eq!(lm.line_gap, 0.0);
    assert_eq!((lm.ascent - lm.descent + lm.line_gap).ceil() as u32, 20);
    assert_eq!(lm.new_line_size, lm.ascent - lm.descent);
}

#[test]
fn line_metrics_prefers_the_font_when_it_has_them() {
    let fonts = load_font(&[]);
    let real = fonts.primary.horizontal_line_metrics(16.0).expect("system font has hhea");
    assert_eq!(line_metrics(&fonts.primary, 16.0), real);
}

/// Tests that change or depend on the global `font-feature` list take this.
static FEATURES_LOCK: parking_lot::Mutex<()> = parking_lot::const_mutex(());

fn cells(text: &str, bold: bool) -> Vec<crate::terminal::grid::Cell> {
    let fg = if bold { crate::terminal::grid::FG_BOLD } else { 0 };
    text.chars().map(|c| crate::terminal::grid::Cell { codepoint: c as u32, fg, ..Default::default() }).collect()
}

/// `out` with piece ids replaced by the pieces they name, so shapers that
/// interned pieces in a different order still compare equal.
fn resolved(shaper: &Shaper, out: &[u32]) -> Vec<(u32, Option<GlyphPiece>)> {
    out.iter()
        .map(|&v| match v & GLYPH_PIECE != 0 {
            true => (v & 31, Some(shaper.pieces[((v & !GLYPH_PIECE) >> 5) as usize])),
            false => (v, None),
        })
        .collect()
}

fn shaping_fonts() -> Vec<Arc<FontSet>> {
    let mut sets = vec![load_font(&[])];
    if let Ok(path) = std::env::var("LITTY_LIGATURE_FONT") {
        sets.push(Arc::new(FontSet { primary: parse_font(&path, 0, true).unwrap(), fallbacks: Vec::new() }));
    }
    sets
}

const CODE_ROWS: [&str; 5] = [
    "fn main() -> Result<(), Box<dyn Error>> { let x = a != b && c <= d; }",
    "    if (x === y || z !== w) { return a >>= 2; } // => ok",
    "ab -> cd != ef",
    "plain words only here",
    "<!-- html --> |> :: ... ++ -- ** www ;;",
];

#[test]
fn cached_shaping_matches_fresh_shaping() {
    // HarfBuzz reads the global features while shaping; keep them fixed.
    let _features = FEATURES_LOCK.lock();
    for fonts in shaping_fonts() {
        let mut cached = Shaper::new(Vec::new());
        let (mut a, mut b) = (Vec::new(), Vec::new());
        // Twice over, so the second pass is served from the cache.
        for pass in 0..2 {
            for (i, text) in CODE_ROWS.iter().enumerate() {
                let row = cells(text, i % 2 == 1);
                let hit = cached.shape_row(&fonts, 28.0, 17, &row, &mut a).0;
                let mut fresh = Shaper::new(Vec::new());
                let miss = fresh.shape_row(&fonts, 28.0, 17, &row, &mut b).0;
                assert_eq!(hit, miss, "pass {pass}, row {text:?}");
                assert_eq!(resolved(&cached, &a), resolved(&fresh, &b), "pass {pass}, row {text:?}");
            }
        }
    }
}

#[test]
fn repeated_runs_are_shaped_once() {
    let fonts = load_font(&[]);
    let mut shaper = Shaper::new(Vec::new());
    let mut out = Vec::new();
    let row = cells(CODE_ROWS[0], false);
    shaper.shape_row(&fonts, 28.0, 17, &row, &mut out);
    let shaped = shaper.platform_shapes;
    assert!(shaped > 0, "the row has punctuation runs to shape");
    shaper.shape_row(&fonts, 28.0, 17, &row, &mut out);
    assert_eq!(shaper.platform_shapes, shaped, "the same runs come from the cache");
    // The same text in another style, size or cell width is a different entry.
    shaper.shape_row(&fonts, 28.0, 17, &cells(CODE_ROWS[0], true), &mut out);
    assert!(shaper.platform_shapes > shaped, "bold runs are cached separately");
    let shaped = shaper.platform_shapes;
    shaper.shape_row(&fonts, 30.0, 18, &row, &mut out);
    assert!(shaper.platform_shapes > shaped, "a new size shapes again");
}

#[test]
fn cached_shaping_does_not_allocate() {
    let fonts = load_font(&[]);
    let mut shaper = Shaper::new(Vec::new());
    let mut out = Vec::new();
    let rows: Vec<_> = CODE_ROWS.iter().map(|t| cells(t, false)).collect();
    for row in &rows {
        shaper.shape_row(&fonts, 28.0, 17, row, &mut out);
    }
    let allocations = crate::test_alloc::allocations(|| {
        for row in &rows {
            shaper.shape_row(&fonts, 28.0, 17, row, &mut out);
        }
    });
    assert_eq!(allocations, 0);
}

#[test]
fn shaping_cache_stays_bounded_and_survives_a_piece_reset() {
    let fonts = load_font(&[]);
    let mut shaper = Shaper::new(Vec::new());
    let mut out = Vec::new();
    for i in 0..2 * run_cache::RUN_CACHE_ENTRIES + 10 {
        shaper.shape_row(&fonts, 28.0, 17, &cells(&format!("a->{i}"), false), &mut out);
        assert!(shaper.cached_runs() <= 2 * run_cache::RUN_CACHE_ENTRIES);
    }
    // Interned piece ids restart after a reset, so cached runs must go too.
    let row = cells(CODE_ROWS[0], false);
    shaper.shape_row(&fonts, 28.0, 17, &row, &mut out);
    for gid in 0..4096u16 {
        shaper.pieces.push(GlyphPiece { gid, span: 2, origin: 0, bold: false, italic: false });
    }
    let shaped = shaper.platform_shapes;
    assert!(shaper.shape_row(&fonts, 28.0, 17, &row, &mut out).1, "pieces reset");
    assert!(shaper.platform_shapes > shaped, "the reset cleared the cache");
}

/// The color texture's side, from the texture itself where the backend exposes it.
fn color_side(atlas: &Atlas) -> u32 {
    #[cfg(target_os = "macos")]
    assert_eq!(atlas.color_texture.width, atlas.color_size, "texture matches the recorded size");
    atlas.color_size
}

fn test_atlas() -> (Atlas, std::sync::Arc<crate::render::backend::Queue>) {
    let (device, queue) = crate::render::backend::shared_device_and_queue();
    (Atlas::new(&device, &queue, load_font(&[]), 16.0), queue)
}

#[test]
fn color_texture_is_a_placeholder_until_a_color_glyph_is_uploaded() {
    let _gpu = crate::render::backend::TEST_GPU.lock();
    let (mut atlas, queue) = test_atlas();
    for cp in ['a', '─', 'é'] {
        atlas.slot(&queue, cp as u32);
    }
    assert_eq!(color_side(&atlas), 1, "no color glyph yet");
    atlas.upload_color(&queue, 1);
    assert_eq!(color_side(&atlas), atlas.size, "full size once a color glyph is stored");
}

#[test]
fn growing_the_atlas_drops_the_color_texture() {
    let _gpu = crate::render::backend::TEST_GPU.lock();
    let (mut atlas, queue) = test_atlas();
    let (device, _) = crate::render::backend::shared_device_and_queue();
    atlas.upload_color(&queue, 1);
    atlas.grow_needed = true;
    assert!(atlas.grow_if_needed(&device, &queue));
    assert_eq!(color_side(&atlas), 1, "every slot was cleared, so no color glyph is left");
    atlas.upload_color(&queue, 1);
    assert_eq!(color_side(&atlas), atlas.size);
}

#[cfg(target_os = "macos")]
#[test]
fn an_emoji_creates_the_color_texture() {
    let _gpu = crate::render::backend::TEST_GPU.lock();
    let (mut atlas, queue) = test_atlas();
    let (left, _) = atlas.glyph_slots(&queue, '😀' as u32, 2, GlyphStyle::default());
    assert!(atlas.is_color(left));
    assert_eq!(color_side(&atlas), atlas.size);
}

#[test]
fn slot_key_hasher_spreads_style_and_width_flags() {
    use std::hash::{BuildHasher, BuildHasherDefault};
    let build = BuildHasherDefault::<SlotKeyHasher>::default();
    let flags = [0, STYLE_BOLD, STYLE_ITALIC, STYLE_BOLD | STYLE_ITALIC, WIDE_LEFT, WIDE_RIGHT, CLUSTER_KEY, GLYPH_PIECE];
    // The table indexes with the low bits and tags entries with the top 7.
    let mut buckets = [0u16; 4096];
    let mut tags = [0u16; 128];
    for cp in 0x21u32..0x221 {
        for flag in flags {
            let hash = build.hash_one(cp | flag);
            assert_eq!(hash, build.hash_one(cp | flag), "deterministic");
            buckets[(hash & 4095) as usize] += 1;
            tags[(hash >> 57) as usize] += 1;
        }
    }
    assert_eq!(buckets.iter().map(|&n| n as usize).sum::<usize>(), 4096);
    assert!(*buckets.iter().max().unwrap() <= 8, "bucket load {:?}", buckets.iter().max());
    assert!(*tags.iter().max().unwrap() <= 48, "tag load {:?}", tags.iter().max());
}
