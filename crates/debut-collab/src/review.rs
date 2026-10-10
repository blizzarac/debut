//! Review comments (COL-03, COL-04) are markers: timeline markers for notes
//! on a moment, clip markers for notes on a shot, with `resolved` tracking
//! their state. A command is a comment when it only touches markers.

use debut_command::Command;

pub fn is_comment(cmd: &Command) -> bool {
    match cmd {
        Command::AddMarker { .. } | Command::UpdateMarker { .. } | Command::RemoveMarker { .. } => {
            true
        }
        Command::Group(cmds) => !cmds.is_empty() && cmds.iter().all(is_comment),
        _ => false,
    }
}
