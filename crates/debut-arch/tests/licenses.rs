//! Licensing (NFR-14): every shipped dependency carries an allowed license,
//! and THIRD_PARTY_NOTICES.md matches what we build from.
#![allow(clippy::disallowed_methods)] // tests read the file system directly

use debut_arch::licenses::{allowed, generate, metadata, rust_packages};
use std::path::Path;

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

#[test]
fn shipped_dependencies_have_allowed_licenses() {
    let meta = metadata(root()).unwrap();
    let pkgs = rust_packages(&meta);
    assert!(pkgs.len() > 100, "found {} packages", pkgs.len());
    assert!(pkgs.iter().any(|p| p.name == "ffmpeg-next"));
    // Build-only and test-only crates are not shipped.
    assert!(
        !pkgs.iter().any(|p| p.name == "cc"),
        "cc is a build dependency"
    );
    let bad: Vec<String> = pkgs
        .iter()
        .filter(|p| !allowed(&p.license))
        .map(|p| format!("{} {}: {:?}", p.name, p.version, p.license))
        .collect();
    assert!(
        bad.is_empty(),
        "licenses outside the allowed list:\n{}",
        bad.join("\n")
    );
}

#[test]
fn notices_are_current() {
    if !root().join("apps/ui/node_modules").exists() {
        eprintln!("skipped: apps/ui/node_modules is missing (run pnpm install)");
        return;
    }
    let want = generate(root()).unwrap();
    let path = root().join("THIRD_PARTY_NOTICES.md");
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        have == want,
        "THIRD_PARTY_NOTICES.md is out of date: run `cargo run -p debut-arch --bin notices`"
    );
}
