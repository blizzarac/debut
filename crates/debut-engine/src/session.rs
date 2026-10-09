//! Project files, autosave and crash recovery (MED-09, NFR-05, NFR-06).
//!
//! A [`Workspace`] owns the project, its history and an on-disk journal. Every
//! command is appended to `<project>.journal` as it happens; `save` writes the
//! project JSON (keeping versioned backups) and truncates the journal. Opening a
//! project whose journal is non-empty means the last session did not save: the
//! journal is replayed over the saved project, so at most the unflushed tail of
//! the last edit is lost.

use debut_command::{journal, Command, History, Journal};
use debut_core::{Error, Result};
use debut_platform::FileStore;
use debut_project::{schema, Project};
use std::sync::Arc;
use std::time::Duration;

/// Append-only JSON-lines journal in a [`FileStore`]. Each append rewrites the
/// whole file through the store's atomic write, which is fine for the journal
/// sizes between autosaves; a streaming append arrives with the native mmap store.
pub struct FileJournal {
    store: Arc<dyn FileStore>,
    path: String,
    lines: String,
}

impl FileJournal {
    pub fn open(store: Arc<dyn FileStore>, path: impl Into<String>) -> Self {
        let path = path.into();
        let lines = store
            .read(&path)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_default();
        Self { store, path, lines }
    }

    pub fn is_empty(&self) -> bool {
        self.lines.trim().is_empty()
    }

    pub fn clear(&mut self) -> Result<()> {
        self.lines.clear();
        self.store.write(&self.path, b"")
    }
}

impl Journal for FileJournal {
    fn append(&mut self, cmd: &Command) -> Result<()> {
        self.lines.push_str(&journal::encode_line(cmd)?);
        self.store.write(&self.path, self.lines.as_bytes())
    }

    fn replay(&self) -> Result<Vec<Command>> {
        journal::decode_lines(&self.lines)
    }
}

/// How many `.bak` generations `save` keeps.
pub const BACKUPS: usize = 5;

pub fn journal_path(project_path: &str) -> String {
    format!("{project_path}.journal")
}

fn backup_path(project_path: &str, n: usize) -> String {
    format!("{project_path}.bak{n}")
}

/// What `open` found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opened {
    /// Commands replayed from the journal because the last session didn't save.
    pub recovered: usize,
}

pub struct Workspace {
    store: Arc<dyn FileStore>,
    pub path: String,
    pub project: Project,
    pub history: History,
    journal: FileJournal,
    dirty: bool,
    /// Monotonic time of the last save as seen by `maybe_autosave`; `None`
    /// until the next check after creation, opening or a save.
    last_save: Option<Duration>,
    pub autosave_interval: Duration,
}

impl Workspace {
    /// Start a new project at `path` (nothing is written until the first command
    /// or save).
    pub fn create(
        store: Arc<dyn FileStore>,
        path: impl Into<String>,
        project: Project,
    ) -> Result<Self> {
        let path = path.into();
        let mut journal = FileJournal::open(Arc::clone(&store), journal_path(&path));
        if !journal.is_empty() {
            journal.clear()?;
        }
        Ok(Self {
            store,
            path,
            project,
            history: History::default(),
            journal,
            dirty: false,
            last_save: None,
            autosave_interval: Duration::from_secs(120),
        })
    }

