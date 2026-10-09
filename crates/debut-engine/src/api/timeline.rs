//! Sequence DTOs, clip insertion and the timeline edit operations (TL-03 .. TL-07).

use super::*;

/// Build the command that nests `[start, end)` of `seq`: a new sequence with the
/// same track layout holding copies of the material in range (re-based to 0),
/// and, on every track that had something there, a compound clip overwriting
/// the range. Clips straddling the range are split by the overwrite.
pub(crate) fn nest_command(
    seq: &Sequence,
    start: Rational,
    end: Rational,
    ids: &mut IdGen,
) -> debut_core::Result<Command> {
    if end <= start {
        return Err(debut_core::Error::InvalidArgument("nothing to nest".into()));
    }
    let mut nested = Sequence::new(
        ids.fresh(),
        format!("{} (nested)", seq.name),
        seq.frame_rate,
        seq.width,
        seq.height,
    );
    let mut cmds = Vec::new();
    let mut overwrites = Vec::new();
    for track in &seq.tracks {
        let mut inner = Track::new(ids.fresh(), track.kind);
        inner.mix = track.mix;
        inner.audio_effects = track.audio_effects.clone();
        for clip in &track.clips {
            if clip.timeline_out() <= start || clip.timeline_in >= end {
                continue;
            }
            let mut c = clip.clone();
            c.id = ids.fresh();
            c.transition_in = None;
            if c.timeline_in < start {
                c.trim_head_to(start);
            }
            if c.timeline_out() > end {
                c.trim_tail_to(end);
            }
            c.timeline_in -= start;
            inner.clips.push(c);
        }
        if !inner.clips.is_empty() {
            overwrites.push((
                track.id,
                Clip::new(
                    ids.fresh(),
                    ClipSource::Sequence(nested.id),
                    start,
                    end - start,
                    Rational::ZERO,
                ),
            ));
        }
        nested.tracks.push(inner);
    }
    if overwrites.is_empty() {
        return Err(debut_core::Error::InvalidArgument(
            "nothing to nest in that range".into(),
        ));
    }
    cmds.push(Command::AddSequence(nested));
    for (track, clip) in overwrites {
        let target = Target {
            sequence: seq.id,
            track,
        };
        cmds.push(Command::overwrite(target, clip, ids));
    }
    Ok(Command::Group(cmds))
}

#[derive(Serialize)]
pub struct ClipDto {
    pub id: String,
    pub media: Option<String>,
    /// Text and style when this is a title clip (GFX-01).
    pub title: Option<Title>,
    /// Name of the nested sequence when this is a compound clip (TL-07).
    pub nested: Option<String>,
    /// Multicam clips: number of angles and the active one (MED-11, TL-08).
    pub angles: Option<usize>,
    pub angle: Option<usize>,
    /// Per-angle head offsets in seconds, from audio sync.
    pub angle_offsets: Option<Vec<f64>>,
    pub timeline_in: f64,
    pub duration: f64,
    pub source_in: f64,
    pub transition_in: Option<f64>,
}

#[derive(Serialize)]
pub struct SequenceListDto {
    pub id: String,
    pub name: String,
    pub duration: f64,
    pub active: bool,
}

#[derive(Serialize)]
pub struct TrackDto {
    pub id: String,
    pub kind: String,
    pub clips: Vec<ClipDto>,
    pub mix: TrackMix,
    pub inserts: Vec<String>,
}

#[derive(Serialize)]
pub struct SequenceDto {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: [i64; 2],
    pub duration: f64,
    pub tracks: Vec<TrackDto>,
}

