# debut

A professional, non-destructive video editor: one Rust engine for desktop (native) and
browser (WebAssembly), one shared TypeScript UI.

- Architecture and crate map: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Engine crates: `crates/`
- Shells: `apps/desktop` (Tauri), `apps/web` (WASM), `apps/ui` (React)
- Shaders: `shaders/` (WGSL, shared by all GPU backends)

## Build

System libraries (Debian/Ubuntu; see `.claude/hooks/session-start.sh` for the full list):
FFmpeg dev headers, ALSA, clang, a Vulkan driver, and for the desktop shell GTK 3 + WebKitGTK 4.1.

```sh
cargo test --workspace                                   # engine + integration tests
pnpm --dir apps/ui install && pnpm --dir apps/ui build   # shared UI -> apps/ui/dist
cargo run -p debut-desktop                               # desktop shell (embeds apps/ui/dist)
cargo build -p debut-web --target wasm32-unknown-unknown # browser engine
```

For the browser, run `wasm-bindgen --target web` on the `debut_web.wasm` artifact into
`apps/ui/public/wasm/` and serve `apps/ui` with `pnpm --dir apps/ui dev`.
