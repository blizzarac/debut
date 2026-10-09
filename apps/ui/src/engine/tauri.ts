import { invoke } from "@tauri-apps/api/core";
import type { Engine } from "./index";

export class TauriEngine implements Engine {
  version() {
    return invoke<string>("version");
  }
  newProject(name: string) {
    return invoke<void>("new_project", { name });
  }
  openProject(json: string) {
    return invoke<void>("open_project", { json });
  }
  projectJson() {
    return invoke<string>("project_json");
  }
  execute(commandJson: string) {
    return invoke<void>("execute", { commandJson });
  }
  undo() {
    return invoke<boolean>("undo");
  }
  redo() {
    return invoke<boolean>("redo");
  }
  canUndo() {
    return invoke<boolean>("can_undo");
  }
}
