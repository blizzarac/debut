//! Schema versioning. Older projects always open in newer versions (NFR-06); desktop
//! and browser share the schema byte for byte (PLT-03).

use crate::Project;
use debut_core::Result;

pub const CURRENT_SCHEMA: u32 = 1;

/// Upgrade `project` to [`CURRENT_SCHEMA`] in place. Each step is one version.
pub fn migrate(project: &mut Project) -> Result<()> {
    while project.schema_version < CURRENT_SCHEMA {
        match project.schema_version {
            // 0 => migrate_v0_to_v1(project)?,
            _ => break,
        }
    }
    Ok(())
}
