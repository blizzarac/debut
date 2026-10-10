//! Licensing (NFR-14): what the shipped programs are built from, whether
//! every license allows shipping it in an MIT/Apache-2.0 app, and the
//! third-party notices file that goes with the app.
//!
//! The Rust side comes from `cargo metadata` (normal dependencies of the
//! shipped crates, all targets), the web side from the UI's production
//! dependencies in `node_modules`, and the vendored C headers and data are
//! listed by hand. FFmpeg is linked dynamically from the system; its
//! license depends on how it was built, which the app reports at run time.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// The crates that end up in what we ship.
pub const SHIPPED: &[&str] = &[
    "debut-desktop",
    "debut-plugin-host",
    "debut-collab-server",
    "debut-web",
];

/// Licenses a dependency may carry (SPDX ids). All permissive, plus
/// MPL-2.0, whose copyleft stays inside the dependency's own files.
pub const ALLOWED: &[&str] = &[
    "MIT",
    "MIT-0",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "0BSD",
    "ISC",
    "Zlib",
    "Unicode-3.0",
    "Unicode-DFS-2016",
    "BSL-1.0",
    "CC0-1.0",
    "Unlicense",
    "WTFPL",
    "MPL-2.0",
    "MPL-2.0+",
    // Permissive data licence of Mozilla's CA list (webpki-roots): keep
    // the licence text with it, which the notices file does.
    "CDLA-Permissive-2.0",
];

/// Whether an SPDX expression is satisfiable with `ALLOWED` licenses
/// (`A OR B` needs one, `A AND B` both). Old-style `A/B` means `A OR B`.
pub fn allowed(expr: &str) -> bool {
    let spaced = expr
        .replace('/', " OR ")
        .replace('(', " ( ")
        .replace(')', " ) ");
    let mut tokens: Vec<String> = Vec::new();
    let mut words = spaced.split_whitespace().peekable();
    while let Some(w) = words.next() {
        // Keep "X WITH Y" as one licence id.
        if words.peek() == Some(&"WITH") {
            words.next();
            let exc = words.next().unwrap_or("");
            tokens.push(format!("{w} WITH {exc}"));
        } else {
            tokens.push(w.to_string());
        }
    }
    let mut pos = 0;
    matches!(or_expr(&tokens, &mut pos), Some(true)) && pos == tokens.len()
}

// `None`: not a well-formed expression.
fn or_expr(t: &[String], pos: &mut usize) -> Option<bool> {
    let mut v = and_expr(t, pos)?;
    while t.get(*pos).map(String::as_str) == Some("OR") {
        *pos += 1;
        v |= and_expr(t, pos)?;
    }
    Some(v)
}

fn and_expr(t: &[String], pos: &mut usize) -> Option<bool> {
    let mut v = atom(t, pos)?;
    while t.get(*pos).map(String::as_str) == Some("AND") {
        *pos += 1;
        v &= atom(t, pos)?;
    }
    Some(v)
}

fn atom(t: &[String], pos: &mut usize) -> Option<bool> {
    match t.get(*pos).map(String::as_str) {
        Some("(") => {
            *pos += 1;
            let v = or_expr(t, pos)?;
            if t.get(*pos).map(String::as_str) != Some(")") {
                return None;
            }
            *pos += 1;
            Some(v)
        }
        Some("OR" | "AND" | ")") | None => None,
        Some(id) => {
            *pos += 1;
            Some(ALLOWED.contains(&id))
        }
    }
}

/// One third-party package.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub license: String,
    pub repository: String,
    /// Where its license files are (the package's folder).
    pub dir: PathBuf,
}

