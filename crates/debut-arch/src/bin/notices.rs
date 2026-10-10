//! `cargo run -p debut-arch --bin notices`: regenerate THIRD_PARTY_NOTICES.md
//! (NFR-14) from cargo metadata and the UI's node_modules.
#![allow(clippy::disallowed_methods)] // tooling writes the file directly

fn main() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let text = debut_arch::licenses::generate(root).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
    let path = root.join("THIRD_PARTY_NOTICES.md");
    std::fs::write(&path, text).expect("write THIRD_PARTY_NOTICES.md");
    println!("wrote {}", path.display());
}
