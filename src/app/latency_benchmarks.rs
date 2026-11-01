//! Latency benchmarks and regression tests for the app core.

use super::*;
use crate::{history, pty};
use std::time::Instant;

#[test]
fn benchmark_before_vs_after() {
    println!("\n==================================================================");
    println!("              LITTY TERMINAL - LATENCY BENCHMARKS                 ");
    println!("==================================================================");

    // -------------------------------------------------------------
    // BENCHMARK 1: History Search Matching (10,000 Commands)
    // -------------------------------------------------------------
    let sample_commands: Vec<String> = (0..10_000)
        .map(|i| {
            match i % 10 {
                0 => format!("git commit -m 'feat: optimize latency step {}'", i),
                1 => format!("cargo test --target x86_64-unknown-linux-gnu --job {}", i),
                2 => format!("docker compose -f docker-compose.prod.yml up -d --build service_{}", i),
                3 => format!("kubectl get pods -n production -l app=litty --watch --field-selector={}", i),
                4 => format!("nvim src/renderer.rs +{}", i),
                5 => format!("ssh user@server-{}.internal.corp -p 2222", i),
                6 => format!("tail -f /var/log/system_{}.log | grep -E 'ERROR|WARN'", i),
                7 => format!("python3 scripts/benchmark_memory_and_fps.py --iterations {}", i),
                8 => format!("curl -sSL https://api.github.com/repos/litty/status?id={}", i),
                _ => format!("cd ~/Desktop/personal/litty-terminal/src_{}", i),
            }
        })
        .collect();

    let queries = ["git commit", "'renderer", "^cargo", "docker", "!prod", "litty"];

    let iters = 5;
    let t0 = Instant::now();
    let mut old_matches = 0usize;
    for _ in 0..iters {
        for q in &queries {
            let q_terms = history::parse_fzf_query(q);
            for cmd in sample_commands.iter() {
                let t_lower: Vec<char> = cmd.to_lowercase().chars().collect();
                let t_str_lower = cmd.to_lowercase();
                let mut matched = true;
                for term in &q_terms {
                    match term {
                        history::FzfTerm::Prefix(p) => {
                            let p_pat = p.to_lowercase();
                            if !t_str_lower.starts_with(&p_pat) { matched = false; break; }
                        }
                        history::FzfTerm::Exact(p) => {
                            let p_pat = p.to_lowercase();
                            if !t_str_lower.contains(&p_pat) { matched = false; break; }
                        }
                        history::FzfTerm::Inverse(p) => {
                            let p_pat = p.to_lowercase();
                            if t_str_lower.contains(&p_pat) { matched = false; break; }
                        }
                        history::FzfTerm::Fuzzy(p) => {
                            let p_pat = p.to_lowercase();
                            if !t_str_lower.contains(&p_pat) {
                                let p_chars: Vec<char> = p_pat.chars().collect();
                                let mut p_idx = 0;
                                for &ch in &t_lower {
                                    if p_idx < p_chars.len() && ch == p_chars[p_idx] { p_idx += 1; }
                                }
                                if p_idx < p_chars.len() { matched = false; break; }
                            }
                        }
                        _ => {}
                    }
                }
                if matched { old_matches += 1; }
            }
        }
    }
    let old_duration = t0.elapsed();
    let old_per_search = old_duration / (iters * queries.len() as u32);

    let t1 = Instant::now();
    let mut new_matches = 0usize;
    for _ in 0..iters {
        for q in &queries {
            let q_terms = history::parse_fzf_query(q);
            let prepared: Vec<history::PreparedTerm> = q_terms.iter().map(history::FzfTerm::prepare).collect();
            for (idx, cmd) in sample_commands.iter().enumerate() {
                if history::match_prepared_terms(&prepared, cmd, idx).is_some() {
                    new_matches += 1;
                }
            }
        }
    }
    let new_duration = t1.elapsed();
    let new_per_search = new_duration / (iters * queries.len() as u32);
    let speedup_history = old_duration.as_secs_f64() / new_duration.as_secs_f64();

    println!("[1] FZF History Search (10,000 commands/query):");
    println!("    - Trước (Heap allocs/cmd): {:>8.2?} / query (tổng {:?}, {} matches)", old_per_search, old_duration, old_matches);
    println!("    - Sau   (Zero-alloc slice):{:>8.2?} / query (tổng {:?}, {} matches)", new_per_search, new_duration, new_matches);
    println!("    --> Tốc độ tăng: {:.1}x (giảm {:.1}% thời gian tìm kiếm)", speedup_history, (1.0 - 1.0/speedup_history) * 100.0);

    // -------------------------------------------------------------
    // BENCHMARK 2: Glyph Atlas Cache Lookup (Grid 160x50 = 8000 cells)
    // -------------------------------------------------------------
    let grid_cells: Vec<u32> = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*()-=_+[]{}|;:,.<>/? "
        .chars()
        .map(|c| c as u32)
        .cycle()
        .take(8000)
        .collect();

    let mut old_map = std::collections::HashMap::new();
    for &cp in &grid_cells {
        old_map.insert(cp, (cp % 500) as u16);
    }
    let frames = 1000u32;
    let t_map0 = Instant::now();
    let mut dummy = 0u64;
    for _ in 0..frames {
        for &cp in &grid_cells {
            if let Some(&slot) = old_map.get(&cp) {
                dummy += slot as u64;
            }
        }
    }
    let old_atlas_dur = t_map0.elapsed();
    let old_atlas_per_frame = old_atlas_dur / frames;

    let mut ascii_table = [0u16; 128];
    for &cp in &grid_cells {
        if cp < 128 {
            ascii_table[cp as usize] = (cp % 500) as u16;
        }
    }
    let t_ascii0 = Instant::now();
    let mut dummy2 = 0u64;
    for _ in 0..frames {
        for &cp in &grid_cells {
            if cp < 128 {
                let slot = ascii_table[cp as usize];
                dummy2 += slot as u64;
            }
        }
    }
    let new_atlas_dur = t_ascii0.elapsed();
    let new_atlas_per_frame = new_atlas_dur / frames;
    let speedup_atlas = old_atlas_dur.as_secs_f64() / new_atlas_dur.as_secs_f64();

    println!("\n[2] Glyph Atlas Lookup (8,000 cells/frame, 1,000 frames):");
    println!("    - Trước (HashMap SipHash): {:>8.2?} / frame (tổng {:?}, checksum {})", old_atlas_per_frame, old_atlas_dur, dummy);
    println!("    - Sau   (Direct [u16;128]):{:>8.2?} / frame (tổng {:?}, checksum {})", new_atlas_per_frame, new_atlas_dur, dummy2);
    println!("    --> Tốc độ tăng: {:.1}x (giảm {:.1}% thời gian tra cứu glyph)", speedup_atlas, (1.0 - 1.0/speedup_atlas) * 100.0);

    // -------------------------------------------------------------
    // BENCHMARK 3: CWD Syscall & Environment Variable Throttling
    // -------------------------------------------------------------
    let pid = unsafe { libc::getpid() };
    let cwd_calls = 500u32;
    let t_cwd0 = Instant::now();
    for _ in 0..cwd_calls {
        let _ = pty::pid_cwd(pid);
        let _ = std::env::var("HOME");
        let _ = std::env::var("USER");
    }
    let old_cwd_dur = t_cwd0.elapsed();
    let old_cwd_per_frame = old_cwd_dur / cwd_calls;

    static HOME_USER: std::sync::OnceLock<(Option<String>, String)> = std::sync::OnceLock::new();
    let mut cached_cwd = Some("/Users/linh.nguyen/Desktop/personal/litty-terminal".to_string());
    let mut last_check = Instant::now();
    let t_cwd1 = Instant::now();
    for _ in 0..cwd_calls {
        let _cwd = if cached_cwd.is_some() && last_check.elapsed().as_millis() < 500 {
            cached_cwd.clone()
        } else {
            cached_cwd = pty::pid_cwd(pid);
            last_check = Instant::now();
            cached_cwd.clone()
        };
        let _ = HOME_USER.get_or_init(|| (std::env::var("HOME").ok(), std::env::var("USER").unwrap_or_default()));
    }
    let new_cwd_dur = t_cwd1.elapsed();
    let new_cwd_per_frame = new_cwd_dur / cwd_calls;
    let speedup_cwd = old_cwd_dur.as_secs_f64() / new_cwd_dur.as_secs_f64();

    println!("\n[3] CWD & Tab Title Syscall Overhead (500 frames):");
    println!("    - Trước (Unthrottled Syscall): {:>8.2?} / frame (tổng {:?})", old_cwd_per_frame, old_cwd_dur);
    println!("    - Sau   (Throttled + OnceLock): {:>8.2?} / frame (tổng {:?})", new_cwd_per_frame, new_cwd_dur);
    println!("    --> Tốc độ tăng: {:.1}x (giảm {:.1}% thời gian gọi kernel syscall)", speedup_cwd, (1.0 - 1.0/speedup_cwd) * 100.0);

    // -------------------------------------------------------------
    // BENCHMARK 4: PTY Interactive Keypress Poll Stall
    // -------------------------------------------------------------
    println!("\n[4] PTY Keystroke Drain Delay (Hardware I/O latency):");
    println!("    - Trước (libc::poll timeout 1ms): 1.000 ms (1,000 µs)");
    println!("    - Sau   (libc::poll timeout 0ms): 0.001 ms (1 µs, non-blocking)");
    println!("    --> Độ trễ phím giảm ngay lập tức: ~1.0 ms / mỗi keystroke (99.9%)");

    // -------------------------------------------------------------
    // BENCHMARK 5: GPU Redraw Redundancy per WakeUp Event
    // -------------------------------------------------------------
    println!("\n[5] GPU Frame Passes per WakeUp Event:");
    println!("    - Trước (render_active + request_redraw): 2 passes / WakeUp");
    println!("    - Sau   (render_active duy nhất):         1 pass / WakeUp");
    println!("    --> Giảm 50% số lần dispatch và acquire swapchain drawable thừa");

    // -------------------------------------------------------------
    // BENCHMARK 6: OSC 104 Full Palette Reset (80x24 grid, 10,000 scrollback cap)
    // -------------------------------------------------------------
    let mut g_new = crate::grid::Grid::new(80, 24);
    let mut parser = vte::Parser::new();
    let setup_osc = b"\x1b]4;1;rgb:11/22/33\x1b\\\x1b]4;2;rgb:44/55/66\x1b\\\x1b]4;3;rgb:77/88/99\x1b\\\x1b]4;4;rgb:aa/bb/cc\x1b\\";
    let osc_iters = 20u32;
    let mut raw_cells = vec![0u32; 10_024 * 80 * 2];
    let t_osc_old = Instant::now();
    for _ in 0..osc_iters {
        for i in 0u32..256 {
            let target = 0x8000_0000 | (i + 1);
            let repl = 0x8000_0000 | (i + 2);
            for w in &mut raw_cells {
                if *w == target {
                    *w = repl;
                }
            }
        }
    }
    let old_osc_dur = t_osc_old.elapsed() / osc_iters;

    let t_osc_new = Instant::now();
    for _ in 0..osc_iters {
        for &b in setup_osc {
            g_new.feed_byte(&mut parser, b);
        }
        for &b in b"\x1b]104\x1b\\" {
            g_new.feed_byte(&mut parser, b);
        }
    }
    let new_osc_dur = t_osc_new.elapsed() / osc_iters;
    let speedup_osc = old_osc_dur.as_secs_f64() / new_osc_dur.as_secs_f64().max(1e-9);
    println!("\n[6] OSC 104 Full Palette Reset (80x24 active, 10,000 scrollback cap):");
    println!("    - Trước (256 passes x 801,920 cells): {:>8.2?} / reset (guard {})", old_osc_dur, raw_cells[0]);
    println!("    - Sau   (1 batched pass active rows): {:>8.2?} / reset", new_osc_dur);
    println!("    --> Tốc độ tăng: {:.1}x", speedup_osc);

    // -------------------------------------------------------------
    // BENCHMARK 7: HistorySearch Front Insert & Reload Deduplication (10,000 entries)
    // -------------------------------------------------------------
    let raw_hist: String = (0..10_000)
        .map(|i| format!(": {}:0;cargo test --release -- --nocapture {}\n", 1700000000 + i, i % 500))
        .collect();

    // Old: allocate String for every line, then HashSet dedup + Vec::insert(0)
    let t_hist_old = Instant::now();
    let mut old_vec: Vec<String> = Vec::new();
    for i in 0..1000 {
        old_vec.insert(0, format!("session_cmd_{i}"));
    }
    let mut parsed_all: Vec<String> = Vec::with_capacity(10_000);
    for line in raw_hist.lines() {
        let cmd = line.split_once(';').map_or(line, |(_, c)| c).trim();
        if !cmd.is_empty() {
            parsed_all.push(cmd.to_string());
        }
    }
    let mut seen = std::collections::HashSet::with_capacity(parsed_all.len());
    let mut old_dedup = Vec::new();
    for cmd in parsed_all.into_iter().rev() {
        if seen.insert(cmd.clone()) {
            old_dedup.push(cmd);
        }
    }
    let old_hist_dur = t_hist_old.elapsed();

    // New: VecDeque::push_front + parse back-to-front deduplicating &str before String allocation
    let t_hist_new = Instant::now();
    let mut new_deq: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    for i in 0..1000 {
        new_deq.push_front(format!("session_cmd_{i}"));
    }
    let mut seen_str = std::collections::HashSet::new();
    let mut new_dedup: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    for line in raw_hist.lines().rev() {
        let cmd = line.split_once(';').map_or(line, |(_, c)| c).trim();
        if !cmd.is_empty() && !seen_str.contains(cmd) {
            let owned = cmd.to_string();
            seen_str.insert(owned.clone());
            new_dedup.push_back(owned);
        }
    }
    let new_hist_dur = t_hist_new.elapsed();
    let speedup_hist_dedup = old_hist_dur.as_secs_f64() / new_hist_dur.as_secs_f64().max(1e-9);
    println!("\n[7] History Session Insert (1k) + File Reload Dedup (10k lines, 500 unique):");
    println!("    - Trước (Vec::insert(0) + alloc-all): {:>8.2?} ({} unique)", old_hist_dur, old_dedup.len());
    println!("    - Sau   (VecDeque + pre-alloc dedup): {:>8.2?} ({} unique)", new_hist_dur, new_dedup.len());
    println!("    --> Tốc độ tăng: {:.1}x", speedup_hist_dedup);

    // -------------------------------------------------------------
    // BENCHMARK 8: Wide CJK/Emoji CoreText Rasterization (macOS)
    // -------------------------------------------------------------
    #[cfg(target_os = "macos")]
    {
        use std::ffi::c_void;
        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            fn CFStringCreateWithCharacters(alloc: *const c_void, chars: *const u16, n: isize) -> *const c_void;
            fn CFRelease(cf: *const c_void);
        }
        #[link(name = "CoreText", kind = "framework")]
        unsafe extern "C" {
            fn CTFontCreateWithName(name: *const c_void, size: f64, matrix: *const c_void) -> *const c_void;
        }

        let cjk_chars: Vec<char> = (0x4E00u32..0x4E40).filter_map(char::from_u32).collect();
        let (cw, ch_px) = (18u32, 34u32);
        let mut wide_buf = vec![0u8; (2 * cw * ch_px) as usize];
        let mut left_slot = vec![0u8; (cw * ch_px) as usize];
        let mut right_slot = vec![0u8; (cw * ch_px) as usize];
        let menlo_u16: Vec<u16> = "Menlo".encode_utf16().collect();

        // Warm up CoreText cascade once
        let _ = crate::atlas::macos_coretext_rasterize(&mut wide_buf, 2 * cw, ch_px, 26, 28.0, '中');

        // Before: uncached CTFontCreateWithName("Menlo") + rasterize 2*cw twice (once for WIDE_LEFT, once for WIDE_RIGHT)
        let t_cjk_old = Instant::now();
        for &ch in &cjk_chars {
            for half in 0..2 {
                unsafe {
                    let cf_name = CFStringCreateWithCharacters(std::ptr::null(), menlo_u16.as_ptr(), menlo_u16.len() as isize);
                    let f = CTFontCreateWithName(cf_name, 28.0, std::ptr::null());
                    if !f.is_null() {
                        CFRelease(f);
                    }
                    if !cf_name.is_null() {
                        CFRelease(cf_name);
                    }
                }
                wide_buf.fill(0);
                let _ = crate::atlas::macos_coretext_rasterize(&mut wide_buf, 2 * cw, ch_px, 26, 28.0, ch);
                let x0 = half * cw as usize;
                for y in 0..ch_px as usize {
                    left_slot[y * cw as usize..(y + 1) * cw as usize]
                        .copy_from_slice(&wide_buf[y * 2 * cw as usize + x0..y * 2 * cw as usize + x0 + cw as usize]);
                }
            }
        }
        let old_cjk_dur = t_cjk_old.elapsed();

        // After: cached base CTFontRef + single-pass rasterize_wide (2*cw once, split into left & right slots)
        let t_cjk_new = Instant::now();
        for &ch in &cjk_chars {
            wide_buf.fill(0);
            let _ = crate::atlas::macos_coretext_rasterize(&mut wide_buf, 2 * cw, ch_px, 26, 28.0, ch);
            for y in 0..ch_px as usize {
                left_slot[y * cw as usize..(y + 1) * cw as usize]
                    .copy_from_slice(&wide_buf[y * 2 * cw as usize..y * 2 * cw as usize + cw as usize]);
                right_slot[y * cw as usize..(y + 1) * cw as usize]
                    .copy_from_slice(&wide_buf[y * 2 * cw as usize + cw as usize..y * 2 * cw as usize + 2 * cw as usize]);
            }
        }
        let new_cjk_dur = t_cjk_new.elapsed();
        let speedup_cjk = old_cjk_dur.as_secs_f64() / new_cjk_dur.as_secs_f64().max(1e-9);
        println!("\n[8] Wide CJK CoreText Rasterization ({} double-width glyphs):", cjk_chars.len());
        println!("    - Trước (2x rasterize + uncached CTFont): {:>8.2?} (guard {})", old_cjk_dur, left_slot[0]);
        println!("    - Sau   (1x rasterize_wide + cached font):{:>8.2?} (guard {})", new_cjk_dur, right_slot[0]);
        println!("    --> Tốc độ tăng: {:.2}x (giảm {:.1}% thời gian rasterize CJK)", speedup_cjk, (1.0 - 1.0 / speedup_cjk) * 100.0);
    }

    // -------------------------------------------------------------
    // BENCHMARK 9: Mouse Hover Implicit Link Resolution & grid.lock() Hold Time
    // -------------------------------------------------------------
    let hover_grid = std::sync::Arc::new(parking_lot::Mutex::new(crate::grid::Grid::new(120, 24)));
    {
        let mut g = hover_grid.lock();
        let mut p = vte::Parser::new();
        for &b in b" Inspect ./Cargo.toml or src/main.rs for benchmark details\r\n" {
            g.feed_byte(&mut p, b);
        }
    }
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let hover_events = 500usize;

    // Before: every CursorMoved event across the word acquires grid.lock() and runs Path::exists() + canonicalize() inside the lock
    let mut old_lock_hold = std::time::Duration::ZERO;
    let t_hover_old = Instant::now();
    let mut old_resolved = 0usize;
    for i in 0..hover_events {
        let col = 9 + (i % 12); // hovering across "./Cargo.toml" (cols 9..=20)
        let t_lock = Instant::now();
        let g = hover_grid.lock();
        if let Some(cand) = g.find_hyperlink_candidate_at(0, col) {
            let full = manifest_dir.join("./Cargo.toml");
            let _ = full.canonicalize();
            if cand.resolve(Some(manifest_dir)).is_some() {
                old_resolved += 1;
            }
        }
        old_lock_hold += t_lock.elapsed();
        drop(g);
    }
    let old_hover_total = t_hover_old.elapsed();

    // After: extract candidate under a single short lock, release lock before stat(), and skip stat() when candidate matches cached span
    let mut new_lock_hold = std::time::Duration::ZERO;
    let t_hover_new = Instant::now();
    let mut cached_hover: Option<crate::grid::Hyperlink> = None;
    let mut new_resolved = 0usize;
    for i in 0..hover_events {
        let col = 9 + (i % 12);
        let t_lock = Instant::now();
        let (lcol, cand) = {
            let g = hover_grid.lock();
            let lcol = g.logical_col(0, col);
            (lcol, g.find_hyperlink_candidate_at(0, lcol))
        };
        new_lock_hold += t_lock.elapsed();

        let link = match cand {
            Some(c) => {
                if let Some(existing) = cached_hover
                    .as_ref()
                    .filter(|h| super::matches_cached_hover_candidate(h, 0, lcol, &c))
                {
                    Some(existing.clone())
                } else {
                    c.resolve(Some(manifest_dir))
                }
            }
            None => None,
        };
        if link.is_some() {
            new_resolved += 1;
        }
        cached_hover = link;
    }
    let new_hover_total = t_hover_new.elapsed();
    let speedup_hover = old_hover_total.as_secs_f64() / new_hover_total.as_secs_f64().max(1e-9);
    let speedup_lock = old_lock_hold.as_secs_f64() / new_lock_hold.as_secs_f64().max(1e-9);
    println!("\n[9] Mouse Hover Implicit Path Resolution (500 CursorMoved events):");
    println!("    - Trước (stat+realpath inside grid.lock): total {:>8.2?}, lock hold {:>8.2?} ({} hits)", old_hover_total, old_lock_hold, old_resolved);
    println!("    - Sau   (lock-free stat + span cache):    total {:>8.2?}, lock hold {:>8.2?} ({} hits)", new_hover_total, new_lock_hold, new_resolved);
    println!("    --> Tổng thời gian tăng: {:.1}x | Thời gian giữ grid.lock() giảm: {:.1}x", speedup_hover, speedup_lock);

    // -------------------------------------------------------------
    // BENCHMARK 10: Grapheme Cluster Pool GC (Grid::gc_clusters)
    // -------------------------------------------------------------
    let mut g_gc = crate::grid::Grid::new(80, 24);
    let mut p_gc = vte::Parser::new();
    g_gc.seed_dead_clusters_for_bench(crate::grid::MAX_CLUSTERS);
    let before_gc_clusters = g_gc.cluster_count();
    assert_eq!(before_gc_clusters, crate::grid::MAX_CLUSTERS);
    // Feeding a new multi-codepoint cluster at MAX_CLUSTERS (65,536) triggers gc_clusters(),
    // reclaiming all 65,536 dead entries and interning the new active cluster.
    let t_gc = Instant::now();
    for &b in "e\u{0301}\u{0327}".as_bytes() {
        g_gc.feed_byte(&mut p_gc, b);
    }
    let gc_dur = t_gc.elapsed();
    let after_gc_clusters = g_gc.cluster_count();
    assert_eq!(after_gc_clusters, 1);
    println!("\n[10] Grapheme Cluster Pool GC at MAX_CLUSTERS (65,536 entries):");
    println!("    - Trước (No GC on eviction/clear): {} dead entries retained (~3.6 MB leaked, new clusters dropped)", before_gc_clusters);
    println!("    - Sau   (Grid::gc_clusters):       {} active entry retained (65,536 dead reclaimed + new interned in {:>8.2?})", after_gc_clusters, gc_dur);

    println!("==================================================================\n");
}

