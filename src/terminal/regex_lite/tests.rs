use super::prefilter::Prefilter;
use super::simd::ByteSet;
use super::*;

#[test]
fn literal_fast_path_and_smart_case() {
    let re_cs = Regex::with_options("foo.bar", false, false).unwrap();
    let m: Vec<_> = re_cs.find_iter("xx foo.bar FOO.BAR fooXbar").map(|m| m.as_str()).collect();
    assert_eq!(m, ["foo.bar"]);

    let re_ci = Regex::with_options("foo.bar", true, false).unwrap();
    let m_ci: Vec<_> = re_ci.find_iter("xx foo.bar FOO.BAR fooXbar").map(|m| m.as_str()).collect();
    assert_eq!(m_ci, ["foo.bar", "FOO.BAR"]);
}

#[test]
fn nfa_quantifiers_classes_and_alternation() {
    let re = Regex::new(r"ERR_\d{2,4}|WARN_[A-Z]+").unwrap();
    let m: Vec<_> = re.find_iter("ok ERR_1 bad ERR_4200 and WARN_DISK!").map(|m| m.as_str()).collect();
    assert_eq!(m, ["ERR_4200", "WARN_DISK"]);
}

#[test]
fn anchors_and_word_boundaries() {
    let re = Regex::new(r"^foo\b.*bar$").unwrap();
    assert!(re.find_at("foo_1 bar", 0).is_none());
    assert_eq!(re.find_at("foo 123 bar", 0).map(|m| m.as_str()), Some("foo 123 bar"));
}

#[test]
fn nfa_scan_allocates_once_not_per_match() {
    let re = Regex::new(r"ERR_\d{4}").unwrap();
    let text = "ERR_0001 ok ".repeat(500);
    let mut count = 0;
    let allocations = crate::test_alloc::allocations(|| count = re.find_iter(&text).count());
    assert_eq!(count, 500);
    // Scratch buffers are built once per scan, never once per match.
    assert!(allocations <= 8, "{allocations} allocations for 500 matches");
}

#[test]
fn find_iter_matches_agree_with_find_at() {
    let re = Regex::new(r"ERR_\d{2,4}|WARN_[A-Z]+").unwrap();
    let text = "ok ERR_1 bad ERR_4200 and WARN_DISK! ERR_77 end";
    let via_iter: Vec<_> = re.find_iter(text).map(|m| m.range()).collect();
    let mut via_at = Vec::new();
    let mut pos = 0;
    while let Some(m) = re.find_at(text, pos) {
        via_at.push(m.range());
        pos = m.end();
    }
    assert_eq!(via_iter, via_at);
    assert_eq!(via_iter.len(), 3);
}

fn all_matches(prog: &Prog, text: &str) -> Vec<(usize, usize)> {
    let mut scratch = Scratch::new(prog);
    let (mut pos, mut out) = (0, Vec::new());
    while let Some((start, end)) = prog.find_at(text, pos, &mut scratch) {
        out.push((start, end));
        pos = if end > start { end } else { end + text[end..].chars().next().map_or(1, char::len_utf8) };
        if pos > text.len() {
            break;
        }
    }
    out
}

/// `prog` as a plain PikeVM: no prefilter, no linear shortcut.
fn unfiltered(prog: &Prog) -> Prog {
    Prog { prefilter: Prefilter::None, linear: false, required: None, ..prog.clone() }
}

/// Matches found by the PikeVM alone, without prefilter or backtracker.
fn pike_matches(prog: &Prog, text: &str) -> Vec<(usize, usize)> {
    let plain = unfiltered(prog);
    let mut scratch = Scratch::new(&plain);
    let (mut pos, mut out) = (0, Vec::new());
    while let Some((start, end)) = plain.find_pike(text, pos, &mut scratch) {
        out.push((start, end));
        pos = if end > start { end } else { end + text[end..].chars().next().map_or(1, char::len_utf8) };
        if pos > text.len() {
            break;
        }
    }
    out
}

fn nfa(pattern: &str) -> Prog {
    match Regex::new(pattern).unwrap().engine {
        Engine::Nfa(prog) => prog,
        Engine::Literal(_) => panic!("{pattern} compiled to a literal"),
    }
}

#[test]
fn linear_fast_path_agrees_with_the_full_nfa() {
    let texts = [
        "ERR_0001 ERR_12 ERR_1234 xERR_9999 ERR_ERR_0042",
        "héllo wörld 日本語 ERR_5678 naïve",
        "foo.bar fooXbar FOO.BAR foo\nbar",
        "",
        "ERR_",
    ];
    for (pattern, ci) in [(r"ERR_\d{4}", false), (r"foo.bar", true), (r"w.rld", false), (r"\w\w\s", false), (r"é.l", false)] {
        let Engine::Nfa(prog) = Regex::with_options(pattern, ci, true).unwrap().engine else { continue };
        assert!(prog.linear, "{pattern} should be linear");
        let mut full = prog.clone();
        full.linear = false;
        for text in texts {
            assert_eq!(all_matches(&prog, text), all_matches(&full, text), "{pattern:?} on {text:?}");
        }
    }
}

