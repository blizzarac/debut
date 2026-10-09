//! Schema versioning and serialization. Older projects always open in newer versions
//! (NFR-06); desktop and browser share the schema byte for byte (PLT-03).

use crate::Project;
use debut_core::{Error, Result};

pub const CURRENT_SCHEMA: u32 = 1;

/// Upgrade `project` to [`CURRENT_SCHEMA`] in place. Each step is one version.
pub fn migrate(project: &mut Project) -> Result<()> {
    // Each step upgrades exactly one version; add arms as the schema grows.
    if project.schema_version < CURRENT_SCHEMA {
        return Err(Error::Other(format!(
            "no migration from schema {}",
            project.schema_version
        )));
    }
    if project.schema_version > CURRENT_SCHEMA {
        return Err(Error::Other(format!(
            "project schema {} is newer than this build ({CURRENT_SCHEMA})",
            project.schema_version
        )));
    }
    Ok(())
}

pub fn to_json(project: &Project) -> Result<String> {
    serde_json::to_string_pretty(project).map_err(|e| Error::Other(e.to_string()))
}

/// Parse and migrate.
pub fn from_json(json: &str) -> Result<Project> {
    let mut p: Project = serde_json::from_str(json).map_err(|e| Error::Other(e.to_string()))?;
    migrate(&mut p)?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Clip, ClipSource, Sequence, Track, TrackKind};
    use debut_core::{FrameRate, IdGen, Rational};

    #[test]
    fn project_round_trips_through_json() {
        let mut ids = IdGen::new(1);
        let mut p = Project::new(ids.fresh(), "Round trip");
        let mut seq = Sequence::new(ids.fresh(), "Main", FrameRate::FPS_23_976, 3840, 2160);
        let mut track = Track::new(ids.fresh(), TrackKind::Video);
        track.clips.push(Clip::new(
            ids.fresh(),
            ClipSource::Media(ids.fresh()),
            Rational::ZERO,
            FrameRate::FPS_23_976.frame_to_time(48),
            Rational::new(5, 1),
        ));
        seq.tracks.push(track);
        p.sequences.push(seq);

        let json = to_json(&p).unwrap();
        let back = from_json(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn newer_schema_is_rejected() {
        let mut p = Project::new(IdGen::new(2).fresh(), "Future");
        p.schema_version = CURRENT_SCHEMA + 1;
        assert!(migrate(&mut p).is_err());
    }
}
