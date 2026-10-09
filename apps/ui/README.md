# debut-ui

Shared TypeScript/React application. Runs inside the Tauri shell (`apps/desktop`) and
as a web app against the WASM engine (`apps/web`). Capability differences between the
two targets surface as feature flags from the engine, never as runtime failures (PLT-05).

Scaffold pending (Vite + React + TypeScript).
