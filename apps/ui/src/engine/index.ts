// The UI talks to the engine only through this interface (PLT-01). The Tauri
// transport forwards to native Rust over IPC; the wasm transport calls the same
// Rust compiled to WebAssembly. Feature differences arrive as capabilities, never
// as runtime failures (PLT-05).

export interface Engine {
  version(): Promise<string>;
  newProject(name: string): Promise<void>;
  openProject(json: string): Promise<void>;
  projectJson(): Promise<string>;
  /** Apply a `Command` (serde JSON form) through the undo history. */
  execute(commandJson: string): Promise<void>;
  undo(): Promise<boolean>;
  redo(): Promise<boolean>;
  canUndo(): Promise<boolean>;
}

export type Target = "desktop" | "browser";

export function detectTarget(): Target {
  return "__TAURI_INTERNALS__" in window ? "desktop" : "browser";
}

export async function loadEngine(): Promise<Engine> {
  if (detectTarget() === "desktop") {
    const { TauriEngine } = await import("./tauri");
    return new TauriEngine();
  }
  const { WasmEngine } = await import("./wasm");
  return WasmEngine.load();
}