#[test]
fn branching_programs_are_not_linear() {
    for pattern in [r"ERR_\d{2,4}", r"a|b", r"^foo", r"a*b", r"x\b"] {
        let Engine::Nfa(prog) = Regex::new(pattern).unwrap().engine else { panic!("{pattern}") };
        assert!(!prog.linear, "{pattern}");
    }
}

fn found(pattern: &str, ci: bool, regex: bool, text: &str) -> Vec<String> {
    let re = Regex::with_options(pattern, ci, regex).unwrap();
    re.find_iter(text).map(|m| m.as_str().to_string()).collect()
}

#[test]
fn alternation_prefers_the_leftmost_first_branch() {
    assert_eq!(found("a|ab", false, true, "ab"), ["a"]);
    assert_eq!(found("ab|a", false, true, "ab"), ["ab"]);
    assert_eq!(found("samwise|sam", false, true, "samwise"), ["samwise"]);
    assert_eq!(found("sam|samwise", false, true, "samwise"), ["sam"]);
}

#[test]
fn lazy_quantifiers_take_the_shortest_match() {
    assert_eq!(found("a+?", false, true, "aaa"), ["a", "a", "a"]);
    assert_eq!(found("a{2,3}?", false, true, "aaaaa"), ["aa", "aa"]);
    assert_eq!(found("<.+?>", false, true, "<a><b>"), ["<a>", "<b>"]);
    assert_eq!(found("<.+>", false, true, "<a><b>"), ["<a><b>"]);
}

#[test]
fn empty_matches_follow_regex_crate_iteration() {
    assert_eq!(found("a*", false, true, "baaa"), ["", "aaa"]);
    assert_eq!(found("x*|b", false, true, "b"), ["", ""]);
}

#[test]
fn word_and_space_classes_are_unicode() {
    assert_eq!(found(r"\w+", false, true, "Tiếng Việt!"), ["Tiếng", "Việt"]);
    assert_eq!(found(r"\W+", false, true, "Tiếng Việt!"), [" ", "!"]);
    assert_eq!(found(r"\bViệt\b", false, true, "Tiếng Việt xViệt"), ["Việt"]);
    assert_eq!(found(r"a\sb", false, true, "a\u{a0}b a\u{3000}b"), ["a\u{a0}b", "a\u{3000}b"]);
    assert_eq!(found(r"[\s\S]", false, true, "aS"), ["a", "S"]);
    assert_eq!(found(r"[^\w]", false, true, "ế-"), ["-"]);
    // A decomposed Vietnamese letter stays one word.
    assert_eq!(found(r"\w+", false, true, "Vie\u{302}\u{323}t"), ["Vie\u{302}\u{323}t"]);
}

#[test]
fn digit_class_stays_ascii() {
    assert_eq!(found(r"\d+", false, true, "٣12"), ["12"]);
}

#[test]
fn case_insensitive_matching_folds_unicode() {
    assert_eq!(found("tiếng", true, false, "TIẾNG Tiếng tiếng"), ["TIẾNG", "Tiếng", "tiếng"]);
    assert_eq!(found("tiếng", true, true, "TIẾNG Tiếng"), ["TIẾNG", "Tiếng"]);
    assert_eq!(found("(?i)việt", false, true, "VIỆT"), ["VIỆT"]);
    assert_eq!(found("(?i)[à-ỹ]+", false, true, "ẾẠ"), ["ẾẠ"]);
    assert_eq!(found("(?i)[a-c]x", false, true, "BX"), ["BX"]);
    assert_eq!(found("ếch", true, false, "con ẾCH"), ["ẾCH"]);
}

#[test]
fn case_insensitive_literal_scan_handles_every_offset() {
    let needle = "error_code";
    for len in needle.len()..80usize {
        for at in 0..=len - needle.len() {
            let mut text: String = "x".repeat(len);
            text.replace_range(at..at + needle.len(), "ErRoR_CoDe");
            assert_eq!(found(needle, true, false, &text), ["ErRoR_CoDe"], "len={len} at={at}");
        }
    }
    // Bytes that only equal the needle after `| 0x20` must not match.
    assert!(found("a_", true, false, "\x01\x7f A\x7f").is_empty());
}

