//! Telemetry (NFR-15), opt-in and local-first. Nothing is collected until
//! the user turns it on, and what is collected is anonymous: how often each
//! kind of edit and feature is used, how many sessions, and crash messages
//! with their source location. No paths, names, project content or media.
//! It stays in the settings folder (`telemetry.json`), the user can read
//! and export it, and it leaves the machine only when they press Send to an
//! endpoint they configured.

use debut_command::Command;
use debut_platform::{HttpBody, HttpRequest, Platform};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Crash reports kept (oldest dropped).
const MAX_CRASHES: usize = 20;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Report {
    /// Report format, for whoever reads it.
    pub schema: u32,
    pub app_version: String,
    /// e.g. "linux-x86_64".
    pub platform: String,
    pub sessions: u64,
    /// Edits by command kind ("blade", "add_effect", ...).
    pub edits: BTreeMap<String, u64>,
    /// Features used ("import", "export:YouTube 1080p", "script", ...).
    pub features: BTreeMap<String, u64>,
    pub crashes: Vec<Crash>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Crash {
    pub app_version: String,
    pub message: String,
    /// "file.rs:line" in debut's source.
    pub location: String,
}

fn path(platform: &dyn Platform) -> Option<String> {
    platform
        .settings_dir()
        .map(|d| format!("{}/telemetry.json", d.trim_end_matches('/')))
}

/// Strip anything that looks like a path or an address from a crash message,
/// so a panic about a file does not leak where the user keeps their media.
pub fn scrub(message: &str) -> String {
    message
        .split_whitespace()
        .map(|w| {
            let bare = w.trim_matches(|c: char| "\"'`()[]{},;:".contains(c));
            if bare.contains('/') || bare.contains('\\') || bare.contains('@') {
                "<redacted>"
            } else {
                w
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(500)
        .collect()
}

impl Report {
    pub fn new() -> Self {
        Self {
            schema: 1,
            app_version: env!("CARGO_PKG_VERSION").into(),
            platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
            ..Default::default()
        }
    }

    pub fn load(platform: &dyn Platform) -> Self {
        let mut r: Report = path(platform)
            .and_then(|p| platform.file_store().read(&p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let fresh = Report::new();
        r.schema = fresh.schema;
        r.app_version = fresh.app_version;
        r.platform = fresh.platform;
        r
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

    /// Count an edit by kind (the parts of a group, not the group).
    pub fn edit(&mut self, cmd: &Command) {
        match cmd {
            Command::Group(cmds) => cmds.iter().for_each(|c| self.edit(c)),
            Command::Noop => {}
            c => *self.edits.entry(c.kind().to_string()).or_default() += 1,
        }
    }

    pub fn feature(&mut self, name: &str) {
        *self.features.entry(name.to_string()).or_default() += 1;
    }

    pub fn crash(&mut self, message: &str, location: &str) {
        self.crashes.push(Crash {
            app_version: env!("CARGO_PKG_VERSION").into(),
            message: scrub(message),
            location: location
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(location)
                .into(),
        });
        let extra = self.crashes.len().saturating_sub(MAX_CRASHES);
        self.crashes.drain(..extra);
    }

    pub fn json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

/// Record a crash from a panic hook (the session may be mid-call and
/// locked): only when the user opted in.
pub fn record_crash(platform: &dyn Platform, message: &str, location: &str) {
    if !crate::settings::Settings::load(platform).telemetry {
        return;
    }
    let mut r = Report::load(platform);
    r.crash(message, location);
    let _ = r.save(platform);
}

/// POST the report as JSON to `endpoint`; returns the HTTP status.
pub fn send(platform: &dyn Platform, report: &Report, endpoint: &str) -> Result<u16, String> {
    if !(endpoint.starts_with("https://")
        || endpoint.starts_with("http://localhost")
        || endpoint.starts_with("http://127.0.0.1"))
    {
        return Err("telemetry goes only to an https:// endpoint (or localhost)".into());
    }
    let resp = platform
        .http(
            HttpRequest::new("POST", endpoint)
                .header("Content-Type", "application/json")
                .body(HttpBody::Bytes(report.json().into_bytes())),
        )
        .map_err(|e| e.to_string())?;
    if !(200..300).contains(&resp.status) {
        return Err(format!("the endpoint answered {}", resp.status));
    }
    Ok(resp.status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_paths_and_addresses() {
        let s = scrub("cannot open \"/home/ana/Footage/wedding.mp4\": No such file (ana@example.com) at C:\\Users\\ana");
        assert!(!s.contains("ana"), "{s}");
        assert!(s.contains("cannot open") && s.contains("No such file"));
        assert!(scrub(&"x".repeat(2000)).len() <= 500);
    }

    #[test]
    fn counts_group_parts_and_caps_crashes() {
        let mut r = Report::new();
        r.edit(&Command::Group(vec![
            Command::Noop,
            Command::Group(vec![Command::Noop]),
        ]));
        assert!(r.edits.is_empty());
        for i in 0..30 {
            r.crash(&format!("boom {i}"), "/build/src/player.rs:42");
        }
        assert_eq!(r.crashes.len(), MAX_CRASHES);
        assert_eq!(r.crashes[0].message, "boom 10");
        assert_eq!(r.crashes[0].location, "player.rs:42");
    }
}
