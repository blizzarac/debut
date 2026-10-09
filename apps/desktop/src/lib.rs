//! Desktop shell (Tauri). Exposes the engine to `apps/ui` over IPC. The command
//! set mirrors `debut-web`, so the UI code is identical on both targets (PLT-01).
//! The installer stays small by keeping codecs and AI models out of the binary
//! (PLT-10).

use debut_command::{Command, History, Journal, MemoryJournal};
use debut_core::IdGen;
use debut_project::{schema, Project};
use std::sync::Mutex;
use tauri::State;

#[derive(Default)]
pub struct Session {
    project: Option<Project>,
    history: History,
    journal: MemoryJournal,
}

type Shared = Mutex<Session>;

fn lock<'a>(state: &'a State<'a, Shared>) -> std::sync::MutexGuard<'a, Session> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

#[tauri::command]
fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
fn new_project(state: State<'_, Shared>, name: String) {
    let mut s = lock(&state);
    s.project = Some(Project::new(IdGen::random().fresh(), name));
    s.history = History::default();
    s.journal = MemoryJournal::default();
}

#[tauri::command]
fn open_project(state: State<'_, Shared>, json: String) -> Result<(), String> {
    let project = schema::from_json(&json).map_err(|e| e.to_string())?;
    let mut s = lock(&state);
    s.project = Some(project);
    s.history = History::default();
    s.journal = MemoryJournal::default();
    Ok(())
}

#[tauri::command]
fn project_json(state: State<'_, Shared>) -> Result<String, String> {
    let s = lock(&state);
    let p = s.project.as_ref().ok_or("no project open")?;
    schema::to_json(p).map_err(|e| e.to_string())
}

#[tauri::command]
fn execute(state: State<'_, Shared>, command_json: String) -> Result<(), String> {
    let cmd: Command = serde_json::from_str(&command_json).map_err(|e| e.to_string())?;
    let mut s = lock(&state);
    let Session {
        project,
        history,
        journal,
    } = &mut *s;
    let p = project.as_mut().ok_or("no project open")?;
    history
        .execute(p, cmd, journal as &mut dyn Journal)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn undo(state: State<'_, Shared>) -> Result<bool, String> {
    let mut s = lock(&state);
    let Session {
        project,
        history,
        journal,
    } = &mut *s;
    let p = project.as_mut().ok_or("no project open")?;
    history
        .undo(p, journal as &mut dyn Journal)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn redo(state: State<'_, Shared>) -> Result<bool, String> {
    let mut s = lock(&state);
    let Session {
        project,
        history,
        journal,
    } = &mut *s;
    let p = project.as_mut().ok_or("no project open")?;
    history
        .redo(p, journal as &mut dyn Journal)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn can_undo(state: State<'_, Shared>) -> bool {
    lock(&state).history.can_undo()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Shared::default())
        .invoke_handler(tauri::generate_handler![
            version,
            new_project,
            open_project,
            project_json,
            execute,
            undo,
            redo,
            can_undo
        ])
        .run(tauri::generate_context!())
        .expect("error while running debut");
}