#[test]
fn inner_literal_candidates_skip_non_char_boundaries() {
    assert_eq!(found(r"\d{4}-\d{2}", false, true, "é234-56 2026-10-29"), ["2026-10"]);
    // `.` is one char, not one byte, so it cannot anchor a fixed byte offset.
    assert_eq!(found(r"..-\d", false, true, "é-1 xé-1 ab-2"), ["xé-1", "ab-2"]);
}

#[test]
fn prefiltered_scans_agree_with_unfiltered_scans() {
    let texts = [
        "Log line 00001 2026-10-29 error_code=ERR_0001 status=CRITICAL WARN_DISK é\nERR_12 err_9 ERRx WARN tiếng Việt\n",
        "é234-56 x2026-10-29 ERR_ERR_1 WARNWARN_A",
        "",
        "ERR",
    ];
    for pattern in [
        r"ERR_\d{4}",
        r"ERR_\d{2,4}",
        r"\d{4}-\d{2}",
        r"status=\w+",
        r"ERR|WARN",
        r"(?i)err_\d+",
        r"WARN_[A-Z]+|ERR_\d+",
        r"^Log",
        r"\w+$",
        r"[EW][RA]",
        r"t.ếng",
        r"x?ERR",
        r"(?i)tiếng|việt",
        r"\bWARN\b",
    ] {
        let prog = nfa(pattern);
        assert!(!matches!(prog.prefilter, Prefilter::None), "{pattern} should have a prefilter");
        let plain = unfiltered(&prog);
        for text in texts {
            let fast = all_matches(&prog, text);
            assert_eq!(fast, all_matches(&plain, text), "{pattern} on {text:?}");
            assert_eq!(fast, pike_matches(&prog, text), "{pattern} on {text:?} (pikevm)");
        }
        assert!(!all_matches(&prog, texts[0]).is_empty(), "{pattern} should match the sample");
    }
}

#[test]
fn prefilter_picks_the_strongest_literal() {
    let literal = |pattern: &str| match nfa(pattern).prefilter {
        Prefilter::Literal { finder, offset } => Some((String::from_utf8(finder.needle().to_vec()).unwrap(), offset)),
        _ => None,
    };
    assert_eq!(literal(r"ERR_\d{2,4}"), Some(("ERR_".into(), 0)));
    assert_eq!(literal(r"(?i)Err_\d+"), Some(("err_".into(), 0)));
    assert_eq!(literal(r"\d{4}-\d{2}"), Some(("-".into(), 4)));
    assert_eq!(literal(r"[a-z]{2}\bfoo"), Some(("foo".into(), 2)));
    assert_eq!(literal(r"..-\d"), None, "`.` has no fixed byte width");
    assert!(matches!(nfa("ERR|WARN").prefilter, Prefilter::Literals { .. }));
    assert!(matches!(nfa(r"\d+\.").prefilter, Prefilter::FirstByte(_)));
    assert!(matches!(nfa(r".*x").prefilter, Prefilter::None));
    assert!(matches!(nfa(r"a*").prefilter, Prefilter::None), "can match empty");
}

/// Deterministic pseudo-random bytes from a small alphabet, so needles recur.
fn lcg_bytes(seed: &mut u64, len: usize) -> Vec<u8> {
    const ALPHABET: &[u8] = b"aAbB_-\x7f\xc3";
    (0..len)
        .map(|_| {
            *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ALPHABET[(*seed >> 33) as usize % ALPHABET.len()]
        })
        .collect()
}

#[test]
fn simd_scanners_agree_with_naive_search() {
    let mut seed = 7;
    for round in 0..4000 {
        let hay = lcg_bytes(&mut seed, round % 97);
        let needle = lcg_bytes(&mut seed, 1 + round % 5);
        for ci in [false, true] {
            let finder = LiteralFinder::new(&needle, ci);
            let naive = hay.windows(needle.len()).position(|w| if ci { w.eq_ignore_ascii_case(&needle) } else { w == needle.as_slice() });
            assert_eq!(finder.find(&hay), naive, "ci={ci} needle={needle:?} hay={hay:?}");
        }
        assert_eq!(find_literal_cs(&hay, &needle), hay.windows(needle.len()).position(|w| w == needle.as_slice()));
        let mut bits = [false; 256];
        needle.iter().for_each(|&b| bits[b as usize] = true);
        let set = ByteSet::new(bits);
        let from = round % 7;
        let naive = hay.iter().enumerate().skip(from).find(|&(_, b)| bits[*b as usize]).map(|(i, _)| i);
        assert_eq!(set.find(&hay, from), naive, "set={needle:?} hay={hay:?} from={from}");
    }
}

