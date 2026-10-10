//! Ingest (MED-01, MED-02, MED-13): bring a folder of media (a camera card,
//! a delivery drive) into the project, optionally copying it to project
//! storage first. Copies keep the card's folder structure and are verified
//! by checksum before anything is imported from them.

/// Media files the importer takes, by extension (case-insensitive).
pub const MEDIA_EXTENSIONS: &[&str] = &[
    "mov", "mp4", "m4v", "mxf", "mkv", "avi", "webm", "mts", "m2ts", "mpg", "mpeg", "wav", "aif",
    "aiff", "mp3", "m4a", "flac", "ogg", "opus", "png", "jpg", "jpeg", "tif", "tiff", "dng",
];

/// Whether a file name looks like media (hidden files and sidecars are not).
pub fn is_media(name: &str) -> bool {
    if name.starts_with('.') {
        return false;
    }
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| MEDIA_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
}

/// Where each file goes when `files` under `source` are copied to `dest`,
/// keeping their paths relative to `source`.
pub fn copy_plan(source: &str, dest: &str, files: &[String]) -> Vec<(String, String)> {
    let src = source.trim_end_matches(['/', '\\']);
    let dst = dest.trim_end_matches(['/', '\\']);
    files
        .iter()
        .map(|f| {
            let rel = f
                .strip_prefix(src)
                .unwrap_or(f)
                .trim_start_matches(['/', '\\']);
            (f.clone(), format!("{dst}/{rel}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_media_and_keeps_card_structure() {
        assert!(is_media("A001C003.MOV") && is_media("take.wav"));
        assert!(!is_media(".DS_Store") && !is_media("A001.XML") && !is_media("README"));
        assert_eq!(
            copy_plan(
                "/media/card/",
                "/proj/footage",
                &["/media/card/CLIP/A001.MOV".into()]
            ),
            vec![(
                "/media/card/CLIP/A001.MOV".to_string(),
                "/proj/footage/CLIP/A001.MOV".to_string()
            )]
        );
    }
}
