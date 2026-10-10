//! Finding plugin binaries on disk, without loading any.

use debut_platform::plugin_host::PluginKind;
use std::path::{Path, PathBuf};

/// Directory names OpenFX bundles keep this platform's binary in.
fn ofx_arch_dirs() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["MacOS"]
    } else if cfg!(target_arch = "aarch64") {
        &["Linux-arm64", "Linux-aarch64"]
    } else if cfg!(target_arch = "x86_64") {
        &["Linux-x86-64"]
    } else {
        &["Linux-x86"]
    }
}

/// Every OpenFX binary and CLAP library under `dirs` (a few levels deep;
/// a path that is itself a plugin counts too), sorted.
pub fn list(dirs: &[String]) -> Vec<(PluginKind, String)> {
    let mut found = Vec::new();
    for d in dirs {
        walk(Path::new(d), 0, &mut found);
    }
    found.sort_by(|a, b| a.1.cmp(&b.1));
    found.dedup();
    found
}

fn walk(path: &Path, depth: usize, found: &mut Vec<(PluginKind, String)>) {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.ends_with(".ofx.bundle") {
        for arch in ofx_arch_dirs() {
            let dir = path.join("Contents").join(arch);
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.extension().is_some_and(|x| x == "ofx") {
                        found.push((PluginKind::OpenFx, p.to_string_lossy().into_owned()));
                    }
                }
            }
        }
        return;
    }
    if name.ends_with(".clap") {
        if path.is_file() {
            found.push((PluginKind::Clap, path.to_string_lossy().into_owned()));
        } else if let Some(bin) = mac_bundle_binary(path) {
            found.push((PluginKind::Clap, bin.to_string_lossy().into_owned()));
        }
        return;
    }
    if path.is_file() && name.ends_with(".ofx") {
        found.push((PluginKind::OpenFx, path.to_string_lossy().into_owned()));
        return;
    }
    if depth >= 4 || !path.is_dir() {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(path) {
        for e in entries.flatten() {
            walk(&e.path(), depth + 1, found);
        }
    }
}

/// `X.clap/Contents/MacOS/X` (a macOS bundle).
fn mac_bundle_binary(bundle: &Path) -> Option<PathBuf> {
    let dir = bundle.join("Contents").join("MacOS");
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_test_plugins_in_their_bundle_layouts() {
        let dir = env!("DEBUT_TEST_PLUGINS").to_string();
        let found = list(&[dir]);
        assert!(
            found
                .iter()
                .any(|(k, p)| *k == PluginKind::OpenFx && p.ends_with("TestGain.ofx")),
            "{found:?}"
        );
        assert!(
            found
                .iter()
                .any(|(k, p)| *k == PluginKind::Clap && p.ends_with("test-gain.clap")),
            "{found:?}"
        );
        assert!(list(&["/no/such/dir".into()]).is_empty());
    }
}
