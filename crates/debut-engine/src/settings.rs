//! Per-user settings kept outside any project: which plugin binaries the
//! user approved (NFR-13) and telemetry consent (NFR-15). Stored as JSON in
//! the platform's settings folder; a target without one keeps them in
//! memory for the session.

use debut_platform::Platform;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Approved plugin binaries: path -> SHA-256 (hex) of the approved file.
    pub trusted_plugins: BTreeMap<String, String>,
    /// Usage counts and crash reports are collected (NFR-15). Off unless
    /// the user turns it on.
    pub telemetry: bool,
}

const FILE: &str = "settings.json";

fn path(platform: &dyn Platform) -> Option<String> {
    platform
        .settings_dir()
        .map(|d| format!("{}/{FILE}", d.trim_end_matches('/')))
}

impl Settings {
    /// The saved settings, or the defaults when there are none (or they
    /// cannot be read: a damaged file is not fatal).
    pub fn load(platform: &dyn Platform) -> Self {
        path(platform)
            .and_then(|p| platform.file_store().read(&p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, platform: &dyn Platform) -> Result<(), String> {
        let Some(p) = path(platform) else {
            return Ok(());
        };
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        platform
            .file_store()
            .write(&p, &json)
            .map_err(|e| e.to_string())
    }
}