#[test]
fn unanchored_scan_is_linear_on_failing_patterns() {
    // A per-start-position retry is quadratic here (~5e9 steps); a single pass is not.
    let text = "a".repeat(100_000);
    assert!(Regex::new("[^z]*z").unwrap().find_at(&text, 0).is_none());
    assert!(Regex::new(r"\w+@").unwrap().find_at(&text, 0).is_none());
}

#[test]
fn backtracker_overflow_falls_back_to_the_pikevm() {
    // ~1500 instructions leave the visited window ~1400 offsets wide, and the
    // attempt at 0 runs `[ab]*` across all 10,000 bytes.
    let re = Regex::new("[ab]*(?:c{0,250}){2}z").unwrap();
    let Engine::Nfa(prog) = &re.engine else { panic!("expected an NFA") };
    let text = format!("{}z", "ab".repeat(5_000));
    let mut backtracker = super::backtrack::Backtracker::new();
    backtracker.prepare(prog, text.len());
    backtracker.reset(0);
    assert!(matches!(backtracker.run(prog, &text, 0), Err(super::backtrack::Overflow)));
    assert_eq!(re.find_at(&text, 0).map(|m| m.range()), Some(0..text.len()));
    assert_eq!(re.find_iter(&text).count(), 1);
}

#[test]
fn backtracker_agrees_with_the_pikevm_on_priority_cases() {
    let cases = [
        ("a|ab", "ab abab"),
        ("ab|a", "ab aab"),
        ("a+?b", "aaab"),
        ("(?:a|ab)(?:c|bcd)", "abcd"),
        ("(a*)*b", "aaab aaa"),
        ("x*", "xxaxx"),
        (r"\bfoo\B", "foo foox xfoo"),
        ("(?s)a.*?b", "a\nb ab"),
        ("^$", "\n\n"),
        // Greedy class loops that must give characters back.
        (r"\w+o", "foo boo xo"),
        ("[^,]*,x", "a,b,x,y"),
        (r"\w*é", "caféé x"),
        ("[a-z]+[a-z]", "ab a"),
        ("(?:[a-z]*)*q", "abc q"),
        (r"\S+\s", "日本語 x"),
    ];
    for (pattern, text) in cases {
        let prog = match Regex::new(pattern).unwrap().engine {
            Engine::Nfa(prog) => prog,
            Engine::Literal(_) => continue,
        };
        assert_eq!(all_matches(&prog, text), pike_matches(&prog, text), "{pattern} on {text:?}");
    }
}

#[test]
fn malformed_patterns_return_err() {
    for bad in ["(", "[abc", "a{5,2}", "*abc", "abc\\"] {
        assert!(Regex::new(bad).is_err(), "expected error for {bad:?}");
    }
}

#[test]
fn case_variants_cover_every_char_with_the_same_fold() {
    let mut groups: std::collections::HashMap<char, Vec<char>> = std::collections::HashMap::new();
    for c in (0x80..=0x10FFFFu32).filter_map(char::from_u32) {
        groups.entry(super::syntax::fold(c)).or_default().push(c);
    }
    for members in groups.values().filter(|m| m.len() > 1) {
        for &c in members {
            let variants = super::syntax::case_variants(c);
            assert!(members.iter().all(|m| variants.contains(m)), "{c:?}: {variants:?} vs {members:?}");
        }
    }
}

#[test]
fn unicode_case_insensitive_alternation_uses_a_literal_prefilter() {
    let prog = nfa("(?i)tiếng|việt");
    let Prefilter::Literals { needles, .. } = &prog.prefilter else { panic!("{:?}", prog.prefilter) };
    assert!(needles.iter().all(|n| n.len() >= 4), "{needles:?}");
    assert_eq!(found("(?i)tiếng|việt", false, true, "TIẾNG tiẾng VIệT timestamp"), ["TIẾNG", "tiẾng", "VIệT"]);
    assert_eq!(found("(?i)σ", false, true, "Σσς"), ["Σ", "σ", "ς"]);
}

#[test]
fn required_literal_short_circuits_scans_without_it() {
    let prog = nfa("[^z]*z");
    assert_eq!(prog.required.as_ref().map(|f| f.needle().to_vec()), Some(b"z".to_vec()));
    assert!(nfa(r"ERR_\d+").required.is_none(), "the prefilter already looks for ERR_");
    let text = "a".repeat(1000);
    assert!(Regex::new("[^z]*z").unwrap().find_at(&text, 0).is_none());
    assert_eq!(found("[^z]*z", false, true, "abz cz"), ["abz", " cz"]);
    assert_eq!(found(r"\w+@example", false, true, "mail me@example x@example"), ["me@example", "x@example"]);
}