pub(crate) fn sequence_dto(seq: &Sequence) -> SequenceDto {
    SequenceDto {
        id: id_str(seq.id.0),
        name: seq.name.clone(),
        width: seq.width,
        height: seq.height,
        frame_rate: [seq.frame_rate.0.num, seq.frame_rate.0.den],
        duration: secs(seq.duration()),
        tracks: seq
            .tracks
            .iter()
            .map(|t| TrackDto {
                id: id_str(t.id.0),
                kind: format!("{:?}", t.kind).to_lowercase(),
                mix: t.mix,
                inserts: t.audio_effects.iter().map(insert_name).collect(),
                clips: t
                    .clips
                    .iter()
                    .map(|c| ClipDto {
                        id: id_str(c.id.0),
                        media: match &c.source {
                            ClipSource::Media(m) => Some(id_str(m.0)),
                            _ => None,
                        },
                        title: match &c.source {
                            ClipSource::Title(t) => Some(t.clone()),
                            _ => None,
                        },
                        nested: match &c.source {
                            ClipSource::Sequence(id) => Some(id_str(id.0)),
                            _ => None,
                        },
                        angles: match &c.source {
                            ClipSource::Multicam { angles, .. } => Some(angles.len()),
                            _ => None,
                        },
                        angle: match &c.source {
                            ClipSource::Multicam { active, .. } => Some(*active),
                            _ => None,
                        },
                        angle_offsets: match &c.source {
                            ClipSource::Multicam { offsets, .. } => {
                                Some(offsets.iter().map(|o| secs(*o)).collect())
                            }
                            _ => None,
                        },
                        timeline_in: secs(c.timeline_in),
                        duration: secs(c.duration),
                        source_in: secs(c.source_in),
                        transition_in: c.transition_in.map(|t| secs(t.duration)),
                    })
                    .collect(),
            })
            .collect(),
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EditOp {
    RippleHead {
        track: String,
        clip: String,
        delta: f64,
    },
    RippleTail {
        track: String,
        clip: String,
        delta: f64,
    },
    Roll {
        track: String,
        clip: String,
        delta: f64,
    },
    Slip {
        track: String,
        clip: String,
        delta: f64,
    },
    Slide {
        track: String,
        clip: String,
        delta: f64,
    },
    Move {
        track: String,
        clip: String,
        delta: f64,
    },
    Blade {
        track: String,
        at: f64,
    },
    Extract {
        track: String,
        start: f64,
        end: f64,
    },
    Lift {
        track: String,
        start: f64,
        end: f64,
    },
    /// Set (duration in seconds) or clear (`None`) the dissolve into `clip`.
    Transition {
        track: String,
        clip: String,
        duration: Option<f64>,
    },
    /// Collapse `[start, end)` on every track into a new sequence and put one
    /// compound clip per track in its place (TL-07).
    Nest {
        start: f64,
        end: f64,
    },
}

impl Session {
    /// Create the first sequence (sized from the first media, else 1080p25) with
    /// one video and one audio track, if the project has none.
    pub fn ensure_sequence(&mut self) -> Result<SequenceDto, String> {
        if self.first_sequence().is_err() {
            let (w, h, fr) = self
                .project()
                .and_then(|p| p.media.first())
                .and_then(|m| {
                    self.probed.get(&m.id).map(|&(w, h, _, _)| {
                        (w, h, m.metadata.frame_rate.unwrap_or(FrameRate::FPS_25))
                    })
                })
                .unwrap_or((1920, 1080, FrameRate::FPS_25));
            let seq = Sequence::new(self.ids.fresh(), "Sequence 1", fr, w, h);
            let seq_id = seq.id;
            let v = Track::new(self.ids.fresh(), TrackKind::Video);
            let a = Track::new(self.ids.fresh(), TrackKind::Audio);
            self.exec(Command::Group(vec![
                Command::AddSequence(seq),
                Command::AddTrack {
                    sequence: seq_id,
                    track: v,
                    index: None,
                },
                Command::AddTrack {
                    sequence: seq_id,
                    track: a,
                    index: None,
                },
            ]))?;
        }
        Ok(sequence_dto(self.first_sequence()?))
    }

    /// Insert the whole of `media` on `track` at `at` seconds (ripple).
    pub fn add_clip(&mut self, track: &str, media: &str, at: f64) -> Result<(), String> {
        let (seq_id, fr) = {
            let seq = self.first_sequence()?;
            (seq.id, seq.frame_rate)
        };
        let track_id = TrackId(parse_id(track)?);
        let media_id = MediaId(parse_id(media)?);
        let duration = self
            .probed
            .get(&media_id)
            .map(|p| p.2)
            .ok_or("unknown media")?;
        let clip = Clip::new(
            self.ids.fresh(),
            ClipSource::Media(media_id),
            Rational::ZERO,
            fr.snap(duration).max(fr.frame_duration()),
            Rational::ZERO,
        );
        let target = Target {
            sequence: seq_id,
            track: track_id,
        };
        let cmd = Command::insert(target, frames_of(at, fr), vec![clip], &mut self.ids);
        self.exec(cmd)
    }

    pub fn edit(&mut self, op: EditOp) -> Result<(), String> {
        let (seq_id, fr) = {
            let seq = self.first_sequence()?;
            (seq.id, seq.frame_rate)
        };
        let target = |t: &str| -> Result<Target, String> {
            Ok(Target {
                sequence: seq_id,
                track: TrackId(parse_id(t)?),
            })
        };
        let clip = |c: &str| -> Result<ClipId, String> { Ok(ClipId(parse_id(c)?)) };
        let Session { workspace, ids, .. } = self;
        let project = &workspace.as_ref().ok_or("no project open")?.project;
        let cmd = match op {
            EditOp::RippleHead {
                track,
                clip: c,
                delta,
            } => debut_timeline::ripple_head(
                project,
                target(&track)?,
                clip(&c)?,
                frames_of(delta, fr),
            ),
            EditOp::RippleTail {
                track,
                clip: c,
                delta,
            } => debut_timeline::ripple_tail(
                project,
                target(&track)?,
                clip(&c)?,
                frames_of(delta, fr),
            ),
            EditOp::Roll {
                track,
                clip: c,
                delta,
            } => debut_timeline::roll(project, target(&track)?, clip(&c)?, frames_of(delta, fr)),
            EditOp::Slip {
                track,
                clip: c,
                delta,
            } => Ok(debut_timeline::slip(
                target(&track)?,
                clip(&c)?,
                frames_of(delta, fr),
            )),
            EditOp::Slide {
                track,
                clip: c,
                delta,
            } => debut_timeline::slide(project, target(&track)?, clip(&c)?, frames_of(delta, fr)),
            EditOp::Move {
                track,
                clip: c,
                delta,
            } => Ok(Command::Move {
                target: target(&track)?,
                clip: clip(&c)?,
                delta: frames_of(delta, fr),
            }),
            EditOp::Blade { track, at } => Ok(Command::Blade {
                target: target(&track)?,
                at: frames_of(at, fr),
                tail_id: ids.fresh(),
            }),
            EditOp::Extract { track, start, end } => Ok(Command::extract(
                target(&track)?,
                frames_of(start, fr),
                frames_of(end, fr),
                ids,
            )),
            EditOp::Lift { track, start, end } => Ok(Command::lift(
                target(&track)?,
                frames_of(start, fr),
                frames_of(end, fr),
                ids,
            )),
            EditOp::Transition {
                track,
                clip: c,
                duration,
            } => Ok(Command::SetTransition {
                target: target(&track)?,
                clip: clip(&c)?,
                transition: duration.map(|d| Transition {
                    kind: TransitionKind::Dissolve,
                    duration: frames_of(d, fr),
                }),
            }),
            EditOp::Nest { start, end } => project
                .sequence(seq_id)
                .ok_or_else(|| debut_core::Error::NotFound("sequence".into()))
                .and_then(|seq| nest_command(seq, frames_of(start, fr), frames_of(end, fr), ids)),
        }
        .map_err(|e| e.to_string())?;
        self.exec(cmd)
    }
}
