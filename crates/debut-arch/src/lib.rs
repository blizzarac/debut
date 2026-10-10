//! The crate layering rules from `docs/ARCHITECTURE.md`, as code. `check` takes
//! the workspace graph (each crate with its normal dependencies) and returns
//! every rule broken; `tests/layering.rs` runs it on the real workspace.
//!
//! Dev- and build-dependencies are exempt: tests may pull in a platform
//! implementation to exercise the engine for real.

use std::collections::BTreeMap;

/// Where a crate sits. Engine-side layers are ordered; a crate may depend only
/// on engine-side crates at its own layer or below.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    /// `debut-core`: time, ids, errors. Depends on nothing in the workspace.
    Core,
    /// `debut-platform`: the traits every OS or browser call goes through.
    PlatformTraits,
    /// `debut-project`: the document model.
    Model,
    /// `debut-command`: the only way the model is mutated.
    Commands,
    /// Feature crates: media, timeline, audio, render, graphics, export, collab.
    Domain,
    /// `debut-engine`: the command surface the shells use.
    Engine,
    /// `debut-platform-native` / `-web`: implementations of the platform traits.
    PlatformImpl,
    /// Standalone services (the collaboration server, the plugin-host
    /// helper): may use the model, commands and domain crates, and own
    /// threads, sockets and processes.
    Service,
    /// `apps/desktop` / `apps/web`: translate IPC or JS calls to the engine.
    Shell,
    /// Workspace tooling (this crate).
    Tooling,
}

impl Layer {
    fn engine_side(self) -> bool {
        self <= Layer::Engine
    }
}

/// Every workspace crate must be placed here; a new crate fails the check
/// until someone decides where it belongs.
pub fn layer_of(name: &str) -> Option<Layer> {
    Some(match name {
        "debut-core" => Layer::Core,
        "debut-platform" => Layer::PlatformTraits,
        "debut-project" => Layer::Model,
        "debut-command" => Layer::Commands,
        "debut-media" | "debut-timeline" | "debut-audio" | "debut-render" | "debut-graphics"
        | "debut-export" | "debut-collab" => Layer::Domain,
        "debut-engine" => Layer::Engine,
        "debut-platform-native" | "debut-platform-web" => Layer::PlatformImpl,
        "debut-collab-server" | "debut-plugin-host" => Layer::Service,
        "debut-desktop" | "debut-web" => Layer::Shell,
        "debut-arch" => Layer::Tooling,
        _ => return None,
    })
}

/// The platform implementation each shell is built on.
fn own_platform(shell: &str) -> Option<&'static str> {
    match shell {
        "debut-desktop" => Some("debut-platform-native"),
        "debut-web" => Some("debut-platform-web"),
        _ => None,
    }
}

/// External crates that tie code to an OS, a device, a codec or a UI runtime,
/// and the only workspace crates allowed to use them.
const PLACED_EXTERNALS: &[(&str, &[&str])] = &[
    ("ffmpeg-next", &["debut-platform-native"]),
    ("cpal", &["debut-platform-native"]),
    ("tauri", &["debut-desktop"]),
    ("wasm-bindgen", &["debut-web", "debut-platform-web"]),
    ("web-sys", &["debut-web", "debut-platform-web"]),
    ("js-sys", &["debut-web", "debut-platform-web"]),
    ("wgpu", &["debut-render"]),
    ("pollster", &["debut-render"]),
    ("fontdue", &["debut-graphics"]),
];

/// The only external crates the foundation (`debut-core`, `debut-platform`)
/// may use: they compile on every target with no OS, GPU or codec code.
const FOUNDATION_EXTERNALS: &[&str] = &["serde", "serde_json", "thiserror"];

/// Crates a shell may use besides its own platform implementation: the engine
/// and the plain data types it exchanges with the UI.
const SHELL_WORKSPACE_DEPS: &[&str] = &[
    "debut-engine",
    "debut-core",
    "debut-project",
    "debut-command",
    // Hosting a collaboration session from the app.
    "debut-collab-server",
];

/// Workspace graph: crate name -> names of its normal (non-dev, non-build)
/// dependencies, workspace and external alike.
pub type Graph = BTreeMap<String, Vec<String>>;

/// Every rule broken by `graph`, as human-readable lines (empty when clean).
pub fn check(graph: &Graph) -> Vec<String> {
    let mut errors = Vec::new();
    for (krate, deps) in graph {
        let Some(layer) = layer_of(krate) else {
            errors.push(format!(
                "{krate}: not placed in a layer; add it to debut_arch::layer_of"
            ));
            continue;
        };
        for dep in deps {
            let dep_layer = graph.contains_key(dep).then(|| layer_of(dep)).flatten();
            match dep_layer {
                // Workspace dependency.
                Some(dl) => {
                    let ok = match layer {
                        _ if layer.engine_side() => dl.engine_side() && dl <= layer,
                        Layer::PlatformImpl => dl <= Layer::PlatformTraits,
                        // Services build on the model, commands and domain
                        // crates, never on the engine or a platform.
                        Layer::Service => dl.engine_side() && dl <= Layer::Domain,
                        Layer::Shell => {
                            SHELL_WORKSPACE_DEPS.contains(&dep.as_str())
                                || own_platform(krate) == Some(dep.as_str())
                        }
                        Layer::Tooling => false,
                        _ => unreachable!(),
                    };
                    if !ok {
                        errors.push(format!(
                            "{krate} ({layer:?}) must not depend on {dep} ({dl:?})"
                        ));
                    }
                }
                // Unplaced workspace crate: reported on its own line above.
                None if graph.contains_key(dep) => {}
                // External dependency.
                None => {
                    if let Some((_, allowed)) =
                        PLACED_EXTERNALS.iter().find(|(name, _)| name == dep)
                    {
                        if !allowed.contains(&krate.as_str()) {
                            errors.push(format!(
                                "{krate} must not use {dep} (only {})",
                                allowed.join(", ")
                            ));
                        }
                    }
                    if layer <= Layer::PlatformTraits
                        && !FOUNDATION_EXTERNALS.contains(&dep.as_str())
                    {
                        errors.push(format!(
                            "{krate} is foundation and may only use {}; found {dep}",
                            FOUNDATION_EXTERNALS.join(", ")
                        ));
                    }
                }
            }
        }
    }
    errors
}

/// Build the graph from `cargo metadata --format-version 1 --no-deps` JSON.
pub fn graph_from_metadata(json: &str) -> Result<Graph, String> {
    let meta: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let packages = meta["packages"]
        .as_array()
        .ok_or("cargo metadata has no packages")?;
    let mut graph = Graph::new();
    for p in packages {
        let name = p["name"].as_str().ok_or("package without a name")?;
        let deps = p["dependencies"]
            .as_array()
            .map(|ds| {
                ds.iter()
                    .filter(|d| d["kind"].is_null())
                    .filter_map(|d| d["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        graph.insert(name.to_string(), deps);
    }
    Ok(graph)
}
