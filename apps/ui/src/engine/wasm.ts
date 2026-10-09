import type { Engine } from "./index";

// Shape of the wasm-bindgen output of `apps/web` (debut-web), generated into
// `public/wasm/` by `wasm-bindgen --target web`.
interface WebProjectCtor {
  new (name: string): WebProjectInstance;
  open(json: string): WebProjectInstance;
}
interface WebProjectInstance {
  to_json(): string;
  execute(commandJson: string): void;
  undo(): boolean;
  redo(): boolean;
  can_undo(): boolean;
}
interface WasmModule {
  default(input?: string | URL): Promise<unknown>;
  version(): string;
  WebProject: WebProjectCtor;
}

export class WasmEngine implements Engine {
  private project: WebProjectInstance | null = null;

  private constructor(private readonly mod: WasmModule) {}

  static async load(): Promise<WasmEngine> {
    const url = new URL("/wasm/debut_web.js", window.location.href).href;
    const mod = (await import(/* @vite-ignore */ url)) as WasmModule;
    await mod.default();
    return new WasmEngine(mod);
  }

  private get p(): WebProjectInstance {
    if (!this.project) throw new Error("no project open");
    return this.project;
  }

  async version() {
    return this.mod.version();
  }
  async newProject(name: string) {
    this.project = new this.mod.WebProject(name);
  }
  async openProject(json: string) {
    this.project = this.mod.WebProject.open(json);
  }
  async projectJson() {
    return this.p.to_json();
  }
  async execute(commandJson: string) {
    this.p.execute(commandJson);
  }
  async undo() {
    return this.p.undo();
  }
  async redo() {
    return this.p.redo();
  }
  async canUndo() {
    return this.p.can_undo();
  }
}
