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
    /// The shape when this is a shape clip (GFX-03).
    pub shape: Option<Shape>,
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
    /// Constant speed (1 normal, 0 freeze, negative reverse) and the ramp
    /// that overrides it when non-empty (TL-09).
    pub speed: f64,
    pub ramp: Vec<SpeedKeyDto>,
}

/// One speed-ramp key in clip-local seconds.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SpeedKeyDto {
    pub at: f64,
    pub speed: f64,
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
    /// Each insert's parameters: a plugin's (AUD-09), empty for built-ins.
    pub insert_params: Vec<Vec<PluginParamDto>>,
    /// Auto-ducking under another track (AUD-08).
    pub duck: Option<DuckDto>,
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
                insert_params: t.audio_effects.iter().map(insert_params).collect(),
                duck: t.duck.map(DuckDto::from),
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
                        shape: match &c.source {
                            ClipSource::Shape(s) => Some(s.clone()),
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
                        speed: c.speed.as_f64(),
                        ramp: c
                            .ramp
                            .iter()
                            .map(|k| SpeedKeyDto {
                                at: secs(k.at),
                                speed: k.speed,
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

#[derive(Clone, Debug, Deserialize)]
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
    /// Remove the gaps between clips on a track, rippling later clips left (TL-06).
    CloseGaps {
        track: String,
    },
    /// Constant speed keeping the clip's material (TL-09); `ripple` moves the
    /// rest of the track, otherwise a longer clip stops at the next one.
    Speed {
        track: String,
        clip: String,
        speed: f64,
        ripple: bool,
    },
    /// Replace the clip's speed ramp; empty clears it.
    Ramp {
        track: String,
        clip: String,
        keys: Vec<SpeedKeyDto>,
    },
}

impl EditOp {
    /// The same edit on another track (and clip, for clip edits).
    fn retarget(&self, track: String, clip: String) -> EditOp {
        let mut op = self.clone();
        match &mut op {
            EditOp::RippleHead {
                track: t, clip: c, ..
            }
            | EditOp::RippleTail {
                track: t, clip: c, ..
            }
            | EditOp::Roll {
                track: t, clip: c, ..
            }
            | EditOp::Slip {
                track: t, clip: c, ..
            }
            | EditOp::Slide {
                track: t, clip: c, ..
            }
            | EditOp::Move {
                track: t, clip: c, ..
            }
            | EditOp::Speed {
                track: t, clip: c, ..
            }
            | EditOp::Ramp {
                track: t, clip: c, ..
            }
            | EditOp::Transition {
                track: t, clip: c, ..
            } => {
                *t = track;
                *c = clip;
            }
            EditOp::Blade { track: t, .. }
            | EditOp::Extract { track: t, .. }
            | EditOp::Lift { track: t, .. }
            | EditOp::CloseGaps { track: t } => *t = track,
            EditOp::Nest { .. } => {}
        }
        op
    }
}

/// A place an edit may snap to (TL-06).
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct SnapPointDto {
    pub t: f64,
    /// "start", "playhead", "edge" or "marker".
    pub kind: String,
}

impl Session {
    /// Where a drag may snap to: sequence start, playhead, every clip edge and
    /// marker, leaving out the clip being dragged.
    pub fn snap_points(&self, exclude: Option<String>) -> Result<Vec<SnapPointDto>, String> {
        let seq = self.first_sequence()?;
        // The dragged clip and, with linked selection, its partners move
        // together, so none of their edges are targets.
        let mut skip = Vec::new();
        if let Some(c) = exclude {
            let id = ClipId(parse_id(&c)?);
            let track = seq
                .tracks
                .iter()
                .find(|t| t.clip(id).is_some())
                .ok_or("clip not found")?;
            if self.linked_selection {
                skip.extend(
                    debut_timeline::linked(seq, track.id, id)
                        .into_iter()
                        .map(|(_, c)| c),
                );
            }
            skip.push(id);
        }
        Ok(
            debut_timeline::snap_targets(seq, seq.frame_rate.snap(self.playhead()), &skip)
                .into_iter()
                .map(|s| SnapPointDto {
                    t: secs(s.t),
                    kind: match s.kind {
                        debut_timeline::SnapKind::SequenceStart => "start",
                        debut_timeline::SnapKind::Playhead => "playhead",
                        debut_timeline::SnapKind::ClipEdge => "edge",
                        debut_timeline::SnapKind::Marker => "marker",
                    }
                    .into(),
                })
                .collect(),
        )
    }

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

    /// Apply a timeline edit. With linked selection on, clip edits, blades and
    /// ripple deletes also apply to the clip's partners on other tracks (TL-05),
    /// all in one undo step.
    pub fn edit(&mut self, op: EditOp) -> Result<(), String> {
        let ops = if self.linked_selection {
            self.with_partners(op)?
        } else {
            vec![op]
        };
        let mut cmds = Vec::with_capacity(ops.len());
        for op in ops {
            cmds.push(self.edit_command(op)?);
        }
        let cmd = if cmds.len() == 1 {
            cmds.pop().unwrap()
        } else {
            Command::Group(cmds)
        };
        self.exec(cmd)
    }

    /// `op` plus the same edit retargeted at each linked partner.
    fn with_partners(&self, op: EditOp) -> Result<Vec<EditOp>, String> {
        let seq = self.first_sequence()?;
        let fr = seq.frame_rate;
        let track_of = |t: &str| parse_id(t).map(TrackId);
        let partners = match &op {
            EditOp::RippleHead { track, clip, .. }
            | EditOp::RippleTail { track, clip, .. }
            | EditOp::Roll { track, clip, .. }
            | EditOp::Slip { track, clip, .. }
            | EditOp::Slide { track, clip, .. }
            | EditOp::Move { track, clip, .. }
            | EditOp::Speed { track, clip, .. }
            | EditOp::Ramp { track, clip, .. } => {
                debut_timeline::linked(seq, track_of(track)?, ClipId(parse_id(clip)?))
            }
            EditOp::Blade { track, at } => {
                let t = track_of(track)?;
                debut_timeline::clip_at(seq, t, frames_of(*at, fr))
                    .map(|c| debut_timeline::linked(seq, t, c))
                    .unwrap_or_default()
            }
            EditOp::Extract { track, start, end } | EditOp::Lift { track, start, end } => {
                let t = track_of(track)?;
                debut_timeline::clip_spanning(seq, t, frames_of(*start, fr), frames_of(*end, fr))
                    .map(|c| debut_timeline::linked(seq, t, c))
                    .unwrap_or_default()
            }
            _ => Vec::new(),
        };
        let mut out = Vec::with_capacity(partners.len() + 1);
        for (t, c) in partners {
            out.push(op.retarget(id_str(t.0), id_str(c.0)));
        }
        out.insert(0, op);
        Ok(out)
    }

    pub fn linked_selection(&self) -> bool {
        self.linked_selection
    }

    pub fn set_linked_selection(&mut self, on: bool) {
        self.linked_selection = on;
    }

    fn edit_command(&mut self, op: EditOp) -> Result<Command, String> {
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
            EditOp::CloseGaps { track } => debut_timeline::close_gaps(project, target(&track)?),
            EditOp::Speed {
                track,
                clip: c,
                speed,
                ripple,
            } => {
                if !speed.is_finite() {
                    return Err("speed must be a number".into());
                }
                // Thousandths are plenty for a speed and keep the rationals small.
                let speed = Rational::new((speed * 1000.0).round() as i64, 1000);
                debut_timeline::set_speed(project, target(&track)?, clip(&c)?, speed, ripple)
            }
            EditOp::Ramp {
                track,
                clip: c,
                keys,
            } => debut_timeline::set_ramp(
                project,
                target(&track)?,
                clip(&c)?,
                keys.iter()
                    .map(|k| SpeedKey {
                        at: frames_of(k.at, fr),
                        speed: k.speed,
                    })
                    .collect(),
            ),
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
        Ok(cmd)
    }
}
