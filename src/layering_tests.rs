//! Layer rules for the source tree (docs/plans/2026-11-01-layered-layout.md):
//!
//! - `terminal` uses no crate module outside itself;
//! - `render` uses only itself and `terminal`;
//! - `platform` uses no crate module outside itself.
//!
//! `app` and the crate-root modules may use anything. Known exceptions live in
//! `TOLERATED`, each with the reason it is still there.

use std::path::Path;

/// Crate-root modules any layer may use from `#[cfg(test)]` code.
const TEST_INFRA: &[&str] = &["test_alloc", "test_window"];

/// `(layer, used crate module, why it is still allowed)`.
const TOLERATED: &[(&str, &str, &str)] = &[];

/// First path segment of every `crate::` path in `src`, including each
/// branch of a `use crate::{a, b::{c, d}}` group.
fn crate_heads(src: &str) -> Vec<String> {
    let mut heads = Vec::new();
    let mut rest = src;
    while let Some(i) = rest.find("crate::") {
        rest = &rest[i + "crate::".len()..];
        let Some(group) = rest.strip_prefix('{') else {
            let head: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if !head.is_empty() {
                heads.push(head);
            }
            continue;
        };
        let (mut depth, mut want, mut cur) = (0usize, true, String::new());
        for c in group.chars() {
            if c.is_alphanumeric() || c == '_' {
                if want {
                    cur.push(c);
                }
                continue;
            }
            if want && !cur.is_empty() {
                heads.push(std::mem::take(&mut cur));
                want = false;
            }
            match c {
                '{' => depth += 1,
                '}' if depth == 0 => break,
                '}' => depth -= 1,
                ',' if depth == 0 => want = true,
                _ => {}
            }
        }
    }
    heads
}

/// Every `.rs` file under `src/<layer>/`, recursively, as `(path, contents)`.
fn layer_files(layer: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(layer)];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).expect("read source");
                out.push((path.display().to_string(), text));
            }
        }
    }
    assert!(!out.is_empty(), "no sources under src/{layer}");
    out
}

/// `path: crate::x` for every crate module a layer uses outside `allowed`.
fn violations(layer: &str, allowed: &[&str]) -> Vec<String> {
    let mut bad = Vec::new();
    for (path, text) in layer_files(layer) {
        for head in crate_heads(&text) {
            let ok = head == layer
                || allowed.contains(&head.as_str())
                || TEST_INFRA.contains(&head.as_str())
                || TOLERATED.iter().any(|(l, m, _)| *l == layer && *m == head);
            if !ok {
                bad.push(format!("{path}: crate::{head}"));
            }
        }
    }
    bad.sort();
    bad.dedup();
    bad
}

#[test]
fn crate_heads_reads_paths_and_nested_groups() {
    let src = "use crate::{terminal::{grid, pty}, render, AppEvent};\n/// [`crate::bidi::x`]\nlet c = crate::config::X;\nmacro_rules! m { () => { $crate::platform::beep() } }";
    assert_eq!(crate_heads(src), ["terminal", "render", "AppEvent", "bidi", "config", "platform"]);
}

#[test]
fn terminal_layer_is_self_contained() {
    let bad = violations("terminal", &[]);
    assert!(bad.is_empty(), "terminal must not depend on other layers:\n{}", bad.join("\n"));
}

#[test]
fn platform_layer_is_self_contained() {
    let bad = violations("platform", &[]);
    assert!(bad.is_empty(), "platform must not depend on other layers:\n{}", bad.join("\n"));
}

#[test]
fn render_layer_uses_only_terminal() {
    let bad = violations("render", &["terminal"]);
    assert!(bad.is_empty(), "render may only depend on terminal:\n{}", bad.join("\n"));
}

/// Non-test `mod` names declared in `text`, skipping `#[cfg(test)]` items.
fn production_modules(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut test_only = false;
    for line in text.lines().map(str::trim) {
        let decl = line.rsplit_once("] ").map_or(line, |(_, rest)| rest);
        // `pub mod`, `pub(crate) mod` and `pub(super) mod` all declare a module.
        let decl = match decl.strip_prefix("pub") {
            Some(rest) if rest.starts_with(' ') => rest.trim_start(),
            Some(rest) if rest.starts_with('(') => rest.split_once(") ").map_or(decl, |(_, after)| after),
            _ => decl,
        };
        if let Some(name) = decl.strip_prefix("mod ").and_then(|rest| rest.strip_suffix(';')) {
            if !test_only {
                names.push(name.to_owned());
            }
            test_only = false;
        } else {
            test_only = line == "#[cfg(test)]";
        }
    }
    names.sort();
    names
}

/// tests/native_activation.rs recompiles the crate's modules by path because
/// this binary crate has no library; a module added to main.rs must be listed
/// there too, or that target silently diverges from the real binary.
#[test]
fn native_activation_test_compiles_every_production_module() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |path: &str| std::fs::read_to_string(root.join(path)).expect("read source");
    let main = production_modules(&read("src/main.rs"));
    let mut native = production_modules(&read("tests/native_activation.rs"));
    // Test-only infrastructure the native target needs for its own counting allocator.
    native.retain(|name| !TEST_INFRA.contains(&name.as_str()));
    assert_eq!(native, main, "keep tests/native_activation.rs in sync with src/main.rs");
}

#[test]
fn production_modules_skip_test_only_declarations() {
    let text = "mod a;\n#[cfg(test)]\nmod t;\npub mod b;\n#[path = \"x.rs\"] mod c;\npub(crate) mod d;\n";
    assert_eq!(production_modules(text), ["a", "b", "c", "d"]);
}
