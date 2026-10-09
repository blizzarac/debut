//! System font discovery for titles and captions (GFX-01): an explicit file
//! path, else `<family>.ttf` in the usual font folders, else the first common
//! fallback face found.

use std::path::{Path, PathBuf};

/// Where fonts are looked for, in order.
const FONT_DIRS: &[&str] = &[
    "/usr/share/fonts",
    "/usr/local/share/fonts",
    "/Library/Fonts",
    "/System/Library/Fonts",
    "C:\\Windows\\Fonts",
];

const FALLBACKS: &[&str] = &[
    "DejaVuSans.ttf",
    "LiberationSans-Regular.ttf",
    "Arial.ttf",
    "arial.ttf",
    "Helvetica.ttc",
    "NotoSans-Regular.ttf",
];

fn find_file(dir: &Path, name: &str, depth: u32) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            subdirs.push(p);
        } else if p
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
        {
            return Some(p);
        }
    }
    if depth > 0 {
        for d in subdirs {
            if let Some(p) = find_file(&d, name, depth - 1) {
                return Some(p);
            }
        }
    }
    None
}

/// Resolve `family` to a font file.
pub fn find_font(family: &str) -> Option<PathBuf> {
    let p = Path::new(family);
    if p.is_file() {
        return Some(p.to_path_buf());
    }
    let candidates: Vec<String> = [
        format!("{family}.ttf"),
        format!("{}.ttf", family.replace(' ', "")),
    ]
    .into_iter()
    .chain(FALLBACKS.iter().map(|s| s.to_string()))
    .collect();
    for name in candidates {
        for dir in FONT_DIRS {
            if let Some(p) = find_file(Path::new(dir), &name, 3) {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn some_font_resolves_and_paths_pass_through() {
        let p = find_font("DejaVu Sans").expect("a system or fallback font");
        assert!(p.is_file());
        assert_eq!(find_font(p.to_str().unwrap()), Some(p.clone()));
    }
}
