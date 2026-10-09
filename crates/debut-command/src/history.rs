//! Unlimited undo/redo backed by the command log (TL-11).

use crate::Command;

#[derive(Default)]
pub struct History {
    done: Vec<Command>,
    undone: Vec<Command>,
}

impl History {
    pub fn push(&mut self, cmd: Command) {
        self.done.push(cmd);
        self.undone.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }
}
