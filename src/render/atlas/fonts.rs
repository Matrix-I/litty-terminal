//! Primary font resolution (`font-family`, `LITTY_FONT`, platform defaults,
//! installed Nerd Fonts) and symbol-font fallbacks, cached per family list.

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;

use super::system::NativeFont;
use super::FontSet;

pub(super) static FONT_CACHE: Mutex<Option<(Vec<String>, Arc<FontSet>)>> = Mutex::new(None);

/// Load the primary monospace font and any available symbol/Nerd Font fallbacks.
///
/// `families` comes from the `font-family` config option: the first family
/// that resolves becomes the primary font, the remaining ones are tried as
/// fallbacks before the auto-discovered symbol fonts. Without configured
/// families, `LITTY_FONT=/path/to/font.ttf` or the platform defaults apply.
pub fn load_font(families: &[String]) -> Arc<FontSet> {
    let mut cache = FONT_CACHE.lock();
    if let Some((ref cached_families, ref font_set)) = *cache
        && cached_families.as_slice() == families
    {
        return Arc::clone(font_set);
    }

    let mut configured_faces: Vec<(String, u32)> = Vec::new();
    for fam in families {
        match crate::render::fontdb::find_family(fam) {
            Some(face) => {
                configured_faces.push((face.path.clone(), face.index));
            }
            None => eprintln!("litty: font-family `{fam}` not found; using fallback"),
        }
    }

    let mut primary_candidates: Vec<String> = Vec::new();
    if let Ok(p) = std::env::var("LITTY_FONT") {
        primary_candidates.push(p);
    }

    #[cfg(target_os = "macos")]
    primary_candidates.extend(
        [
            "/System/Applications/Utilities/Terminal.app/Contents/Resources/Fonts/SF-Mono-Regular.otf",
            "/System/Library/Fonts/SFNSMono.ttf",
            "/System/Library/Fonts/Menlo.ttc",
            "/System/Library/Fonts/Monaco.ttf",
        ]
        .map(String::from),
    );

    #[cfg(not(target_os = "macos"))]
    primary_candidates.extend(
        [
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
            "/usr/share/fonts/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/dejavu-sans-mono-fonts/DejaVuSansMono.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
            "/usr/share/fonts/liberation-mono/LiberationMono-Regular.ttf",
            "/usr/share/fonts/noto/NotoSansMono-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
        ]
        .map(String::from),
    );

    let (mut primary_path, mut primary) = (None, None);

    let mut configured_iter = configured_faces.into_iter();
    for (p, idx) in configured_iter.by_ref() {
        if let Some(f) = parse_font(&p, idx, true) {
            primary_path = Some(p);
            primary = Some(f);
            break;
        }
    }

    if primary.is_none() {
        for path in &primary_candidates {
            if let Some(f) = parse_font(path, 0, true) {
                primary_path = Some(path.clone());
                primary = Some(f);
                break;
            }
        }
    }

    // Only if none of the standard system/configured fonts were found, fall back to scanning for Nerd Fonts.
    if primary.is_none() {
        let (discovered_mono_nerd, _) = discover_nerd_fonts();
        for path in &discovered_mono_nerd {
            if let Some(f) = parse_font(path, 0, true) {
                primary_path = Some(path.clone());
                primary = Some(f);
                break;
            }
        }
    }
    let primary = primary.expect("litty: no usable monospace font found; set font-family in the config");

    let mut fallbacks: Vec<NativeFont> = Vec::new();
    for (p, idx) in configured_iter {
        if let Some(f) = try_parse_font_index(&p, idx) {
            fallbacks.push(f);
        }
    }
    if let Some(f) = find_first_symbol_font(primary_path.as_deref()) {
        fallbacks.push(f);
    }

    let set = Arc::new(FontSet { primary, fallbacks });
    *cache = Some((families.to_vec(), Arc::clone(&set)));
    set
}

fn find_first_symbol_font(exclude_path: Option<&str>) -> Option<NativeFont> {
    let mut candidates = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(format!("{home}/Library/Fonts/SymbolsNerdFont-Regular.ttf"));
        candidates.push(format!("{home}/Library/Fonts/SymbolsNerdFontMono-Regular.ttf"));
        candidates.push(format!("{home}/.local/share/fonts/SymbolsNerdFont-Regular.ttf"));
        candidates.push(format!("{home}/.fonts/SymbolsNerdFont-Regular.ttf"));
    }
    candidates.extend(
        [
            "/Library/Fonts/SymbolsNerdFont-Regular.ttf",
            "/Library/Fonts/SymbolsNerdFontMono-Regular.ttf",
            "/opt/homebrew/share/fonts/SymbolsNerdFont-Regular.ttf",
            "/usr/share/fonts/truetype/nerd-fonts/SymbolsNerdFont-Regular.ttf",
            "/usr/share/fonts/TTF/SymbolsNerdFont-Regular.ttf",
        ]
        .map(String::from),
    );
    for path in &candidates {
        if exclude_path == Some(path.as_str()) {
            continue;
        }
        if let Some(f) = try_parse_font(path) {
            return Some(f);
        }
    }
    None
}

fn try_parse_font(path: &str) -> Option<NativeFont> {
    try_parse_font_index(path, 0)
}

fn try_parse_font_index(path: &str, collection_index: u32) -> Option<NativeFont> {
    parse_font(path, collection_index, false)
}

/// `substitutions`: also configure GSUB/GPOS shaping (`font-feature`, ligatures)
/// on the primary font.
pub(super) fn parse_font(path: &str, collection_index: u32, substitutions: bool) -> Option<NativeFont> {
    super::system::parse_font(path, collection_index, substitutions)
}

/// Scan standard font directories for installed Nerd Fonts or Symbols Nerd Font.
fn discover_nerd_fonts() -> (Vec<String>, Vec<String>) {
    let mut mono_nerd = Vec::new();
    let mut symbols = Vec::new();

    let mut dirs = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(format!("{home}/Library/Fonts"));
        dirs.push(format!("{home}/.local/share/fonts"));
        dirs.push(format!("{home}/.fonts"));
    }
    dirs.extend(
        [
            "/Library/Fonts",
            "/Applications/Ghostty.app/Contents/Resources/fonts",
            "/usr/share/fonts",
            "/usr/local/share/fonts",
            "/opt/homebrew/share/fonts",
        ]
        .map(String::from),
    );

    for d in dirs {
        let Ok(entries) = std::fs::read_dir(Path::new(&d)) else { continue };
        for entry in entries.flatten() {
            let p = entry.path();
            let Some(name) = p.file_name().and_then(|s| s.to_str()) else { continue };
            let lower = name.to_ascii_lowercase();
            if !(lower.ends_with(".ttf") || lower.ends_with(".otf")) {
                continue;
            }
            if lower.contains("symbolsnerdfont") || lower.contains("nerd-fonts-symbols") {
                symbols.push(p.to_string_lossy().into_owned());
            } else if lower.contains("nerdfont")
                && (lower.contains("regular") || !lower.contains("italic") && !lower.contains("bold"))
            {
                mono_nerd.push(p.to_string_lossy().into_owned());
            }
        }
    }
    mono_nerd.sort();
    symbols.sort();
    (mono_nerd, symbols)
}
