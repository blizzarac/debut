//! Roles (COL-08). Editors change anything; reviewers only leave comments,
//! which are markers (COL-04), so a review pass can never alter the cut.

use debut_command::Command;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Editor,
    Reviewer,
}

/// Whether `role` may submit `cmd`.
pub fn allowed(role: Role, cmd: &Command) -> bool {
    match role {
        Role::Editor => true,
        Role::Reviewer => crate::review::is_comment(cmd),
    }
}
