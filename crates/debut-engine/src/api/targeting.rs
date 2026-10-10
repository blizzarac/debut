//! Source patching and track targeting (TL-05). Patching says which track
//! the picture and the sound of a media go to when it is inserted from the
//! media panel; targeting says which tracks a playhead edit (blade) acts on.
//! Both are working state of the editing session, kept per sequence, not
//! project data, so they never enter the undo history.

use super::*;
use std::collections::HashSet;

/// Where one kind of source goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Slot {
    /// The sequence's first track of that kind.
    #[default]
    First,
    Track(TrackId),
    Off,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Patch {
    video: Slot,
    audio: Slot,
    /// Tracks switched out of targeting (all are targeted by default).
    untargeted: HashSet<TrackId>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct TargetingDto {
    /// Track the picture of an inserted media goes to; None = off.
    pub video_source: Option<String>,
    /// Track its sound goes to; None = off.
    pub audio_source: Option<String>,
    /// Tracks playhead edits act on.
    pub targeted: Vec<String>,
}

fn resolve(seq: &Sequence, slot: Slot, kind: TrackKind) -> Option<TrackId> {
    let first = || seq.tracks.iter().find(|t| t.kind == kind).map(|t| t.id);
    match slot {
        Slot::Off => None,
        Slot::First => first(),
        Slot::Track(id) => seq
            .track(id)
            .filter(|t| t.kind == kind)
            .map(|t| t.id)
            .or_else(first),
    }
}

impl Session {
    fn patch(&self) -> Result<(Patch, &Sequence), String> {
        let seq = self.first_sequence()?;
        Ok((self.patches.get(&seq.id).cloned().unwrap_or_default(), seq))
    }

    /// Patched source tracks and targeted tracks of the active sequence.
    pub fn targeting(&self) -> Result<TargetingDto, String> {
        let (p, seq) = self.patch()?;
        Ok(TargetingDto {
            video_source: resolve(seq, p.video, TrackKind::Video).map(|t| id_str(t.0)),
            audio_source: resolve(seq, p.audio, TrackKind::Audio).map(|t| id_str(t.0)),
            targeted: seq
                .tracks
                .iter()
                .filter(|t| !p.untargeted.contains(&t.id))
                .map(|t| id_str(t.id.0))
                .collect(),
        })
    }

    /// Send `kind` ("video" / "audio") sources to `track`, or nowhere (None).
    pub fn set_source_patch(&mut self, kind: &str, track: Option<String>) -> Result<(), String> {
        let kind = match kind {
            "video" => TrackKind::Video,
            "audio" => TrackKind::Audio,
            _ => return Err(format!("unknown source kind {kind}")),
        };
        let (mut p, seq) = self.patch()?;
        let slot = match track {
            None => Slot::Off,
            Some(t) => {
                let id = TrackId(parse_id(&t)?);
                if seq.track(id).map(|t| t.kind) != Some(kind) {
                    return Err("patch a source to a track of its kind".into());
                }
                Slot::Track(id)
            }
        };
        if kind == TrackKind::Video {
            p.video = slot;
        } else {
            p.audio = slot;
        }
        let id = seq.id;
        self.patches.insert(id, p);
        Ok(())
    }

    pub fn set_track_targeted(&mut self, track: &str, on: bool) -> Result<(), String> {
        let (mut p, seq) = self.patch()?;
        let id = TrackId(parse_id(track)?);
        seq.track(id).ok_or("track not found")?;
        if on {
            p.untargeted.remove(&id);
        } else {
            p.untargeted.insert(id);
        }
        let seq_id = seq.id;
        self.patches.insert(seq_id, p);
        Ok(())
    }

    /// Put `media` at `at` on the patched tracks: its picture on the video
    /// source track, its sound (when it has any) on the audio one, as an
    /// insert that pushes later clips along or an overwrite. One undo step.
    pub fn insert_media(&mut self, media: &str, at: f64, overwrite: bool) -> Result<(), String> {
        let (p, seq) = self.patch()?;
        let (seq_id, fr) = (seq.id, seq.frame_rate);
        let video = resolve(seq, p.video, TrackKind::Video);
        let audio = resolve(seq, p.audio, TrackKind::Audio);
        let media_id = MediaId(parse_id(media)?);
        let &(_, _, duration, has_audio) = self.probed.get(&media_id).ok_or("unknown media")?;
        let tracks: Vec<TrackId> = video
            .into_iter()
            .chain(audio.filter(|_| has_audio))
            .collect();
        if tracks.is_empty() {
            return Err("no source track is patched".into());
        }
        let at = frames_of(at, fr);
        let duration = fr.snap(duration).max(fr.frame_duration());
        let mut cmds = Vec::new();
        for track in tracks {
            let target = Target {
                sequence: seq_id,
                track,
            };
            let mut clip = Clip::new(
                self.ids.fresh(),
                ClipSource::Media(media_id),
                at,
                duration,
                Rational::ZERO,
            );
            cmds.push(if overwrite {
                Command::overwrite(target, clip, &mut self.ids)
            } else {
                clip.timeline_in = Rational::ZERO;
                Command::insert(target, at, vec![clip], &mut self.ids)
            });
        }
        self.exec(Command::Group(cmds))
    }

    /// Blade every targeted track that has a clip under `at` (one undo step).
    pub fn blade_targeted(&mut self, at: f64) -> Result<(), String> {
        let (p, seq) = self.patch()?;
        let (seq_id, fr) = (seq.id, seq.frame_rate);
        let at = frames_of(at, fr);
        let tracks: Vec<TrackId> = seq
            .tracks
            .iter()
            .filter(|t| !p.untargeted.contains(&t.id))
            .filter(|t| {
                t.clips
                    .iter()
                    .any(|c| c.timeline_in < at && at < c.timeline_out())
            })
            .map(|t| t.id)
            .collect();
        if tracks.is_empty() {
            return Ok(());
        }
        let cmds = tracks
            .into_iter()
            .map(|track| Command::Blade {
                target: Target {
                    sequence: seq_id,
                    track,
                },
                at,
                tail_id: self.ids.fresh(),
            })
            .collect();
        self.exec(Command::Group(cmds))
    }
}
