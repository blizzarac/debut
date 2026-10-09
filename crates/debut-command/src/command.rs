use debut_core::Result;
use debut_project::Project;
use serde::{Deserialize, Serialize};

/// A reversible, serializable edit. Each variant carries enough to undo itself.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Command {
    // Media / project (MED)
    ImportMedia { path: String },
    // Timeline (TL-03 .. TL-06)
    Insert,
    Overwrite,
    Lift,
    Extract,
    Ripple,
    Roll,
    Slip,
    Slide,
    Blade,
    // Grouping: a user-visible step made of several commands
    Group(Vec<Command>),
}

impl Command {
    pub fn apply(&self, _project: &mut Project) -> Result<()> {
        todo!("apply command to project")
    }

    pub fn invert(&self, _project: &Project) -> Result<Command> {
        todo!("compute inverse command")
    }
}
