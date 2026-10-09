//! Architecture tests: the real workspace obeys the layering rules, and the
//! rules catch the violations they exist for.

use debut_arch::{check, graph_from_metadata, Graph};
use std::process::Command;

fn workspace_graph() -> Graph {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let out = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
        ])
        .current_dir(root)
        .output()
        .expect("run cargo metadata");
    assert!(
        out.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    graph_from_metadata(&String::from_utf8(out.stdout).unwrap()).unwrap()
}

#[test]
fn workspace_follows_the_layering_rules() {
    let graph = workspace_graph();
    assert!(graph.contains_key("debut-engine") && graph.contains_key("debut-desktop"));
    let errors = check(&graph);
    assert!(
        errors.is_empty(),
        "architecture rules broken (see docs/ARCHITECTURE.md):\n  {}",
        errors.join("\n  ")
    );
}

/// The workspace graph with one crate's dependency list extended.
fn with(graph: &Graph, krate: &str, dep: &str) -> Graph {
    let mut g = graph.clone();
    g.get_mut(krate).unwrap().push(dep.to_string());
    g
}

#[test]
fn the_rules_catch_each_kind_of_violation() {
    let base = workspace_graph();
    let broken = |krate: &str, dep: &str| -> Vec<String> { check(&with(&base, krate, dep)) };
    let cases = [
        // Engine code reaching for a platform implementation (PLT-02).
        (
            "debut-engine",
            "debut-platform-native",
            "must not depend on debut-platform-native",
        ),
        // A feature crate depending upward on the engine.
        (
            "debut-audio",
            "debut-engine",
            "must not depend on debut-engine",
        ),
        // The document model knowing about a feature crate.
        (
            "debut-project",
            "debut-render",
            "must not depend on debut-render",
        ),
        // The shell bypassing the engine to use a feature crate directly.
        (
            "debut-desktop",
            "debut-render",
            "must not depend on debut-render",
        ),
        // A shell using the other target's platform.
        (
            "debut-desktop",
            "debut-platform-web",
            "must not depend on debut-platform-web",
        ),
        // A platform implementation depending on feature code.
        (
            "debut-platform-native",
            "debut-project",
            "must not depend on debut-project",
        ),
        // OS / codec / UI runtime crates outside their one home.
        ("debut-audio", "cpal", "must not use cpal"),
        ("debut-engine", "tauri", "must not use tauri"),
        ("debut-export", "ffmpeg-next", "must not use ffmpeg-next"),
        // The foundation picking up an arbitrary dependency.
        ("debut-core", "rand", "foundation and may only use"),
    ];
    for (krate, dep, expect) in cases {
        let errors = broken(krate, dep);
        assert!(
            errors.iter().any(|e| e.contains(expect)),
            "{krate} -> {dep} should be reported ({expect}); got {errors:?}"
        );
    }
    // A new crate must be placed before it can pass.
    let mut g = base.clone();
    g.insert("debut-new".into(), vec![]);
    assert!(check(&g)
        .iter()
        .any(|e| e.contains("not placed in a layer")));
}
