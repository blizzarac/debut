//! Browser entry point (PLT-01). Build with
//! `cargo build -p debut-web --target wasm32-unknown-unknown` (or wasm-pack).
//! Exposes the engine to the same TypeScript UI the desktop shell uses; the API
//! surface mirrors `debut_engine::api` and grows with it.

use debut_command::{Command, History, MemoryJournal};
use debut_core::IdGen;
use debut_project::{schema, Project};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// A project held in the WASM heap and edited through the command log.
#[wasm_bindgen]
pub struct WebProject {
    project: Project,
    history: History,
    journal: MemoryJournal,
    ids: IdGen,
}

#[wasm_bindgen]
impl WebProject {
    #[wasm_bindgen(constructor)]
    pub fn new(name: &str) -> WebProject {
        let mut ids = IdGen::random();
        WebProject {
            project: Project::new(ids.fresh(), name),
            history: History::default(),
            journal: MemoryJournal::default(),
            ids,
        }
    }

    /// Open a project from its JSON form (identical to the desktop format, PLT-03).
    pub fn open(json: &str) -> Result<WebProject, JsError> {
        let project = schema::from_json(json).map_err(|e| JsError::new(&e.to_string()))?;
        Ok(WebProject {
            project,
            history: History::default(),
            journal: MemoryJournal::default(),
            ids: IdGen::random(),
        })
    }

    pub fn to_json(&self) -> Result<String, JsError> {
        schema::to_json(&self.project).map_err(|e| JsError::new(&e.to_string()))
    }

    /// Apply a command given as JSON (the `Command` enum's serde form).
    pub fn execute(&mut self, command_json: &str) -> Result<(), JsError> {
        let cmd: Command =
            serde_json::from_str(command_json).map_err(|e| JsError::new(&e.to_string()))?;
        self.history
            .execute(&mut self.project, cmd, &mut self.journal)
            .map_err(|e| JsError::new(&e.to_string()))
    }

    pub fn undo(&mut self) -> Result<bool, JsError> {
        self.history
            .undo(&mut self.project, &mut self.journal)
            .map_err(|e| JsError::new(&e.to_string()))
    }

    pub fn redo(&mut self) -> Result<bool, JsError> {
        self.history
            .redo(&mut self.project, &mut self.journal)
            .map_err(|e| JsError::new(&e.to_string()))
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn fresh_id(&mut self) -> String {
        let id: debut_core::ClipId = self.ids.fresh();
        id.0.to_string()
    }
}