    /// Open `path`, replaying any journal left by a crashed session.
    pub fn open(store: Arc<dyn FileStore>, path: impl Into<String>) -> Result<(Self, Opened)> {
        let path = path.into();
        let text =
            String::from_utf8(store.read(&path)?).map_err(|e| Error::Other(e.to_string()))?;
        let mut project = schema::from_json(&text)?;
        let journal = FileJournal::open(Arc::clone(&store), journal_path(&path));
        let pending = journal.replay()?;
        for cmd in &pending {
            cmd.apply(&mut project)?;
        }
        let recovered = pending.len();
        let ws = Self {
            store,
            path,
            project,
            history: History::default(),
            journal,
            dirty: recovered > 0,
            last_save: None,
            autosave_interval: Duration::from_secs(120),
        };
        Ok((ws, Opened { recovered }))
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn execute(&mut self, cmd: Command) -> Result<()> {
        self.history
            .execute(&mut self.project, cmd, &mut self.journal)?;
        self.dirty = true;
        Ok(())
    }

    pub fn undo(&mut self) -> Result<bool> {
        let r = self.history.undo(&mut self.project, &mut self.journal)?;
        self.dirty |= r;
        Ok(r)
    }

    pub fn redo(&mut self) -> Result<bool> {
        let r = self.history.redo(&mut self.project, &mut self.journal)?;
        self.dirty |= r;
        Ok(r)
    }

    /// Write the project, rotate backups, truncate the journal.
    pub fn save(&mut self) -> Result<()> {
        if self.store.exists(&self.path) {
            for n in (1..BACKUPS).rev() {
                let from = backup_path(&self.path, n);
                if self.store.exists(&from) {
                    let data = self.store.read(&from)?;
                    self.store.write(&backup_path(&self.path, n + 1), &data)?;
                }
            }
            let current = self.store.read(&self.path)?;
            self.store.write(&backup_path(&self.path, 1), &current)?;
        }
        self.store
            .write(&self.path, schema::to_json(&self.project)?.as_bytes())?;
        self.journal.clear()?;
        self.dirty = false;
        self.last_save = None;
        Ok(())
    }

    /// Save if dirty and the interval has passed since the last save. `now` is
    /// the platform's monotonic time; the interval starts at the first check
    /// after creation, opening or a save. Returns whether it saved.
    pub fn maybe_autosave(&mut self, now: Duration) -> Result<bool> {
        let since = *self.last_save.get_or_insert(now);
        if self.dirty && now.saturating_sub(since) >= self.autosave_interval {
            self.save()?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Save to a new path (Save As); backups and journal follow the new name.
    pub fn save_as(&mut self, path: impl Into<String>) -> Result<()> {
        let old_journal = journal_path(&self.path);
        self.path = path.into();
        self.journal = FileJournal::open(Arc::clone(&self.store), journal_path(&self.path));
        self.save()?;
        let _ = self.store.write(&old_journal, b"");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_command::Command;
    use debut_core::{FrameRate, IdGen};
    use debut_project::{Sequence, Track, TrackKind};
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory FileStore for tests.
    #[derive(Default)]
    struct MemStore(Mutex<HashMap<String, Vec<u8>>>);
    impl FileStore for MemStore {
        fn read(&self, path: &str) -> Result<Vec<u8>> {
            self.0
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .ok_or_else(|| Error::NotFound(path.into()))
        }
        fn write(&self, path: &str, data: &[u8]) -> Result<()> {
            self.0.lock().unwrap().insert(path.into(), data.to_vec());
            Ok(())
        }
        fn exists(&self, path: &str) -> bool {
            self.0.lock().unwrap().contains_key(path)
        }
        fn list(&self, _dir: &str) -> Result<Vec<String>> {
            Ok(self.0.lock().unwrap().keys().cloned().collect())
        }
    }

    fn add_sequence(ids: &mut IdGen, name: &str) -> Command {
        let seq = Sequence::new(ids.fresh(), name, FrameRate::FPS_25, 16, 9);
        let id = seq.id;
        Command::Group(vec![
            Command::AddSequence(seq),
            Command::AddTrack {
                sequence: id,
                track: Track::new(ids.fresh(), TrackKind::Video),
                index: None,
            },
        ])
    }

    #[test]
    fn save_then_crash_then_open_recovers_the_journal() {
        let store: Arc<dyn FileStore> = Arc::new(MemStore::default());
        let mut ids = IdGen::new(1);
        let project = Project::new(ids.fresh(), "p");
        {
            let mut ws = Workspace::create(Arc::clone(&store), "a.debut", project).unwrap();
            ws.execute(add_sequence(&mut ids, "one")).unwrap();
            ws.save().unwrap();
            assert!(!ws.is_dirty());
            ws.execute(add_sequence(&mut ids, "two")).unwrap();
            ws.execute(add_sequence(&mut ids, "three")).unwrap();
            ws.undo().unwrap();
            assert!(ws.is_dirty());
            // Crash: dropped without saving.
        }
        let (ws, opened) = Workspace::open(Arc::clone(&store), "a.debut").unwrap();
        assert_eq!(opened.recovered, 3, "two commands and the undo");
        let names: Vec<&str> = ws
            .project
            .sequences
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(names, vec!["one", "two"]);
        assert!(ws.is_dirty(), "recovered state is not on disk yet");
    }

    #[test]
    fn save_rotates_backups_and_truncates_the_journal() {
        let store: Arc<dyn FileStore> = Arc::new(MemStore::default());
        let mut ids = IdGen::new(2);
        let mut ws = Workspace::create(
            Arc::clone(&store),
            "b.debut",
            Project::new(ids.fresh(), "p"),
        )
        .unwrap();
        for i in 0..7 {
            ws.execute(add_sequence(&mut ids, &format!("s{i}")))
                .unwrap();
            ws.save().unwrap();
        }
        assert!(
            store.exists("b.debut.bak1")
                && store.exists("b.debut.bak5")
                && !store.exists("b.debut.bak6")
        );
        let bak1: Project =
            schema::from_json(std::str::from_utf8(&store.read("b.debut.bak1").unwrap()).unwrap())
                .unwrap();
        assert_eq!(bak1.sequences.len(), 6, "bak1 is the previous save");
        assert!(store.read("b.debut.journal").unwrap().is_empty());
        let (reopened, opened) = Workspace::open(Arc::clone(&store), "b.debut").unwrap();
        assert_eq!(opened.recovered, 0);
        assert_eq!(reopened.project.sequences.len(), 7);
    }

    #[test]
    fn autosave_fires_only_when_dirty_and_due() {
        let store: Arc<dyn FileStore> = Arc::new(MemStore::default());
        let mut ids = IdGen::new(3);
        let mut ws = Workspace::create(
            Arc::clone(&store),
            "c.debut",
            Project::new(ids.fresh(), "p"),
        )
        .unwrap();
        ws.autosave_interval = Duration::from_secs(10);
        let s = Duration::from_secs;
        assert!(
            !ws.maybe_autosave(s(100)).unwrap(),
            "first check starts the interval"
        );
        assert!(
            !ws.maybe_autosave(s(160)).unwrap(),
            "clean: nothing to save"
        );
        ws.execute(add_sequence(&mut ids, "s")).unwrap();
        assert!(ws.maybe_autosave(s(170)).unwrap(), "dirty and due");
        assert!(store.exists("c.debut"));
        assert!(!ws.is_dirty());
        // After a save the interval restarts at the next check.
        ws.execute(add_sequence(&mut ids, "t")).unwrap();
        assert!(!ws.maybe_autosave(s(171)).unwrap(), "restarted");
        assert!(!ws.maybe_autosave(s(175)).unwrap(), "dirty but not due");
        assert!(ws.maybe_autosave(s(181)).unwrap());
    }

    #[test]
    fn save_as_moves_the_journal() {
        let store: Arc<dyn FileStore> = Arc::new(MemStore::default());
        let mut ids = IdGen::new(4);
        let mut ws = Workspace::create(
            Arc::clone(&store),
            "d.debut",
            Project::new(ids.fresh(), "p"),
        )
        .unwrap();
        ws.execute(add_sequence(&mut ids, "s")).unwrap();
        ws.save_as("e.debut").unwrap();
        ws.execute(add_sequence(&mut ids, "t")).unwrap();
        assert!(!store.read("e.debut.journal").unwrap().is_empty());
        assert!(store.read("d.debut.journal").unwrap().is_empty());
        let (_, opened) = Workspace::open(Arc::clone(&store), "e.debut").unwrap();
        assert_eq!(opened.recovered, 1);
    }
}