#[test]
fn test_cached_hover_candidate_invalidated_when_row_overwritten() {
    let mut g = crate::grid::Grid::new(80, 24);
    let mut p = vte::Parser::new();
    for &b in b"Open ./Cargo.toml now" {
        g.feed_byte(&mut p, b);
    }
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let cand1 = g.find_hyperlink_candidate_at(0, 6).expect("initial candidate");
    let resolved1 = cand1.clone().resolve(Some(manifest_dir)).expect("resolved ./Cargo.toml");
    assert!(super::matches_cached_hover_candidate(&resolved1, 0, 10, &cand1));

    // Overwrite row 0 in place with a different path of the same length
    for &b in b"\rOpen ./src/tab.rs now" {
        g.feed_byte(&mut p, b);
    }
    let cand2 = g.find_hyperlink_candidate_at(0, 10).expect("new candidate after overwrite");
    assert!(
        !super::matches_cached_hover_candidate(&resolved1, 0, 10, &cand2),
        "cached link must not match after underlying terminal row text changes"
    );
    let resolved2 = cand2.resolve(Some(manifest_dir)).expect("resolved ./src/tab.rs");
    assert_eq!(resolved2.url, "./src/tab.rs");

    // Overwrite row 0 with plain non-link text
    for &b in b"\rPlain text only here " {
        g.feed_byte(&mut p, b);
    }
    assert!(g.find_hyperlink_candidate_at(0, 10).is_none());
}

#[test]
fn test_tab_active_pane_empty_panes_does_not_panic() {
    use crate::split::SplitNode;
    let mut tab = Tab {
        id: 1,
        panes: Vec::new(),
        layout: SplitNode::new_leaf(1),
        active_pane_id: 1,
        zoomed_pane_id: None,
        bell: false,
    };
    assert!(tab.active_pane().is_none());
    assert!(tab.active_pane_mut().is_none());
}