/// Third-party crates reachable through normal dependencies from
/// `SHIPPED`, from `cargo metadata --format-version 1` JSON.
pub fn rust_packages(meta: &Value) -> Vec<Package> {
    let empty = Vec::new();
    let packages = meta["packages"].as_array().unwrap_or(&empty);
    let by_id: HashMap<&str, &Value> = packages
        .iter()
        .filter_map(|p| Some((p["id"].as_str()?, p)))
        .collect();
    let workspace: BTreeSet<&str> = meta["workspace_members"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let nodes: HashMap<&str, &Value> = meta["resolve"]["nodes"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .filter_map(|n| Some((n["id"].as_str()?, n)))
        .collect();
    let mut stack: Vec<&str> = packages
        .iter()
        .filter(|p| SHIPPED.contains(&p["name"].as_str().unwrap_or("")))
        .filter_map(|p| p["id"].as_str())
        .collect();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(node) = nodes.get(id) else { continue };
        for dep in node["deps"].as_array().unwrap_or(&empty) {
            // Normal dependencies only: build scripts and tests ship nothing.
            let normal = dep["dep_kinds"]
                .as_array()
                .unwrap_or(&empty)
                .iter()
                .any(|k| k["kind"].is_null());
            if normal {
                if let Some(d) = dep["pkg"].as_str() {
                    stack.push(d);
                }
            }
        }
    }
    let mut out: Vec<Package> = seen
        .into_iter()
        .filter(|id| !workspace.contains(id))
        .filter_map(|id| by_id.get(id))
        .map(|p| Package {
            name: p["name"].as_str().unwrap_or("").to_string(),
            version: p["version"].as_str().unwrap_or("").to_string(),
            license: p["license"].as_str().unwrap_or("").to_string(),
            repository: p["repository"].as_str().unwrap_or("").to_string(),
            dir: Path::new(p["manifest_path"].as_str().unwrap_or(""))
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default(),
        })
        .collect();
    out.sort();
    out
}

/// Production dependencies of the UI, from `node_modules` (pnpm layout:
/// a package's own dependencies sit next to its real folder).
#[allow(clippy::disallowed_methods)] // tooling reads the file system directly
pub fn npm_packages(ui: &Path) -> Option<Vec<Package>> {
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(ui.join("package.json")).ok()?).ok()?;
    let mut out: BTreeMap<String, Package> = BTreeMap::new();
    let mut stack: Vec<(PathBuf, String)> = manifest["dependencies"]
        .as_object()?
        .keys()
        .map(|k| (ui.join("node_modules"), k.clone()))
        .collect();
    while let Some((modules, name)) = stack.pop() {
        let dir = modules.join(&name);
        let Ok(real) = std::fs::canonicalize(&dir) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(real.join("package.json")) else {
            continue;
        };
        let Ok(pkg) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let version = pkg["version"].as_str().unwrap_or("").to_string();
        let key = format!("{name}@{version}");
        if out.contains_key(&key) {
            continue;
        }
        let repository = match &pkg["repository"] {
            Value::String(s) => s.clone(),
            v => v["url"].as_str().unwrap_or("").to_string(),
        };
        out.insert(
            key,
            Package {
                name: name.clone(),
                version,
                license: pkg["license"].as_str().unwrap_or("").to_string(),
                repository,
                dir: real.clone(),
            },
        );
        // pnpm: dependencies resolve from the folder holding the real package.
        let mut base = real.parent().map(Path::to_path_buf).unwrap_or_default();
        if name.contains('/') {
            base = base.parent().map(Path::to_path_buf).unwrap_or_default();
        }
        if let Some(deps) = pkg["dependencies"].as_object() {
            for d in deps.keys() {
                stack.push((base.clone(), d.clone()));
            }
        }
    }
    Some(out.into_values().collect())
}

/// Code and data vendored into the repository: (what, license, where).
pub const VENDORED: &[(&str, &str, &str)] = &[
    (
        "OpenFX headers",
        "BSD-3-Clause",
        "crates/debut-plugin-host/vendor/openfx",
    ),
    (
        "CLAP headers",
        "MIT",
        "crates/debut-plugin-host/vendor/clap",
    ),
    (
        "AAF baseline dictionary (generated with pyaaf2)",
        "MIT",
        "crates/debut-media/src/aaf",
    ),
];

/// License, copying and notice files in `dir`, as (file name, text).
#[allow(clippy::disallowed_methods)] // tooling reads the file system directly
pub fn license_files(dir: &Path) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(String, String)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let upper = name.to_ascii_uppercase();
            let wanted = [
                "LICENSE",
                "LICENCE",
                "COPYING",
                "NOTICE",
                "UNLICENSE",
                "COPYRIGHT",
            ]
            .iter()
            .any(|p| upper.starts_with(p));
            if !wanted || !e.path().is_file() {
                return None;
            }
            let text = std::fs::read_to_string(e.path()).ok()?;
            Some((name, text.replace("\r\n", "\n").trim().to_string()))
        })
        .collect();
    files.sort();
    files
}

