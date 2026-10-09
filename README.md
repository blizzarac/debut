# debut

A professional, non-destructive video editor: one Rust engine for desktop (native) and
browser (WebAssembly), one shared TypeScript UI.

- Architecture and crate map: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Engine crates: `crates/`
- Shells: `apps/desktop` (Tauri), `apps/web` (WASM), `apps/ui` (React)
- Shaders: `shaders/` (WGSL, shared by all GPU backends)

```
cargo check --workspace
```
