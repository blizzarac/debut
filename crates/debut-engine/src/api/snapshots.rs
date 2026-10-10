//! Snapshots of the active sequence: save, compare, restore (TL-14).

use super::*;
use debut_core::id::SnapshotId;
use debut_project::Snapshot;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct SnapshotDto {
    pub id: String,
    pub name: String,
    pub clips: usize,
    pub duration: f64,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct ClipChangeDto {
    pub clip: String,
    /// "V1", "A2", …
    pub track: String,
    pub at: f64,
    /// "added", "removed", "moved", "trimmed", "retimed", "changed".
    pub kinds: Vec<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct SnapshotDiffDto {
    pub clips: Vec<ClipChangeDto>,
    pub markers_added: usize,
    pub markers_removed: usize,
    pub captions_added: usize,
    pub captions_removed: usize,
}

/// "V1" / "A2" labels for a sequence's tracks.
fn track_label(seq: &Sequence, track: TrackId) -> Option<String> {
    let t = seq.track(track)?;
    let n = seq
        .tracks
        .iter()
        .filter(|o| o.kind == t.kind)
        .position(|o| o.id == track)?
        + 1;
    Some(format!(
        "{}{n}",
        if t.kind == TrackKind::Audio { "A" } else { "V" }
    ))
}

impl Session {
    fn snapshot(&self, id: &str) -> Result<&Snapshot, String> {
        let id = SnapshotId(parse_id(id)?);
        let seq = self.first_sequence()?.id;
        self.project()
            .and_then(|p| p.snapshots.iter().find(|s| s.id == id))
            .filter(|s| s.sequence.id == seq)
            .ok_or_else(|| "no such snapshot of this sequence".into())
    }

    /// Snapshots of the active sequence, oldest first.
    pub fn snapshots(&self) -> Result<Vec<SnapshotDto>, String> {
        let seq = self.first_sequence()?.id;
        Ok(self
            .project()
            .map(|p| {
                p.snapshots
                    .iter()
                    .filter(|s| s.sequence.id == seq)
                    .map(|s| SnapshotDto {
                        id: id_str(s.id.0),
                        name: s.name.clone(),
                        clips: s.sequence.tracks.iter().map(|t| t.clips.len()).sum(),
                        duration: secs(s.sequence.duration()),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Save the active sequence as it is now; returns the snapshot id.
    pub fn take_snapshot(&mut self, name: String) -> Result<String, String> {
        let sequence = self.first_sequence()?.clone();
        let name = if name.trim().is_empty() {
            format!("Version {}", self.snapshots()?.len() + 1)
        } else {
            name.trim().to_string()
        };
        let id: SnapshotId = self.ids.fresh();
        self.exec(Command::AddSnapshot(Snapshot { id, name, sequence }))?;
        Ok(id_str(id.0))
    }

    /// Put the active sequence back to a snapshot (one undoable step).
    pub fn restore_snapshot(&mut self, id: &str) -> Result<(), String> {
        let seq = self.snapshot(id)?.sequence.clone();
        self.exec(Command::ReplaceSequence(seq))
    }

    pub fn remove_snapshot(&mut self, id: &str) -> Result<(), String> {
        let id = self.snapshot(id)?.id;
        self.exec(Command::RemoveSnapshot(id))
    }

    /// What changed from the snapshot to the sequence as it is now.
    pub fn compare_snapshot(&self, id: &str) -> Result<SnapshotDiffDto, String> {
        let old = &self.snapshot(id)?.sequence;
        let new = self.first_sequence()?;
        let d = debut_timeline::snapshot::diff(old, new);
        use debut_timeline::snapshot::ChangeKind as K;
        Ok(SnapshotDiffDto {
            clips: d
                .clips
                .iter()
                .map(|c| ClipChangeDto {
                    clip: id_str(c.clip.0),
                    track: track_label(new, c.track)
                        .or_else(|| track_label(old, c.track))
                        .unwrap_or_default(),
                    at: secs(c.at),
                    kinds: c
                        .kinds
                        .iter()
                        .map(|k| {
                            match k {
                                K::Added => "added",
                                K::Removed => "removed",
                                K::Moved => "moved",
                                K::Trimmed => "trimmed",
                                K::Retimed => "retimed",
                                K::Changed => "changed",
                            }
                            .to_string()
                        })
                        .collect(),
                })
                .collect(),
            markers_added: d.markers_added,
            markers_removed: d.markers_removed,
            captions_added: d.captions_added,
            captions_removed: d.captions_removed,
        })
    }
}