/// The notices file: every package with its license, then each distinct
/// license text once with the packages that ship it.
pub fn notices(rust: &[Package], npm: Option<&[Package]>, repo: &Path) -> String {
    let mut out = String::from(
        "# Third-party notices\n\n\
         debut is MIT OR Apache-2.0. It is built from the open-source packages \
         below, under their own licenses. Generated by \
         `cargo run -p debut-arch --bin notices`; do not edit by hand.\n\n\
         FFmpeg is not bundled: debut links the FFmpeg libraries installed on \
         the system, under the license that build carries (LGPL-2.1-or-later, \
         or GPL-2.0-or-later when built with GPL parts such as libx264). The \
         About panel shows which one this copy of debut is using.\n",
    );
    let mut texts: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut missing: Vec<String> = Vec::new();
    let mut section = |title: &str, pkgs: &[Package], out: &mut String| {
        out.push_str(&format!("\n## {title} ({})\n\n", pkgs.len()));
        out.push_str("| Package | Version | License |\n| --- | --- | --- |\n");
        for p in pkgs {
            out.push_str(&format!(
                "| {} | {} | {} |\n",
                if p.repository.is_empty() {
                    p.name.clone()
                } else {
                    format!("[{}]({})", p.name, p.repository)
                },
                p.version,
                p.license
            ));
            let files = license_files(&p.dir);
            if files.is_empty() {
                missing.push(format!("{} {}", p.name, p.version));
            }
            for (_, text) in files {
                texts
                    .entry(text)
                    .or_default()
                    .insert(format!("{} {}", p.name, p.version));
            }
        }
    };
    section("Rust crates", rust, &mut out);
    if let Some(npm) = npm {
        section("Web UI packages", npm, &mut out);
    }
    out.push_str(
        "\n## Vendored code and data\n\n| What | License | Where |\n| --- | --- | --- |\n",
    );
    for (what, license, dir) in VENDORED {
        out.push_str(&format!("| {what} | {license} | `{dir}` |\n"));
        for (_, text) in license_files(&repo.join(dir)) {
            texts.entry(text).or_default().insert(what.to_string());
        }
    }
    if !missing.is_empty() {
        out.push_str(
            "\nThese packages ship no license file; their license is the one \
             listed above, as published in their repository, and the matching \
             standard text below applies with copyright held by their authors:\n\n",
        );
        for m in &missing {
            out.push_str(&format!("- {m}\n"));
        }
    }
    out.push_str("\n## License texts\n");
    for (text, users) in &texts {
        let users: Vec<&str> = users.iter().map(String::as_str).collect();
        out.push_str(&format!(
            "\n### Used by: {}\n\n```text\n{}\n```\n",
            users.join(", "),
            text.replace("```", "'''")
        ));
    }
    out
}

/// `cargo metadata` for the workspace at `root`, with dependencies.
pub fn metadata(root: &Path) -> Result<Value, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = std::process::Command::new(cargo)
        .args(["metadata", "--format-version", "1"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
}

/// The notices file for the workspace at `root`.
pub fn generate(root: &Path) -> Result<String, String> {
    let meta = metadata(root)?;
    let rust = rust_packages(&meta);
    let npm = npm_packages(&root.join("apps/ui"));
    Ok(notices(&rust, npm.as_deref(), root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spdx_expressions() {
        assert!(allowed("MIT"));
        assert!(allowed("MIT OR Apache-2.0"));
        assert!(allowed("MIT/Apache-2.0"));
        assert!(allowed(
            "Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT"
        ));
        assert!(allowed("(MIT OR Apache-2.0) AND Unicode-3.0"));
        assert!(allowed("MIT OR Apache-2.0 OR LGPL-2.1-or-later"));
        assert!(!allowed("GPL-3.0-only"));
        assert!(!allowed("MIT AND GPL-2.0-or-later"));
        assert!(!allowed("LGPL-2.1-or-later"));
        assert!(!allowed(""));
        assert!(!allowed("MIT OR"));
    }
}
