//! Edit primitives and their inverses.
//!
//! Four track-level primitives cover the TL-03 core edits:
//! - [`Command::Replace`]: clear a range (trimming or splitting what straddles it)
//!   and place clips. Overwrite and lift are this.
//! - [`Command::Shift`]: move every clip at or after a point by a delta. Ripple.
//! - [`Command::Blade`] / [`Command::Join`]: cut a clip or re-merge a continuous pair.
//! - [`Command::Group`]: several primitives as one undo step.
//!
//! Insert and extract are groups built by the constructors at the bottom.

use debut_core::id::{BinId, CaptionId, MarkerId};
use debut_core::Curve;
use debut_core::{ClipId, Error, IdGen, MediaId, Rational, Result, SequenceId, TrackId};
use debut_project::media_ref::MediaRef;
use debut_project::{
    AudioEffect, Clip, ClipSource, Effect, Marker, Param, Project, Sequence, Track, TrackMix,
    Transition,
};
use debut_project::{Bin, Caption, CaptionSettings};
use serde::{Deserialize, Serialize};

/// Where a marker lives: on the sequence, or on one clip (clip-local time).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkerTarget {
    pub sequence: SequenceId,
    pub clip: Option<(TrackId, ClipId)>,
}

/// Which track a primitive operates on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub sequence: SequenceId,
    pub track: TrackId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// Clear `[start, end)` plus the spans of `clips`, then place `clips`.
    /// `split_id` names the tail if a single clip has to be split around the range.
    Replace {
        target: Target,
        start: Rational,
        end: Rational,
        clips: Vec<Clip>,
        split_id: ClipId,
    },
    /// Move every clip whose `timeline_in >= from` by `by`. The caller guarantees no
    /// clip straddles `from` and that a negative shift creates no overlap.
    Shift {
        target: Target,
        from: Rational,
        by: Rational,
    },
    /// Split the clip strictly containing `at`; the tail gets `tail_id`. No-op if
    /// no clip contains `at`.
    Blade {
        target: Target,
        at: Rational,
        tail_id: ClipId,
    },
    /// Merge the two clips meeting at `at` if the second continues the first. No-op
    /// otherwise. The first clip's ID survives.
    Join {
        target: Target,
        at: Rational,
    },
    /// Move a clip's head by `delta` keeping its tail and source in sync
    /// (positive shortens). The clip must stay non-empty and non-overlapping.
    TrimHead {
        target: Target,
        clip: ClipId,
        delta: Rational,
    },
    /// Move a clip's tail by `delta` (positive lengthens).
    TrimTail {
        target: Target,
        clip: ClipId,
        delta: Rational,
    },
    /// Change which part of the source plays without moving the clip (TL-04 slip).
    Slip {
        target: Target,
        clip: ClipId,
        delta: Rational,
    },
    /// Move a clip along the timeline by `delta`; nothing else moves.
    Move {
        target: Target,
        clip: ClipId,
        delta: Rational,
    },
    // ---- effects (FX-01, FX-02) --------------------------------------------------
    AddEffect {
        target: Target,
        clip: ClipId,
        effect: Effect,
        index: Option<usize>,
    },
    /// Swap the effect at `index` wholesale (shape, key colour and other
    /// non-animated options); keyframed values go through `SetParam`.
    ReplaceEffect {
        target: Target,
        clip: ClipId,
        index: usize,
        effect: Effect,
    },
    RemoveEffect {
        target: Target,
        clip: ClipId,
        index: usize,
    },
    /// Set a parameter at a keyframe (`at`, clip-local) or as a constant.
    SetParam {
        target: Target,
        clip: ClipId,
        effect: usize,
        param: Param,
        at: Option<Rational>,
        value: f64,
    },
    /// Replace a whole curve (the inverse of `SetParam`).
    SetCurve {
        target: Target,
        clip: ClipId,
        effect: usize,
        param: Param,
        curve: Curve,
    },
    /// Replace a track's audio insert chain (AUD-05).
    SetTrackAudio {
        target: Target,
        effects: Vec<AudioEffect>,
    },
    /// Set a track's fader/pan/mute/solo (AUD-02).
    SetTrackMix {
        target: Target,
        mix: TrackMix,
    },
    /// Replace what a clip plays (title text/style edits, relinks) in place.
    SetClipSource {
        target: Target,
        clip: ClipId,
        source: ClipSource,
    },
    // ---- markers (TL-10) ----------------------------------------------------------
    AddMarker {
        target: MarkerTarget,
        marker: Marker,
    },
    RemoveMarker {
        target: MarkerTarget,
        id: MarkerId,
    },
    /// Replace a marker's fields (matched by id).
    UpdateMarker {
        target: MarkerTarget,
        marker: Marker,
    },
    // ---- captions (GFX-05) ----
    AddCaption {
        sequence: SequenceId,
        caption: Caption,
    },
    RemoveCaption {
        sequence: SequenceId,
        id: CaptionId,
    },
    /// Replace a caption's fields (matched by id).
    UpdateCaption {
        sequence: SequenceId,
        caption: Caption,
    },
    SetCaptionSettings {
        sequence: SequenceId,
        settings: CaptionSettings,
    },
    /// Set or clear the transition into `clip` from its predecessor (FX-03).
    SetTransition {
        target: Target,
        clip: ClipId,
        transition: Option<Transition>,
    },
    // ---- bins (MED-07) ----
    AddBin(Bin),
    RemoveBin(BinId),
    RenameBin {
        id: BinId,
        name: String,
    },
    /// Put `media` in `bin` (a manual one), removing it from every other manual
    /// bin; `None` leaves it in no bin.
    AssignMedia {
        media: MediaId,
        bin: Option<BinId>,
    },
    /// Point a media at another file (relink, MED-05).
    SetMediaPath {
        media: MediaId,
        path: String,
    },
    /// Keywords and star rating of a media (MED-08).
    SetMediaTags {
        media: MediaId,
        keywords: Vec<String>,
        rating: u8,
    },
    // ---- project structure (MED-07, TL-01) ----------------------------------
    AddMedia(MediaRef),
    RemoveMedia(MediaId),
    AddSequence(Sequence),
    RemoveSequence(SequenceId),
    AddTrack {
        sequence: SequenceId,
        track: Track,
        index: Option<usize>,
    },
    RemoveTrack {
        sequence: SequenceId,
        track: TrackId,
    },
    /// One undo step made of several commands, applied in order.
    Group(Vec<Command>),
    /// Inverse of a primitive that did nothing.
    Noop,
}

fn captions_mut(project: &mut Project, s: SequenceId) -> Result<&mut Vec<Caption>> {
    Ok(&mut project
        .sequence_mut(s)
        .ok_or_else(|| Error::NotFound(format!("sequence {s:?}")))?
        .captions)
}

fn track_mut(project: &mut Project, t: Target) -> Result<&mut Track> {
    project
        .sequence_mut(t.sequence)
        .ok_or_else(|| Error::NotFound(format!("sequence {:?}", t.sequence)))?
        .track_mut(t.track)
        .ok_or_else(|| Error::NotFound(format!("track {:?}", t.track)))
}

fn track(project: &Project, t: Target) -> Result<&Track> {
    project
        .sequence(t.sequence)
        .ok_or_else(|| Error::NotFound(format!("sequence {:?}", t.sequence)))?
        .track(t.track)
        .ok_or_else(|| Error::NotFound(format!("track {:?}", t.track)))
}

/// `[start, end)` widened to cover every clip in `clips`.
fn span_with(start: Rational, end: Rational, clips: &[Clip]) -> (Rational, Rational) {
    clips.iter().fold((start, end), |(s, e), c| {
        (s.min(c.timeline_in), e.max(c.timeline_out()))
    })
}

/// Remove everything inside `[start, end)` from `track`, trimming clips that cross
/// an edge and splitting a clip that contains the whole range.
fn clear_range(track: &mut Track, start: Rational, end: Rational, split_id: ClipId) {
    if end <= start {
        return;
    }
    let mut kept = Vec::with_capacity(track.clips.len() + 1);
    for mut c in track.clips.drain(..) {
        let (cin, cout) = (c.timeline_in, c.timeline_out());
        if cout <= start || cin >= end {
            kept.push(c);
        } else if cin < start && cout > end {
            let mut tail = c.split_at(end, split_id);
            c.trim_tail_to(start);
            tail.trim_head_to(end);
            kept.push(c);
            kept.push(tail);
        } else if cin < start {
            c.trim_tail_to(start);
            kept.push(c);
        } else if cout > end {
            c.trim_head_to(end);
            kept.push(c);
        }
        // else: fully inside, dropped
    }
    track.clips = kept;
}

impl Command {
    pub fn apply(&self, project: &mut Project) -> Result<()> {
        match self {
            Command::Replace {
                target,
                start,
                end,
                clips,
                split_id,
            } => {
                let tr = track_mut(project, *target)?;
                let (s, e) = span_with(*start, *end, clips);
                let mut next = tr.clone();
                clear_range(&mut next, s, e, *split_id);
                next.clips.extend(clips.iter().cloned());
                next.sort();
                if !next.is_consistent() || next.clips.iter().any(|c| c.timeline_in.is_negative()) {
                    return Err(Error::InvalidArgument(
                        "replace produced an invalid layout".into(),
                    ));
                }
                tr.clips = next.clips;
                Ok(())
            }
            Command::Shift { target, from, by } => {
                let tr = track_mut(project, *target)?;
                if tr
                    .clips
                    .iter()
                    .any(|c| c.timeline_in < *from && c.timeline_out() > *from)
                {
                    return Err(Error::InvalidArgument(
                        "shift point lies inside a clip".into(),
                    ));
                }
                let mut next = tr.clone();
                for c in next.clips.iter_mut().filter(|c| c.timeline_in >= *from) {
                    c.timeline_in += *by;
                }
                if !next.is_consistent() || next.clips.iter().any(|c| c.timeline_in.is_negative()) {
                    return Err(Error::InvalidArgument(
                        "shift produced an invalid layout".into(),
                    ));
                }
                tr.clips = next.clips;
                Ok(())
            }
            Command::Blade {
                target,
                at,
                tail_id,
            } => {
                let tr = track_mut(project, *target)?;
                if let Some(i) = tr
                    .clips
                    .iter()
                    .position(|c| c.timeline_in < *at && *at < c.timeline_out())
                {
                    let tail = tr.clips[i].split_at(*at, *tail_id);
                    tr.clips.insert(i + 1, tail);
                }
                Ok(())
            }
            Command::Join { target, at } => {
                let tr = track_mut(project, *target)?;
                if let Some(i) = joinable_at(tr, *at) {
                    let tail = tr.clips.remove(i + 1);
                    tr.clips[i].join(tail);
                }
                Ok(())
            }
            Command::TrimHead {
                target,
                clip,
                delta,
            } => edit_clip(project, *target, *clip, |c| {
                let t = c.timeline_in + *delta;
                if t >= c.timeline_out() {
                    return Err(Error::InvalidArgument("trim would empty the clip".into()));
                }
                c.source_in = c.source_at(t);
                c.duration = c.timeline_out() - t;
                c.timeline_in = t;
                Ok(())
            }),
            Command::TrimTail {
                target,
                clip,
                delta,
            } => edit_clip(project, *target, *clip, |c| {
                if c.duration + *delta <= Rational::ZERO {
                    return Err(Error::InvalidArgument("trim would empty the clip".into()));
                }
                c.duration += *delta;
                Ok(())
            }),
            Command::Slip {
                target,
                clip,
                delta,
            } => edit_clip(project, *target, *clip, |c| {
                c.source_in += *delta;
                Ok(())
            }),
            Command::Move {
                target,
                clip,
                delta,
            } => edit_clip(project, *target, *clip, |c| {
                c.timeline_in += *delta;
                Ok(())
            }),
            Command::AddEffect {
                target,
                clip,
                effect,
                index,
            } => edit_clip(project, *target, *clip, |c| {
                let at = index.unwrap_or(c.effects.len()).min(c.effects.len());
                c.effects.insert(at, effect.clone());
                Ok(())
            }),
            Command::ReplaceEffect {
                target,
                clip,
                index,
                effect,
            } => edit_clip(project, *target, *clip, |c| {
                let slot = c
                    .effects
                    .get_mut(*index)
                    .ok_or_else(|| Error::InvalidArgument("no such effect".into()))?;
                *slot = effect.clone();
                Ok(())
            }),
            Command::RemoveEffect {
                target,
                clip,
                index,
            } => edit_clip(project, *target, *clip, |c| {
                if *index >= c.effects.len() {
                    return Err(Error::NotFound(format!("effect {index}")));
                }
                c.effects.remove(*index);
                Ok(())
            }),
            Command::SetParam {
                target,
                clip,
                effect,
                param,
                at,
                value,
            } => edit_clip(project, *target, *clip, |c| {
                let e = c
                    .effects
                    .get_mut(*effect)
                    .ok_or_else(|| Error::NotFound(format!("effect {effect}")))?;
                if !e.set(*param, *at, *value) {
                    return Err(Error::InvalidArgument(format!(
                        "{param:?} is not a parameter of {}",
                        e.kind()
                    )));
                }
                Ok(())
            }),
            Command::SetCurve {
                target,
                clip,
                effect,
                param,
                curve,
            } => edit_clip(project, *target, *clip, |c| {
                let e = c
                    .effects
                    .get_mut(*effect)
                    .ok_or_else(|| Error::NotFound(format!("effect {effect}")))?;
                let kind = e.kind();
                let slot = e.curve_mut(*param).ok_or_else(|| {
                    Error::InvalidArgument(format!("{param:?} is not a parameter of {kind}"))
                })?;
                *slot = curve.clone();
                Ok(())
            }),
            Command::SetTrackAudio { target, effects } => {
                track_mut(project, *target)?.audio_effects = effects.clone();
                Ok(())
            }
            Command::SetTrackMix { target, mix } => {
                track_mut(project, *target)?.mix = *mix;
                Ok(())
            }
            Command::SetClipSource {
                target,
                clip,
                source,
            } => edit_clip(project, *target, *clip, |c| {
                c.source = source.clone();
                Ok(())
            }),
            Command::AddMarker { target, marker } => {
                let list = markers_mut(project, *target)?;
                if list.iter().any(|m| m.id == marker.id) {
                    return Err(Error::InvalidArgument("marker id already exists".into()));
                }
                list.push(marker.clone());
                list.sort_by_key(|m| m.at);
                Ok(())
            }
            Command::RemoveMarker { target, id } => {
                let list = markers_mut(project, *target)?;
                let i = list
                    .iter()
                    .position(|m| m.id == *id)
                    .ok_or_else(|| Error::NotFound(format!("marker {id:?}")))?;
                list.remove(i);
                Ok(())
            }
            Command::AddCaption { sequence, caption } => {
                if caption.end <= caption.start {
                    return Err(Error::InvalidArgument(
                        "caption ends before it starts".into(),
                    ));
                }
                let list = captions_mut(project, *sequence)?;
                if list.iter().any(|c| c.id == caption.id) {
                    return Err(Error::InvalidArgument("caption id already exists".into()));
                }
                list.push(caption.clone());
                list.sort_by_key(|c| c.start);
                Ok(())
            }
            Command::RemoveCaption { sequence, id } => {
                let list = captions_mut(project, *sequence)?;
                let i = list
                    .iter()
                    .position(|c| c.id == *id)
                    .ok_or_else(|| Error::NotFound(format!("caption {id:?}")))?;
                list.remove(i);
                Ok(())
            }
            Command::UpdateCaption { sequence, caption } => {
                if caption.end <= caption.start {
                    return Err(Error::InvalidArgument(
                        "caption ends before it starts".into(),
                    ));
                }
                let list = captions_mut(project, *sequence)?;
                let slot = list
                    .iter_mut()
                    .find(|c| c.id == caption.id)
                    .ok_or_else(|| Error::NotFound(format!("caption {:?}", caption.id)))?;
                *slot = caption.clone();
                list.sort_by_key(|c| c.start);
                Ok(())
            }
            Command::SetCaptionSettings { sequence, settings } => {
                project
                    .sequence_mut(*sequence)
                    .ok_or_else(|| Error::NotFound(format!("sequence {sequence:?}")))?
                    .caption_settings = settings.clone();
                Ok(())
            }
            Command::UpdateMarker { target, marker } => {
                let list = markers_mut(project, *target)?;
                let slot = list
                    .iter_mut()
                    .find(|m| m.id == marker.id)
                    .ok_or_else(|| Error::NotFound(format!("marker {:?}", marker.id)))?;
                *slot = marker.clone();
                list.sort_by_key(|m| m.at);
                Ok(())
            }
            Command::SetTransition {
                target,
                clip,
                transition,
            } => {
                let tr = track_mut(project, *target)?;
                let i = tr
                    .clip_index(*clip)
                    .ok_or_else(|| Error::NotFound(format!("clip {clip:?}")))?;
                if let Some(t) = transition {
                    if t.duration <= Rational::ZERO {
                        return Err(Error::InvalidArgument(
                            "transition needs a positive duration".into(),
                        ));
                    }
                    let prev = i
                        .checked_sub(1)
                        .map(|p| &tr.clips[p])
                        .filter(|p| p.timeline_out() == tr.clips[i].timeline_in);
                    let Some(prev) = prev else {
                        return Err(Error::InvalidArgument(
                            "no adjacent clip before this one".into(),
                        ));
                    };
                    let c = &tr.clips[i];
                    if c.source_in < t.half() * c.speed {
                        return Err(Error::InvalidArgument(
                            "not enough head handle for the transition".into(),
                        ));
                    }
                    if t.duration > c.duration || t.duration > prev.duration {
                        return Err(Error::InvalidArgument(
                            "transition longer than a clip".into(),
                        ));
                    }
                }
                tr.clips[i].transition_in = *transition;
                Ok(())
            }
            Command::AddBin(b) => {
                if project.bins.iter().any(|x| x.id == b.id) {
                    return Err(Error::InvalidArgument("bin id already exists".into()));
                }
                project.bins.push(b.clone());
                Ok(())
            }
            Command::RemoveBin(id) => {
                let i = project
                    .bins
                    .iter()
                    .position(|b| b.id == *id)
                    .ok_or_else(|| Error::NotFound(format!("bin {id:?}")))?;
                project.bins.remove(i);
                Ok(())
            }
            Command::RenameBin { id, name } => {
                let b = project
                    .bins
                    .iter_mut()
                    .find(|b| b.id == *id)
                    .ok_or_else(|| Error::NotFound(format!("bin {id:?}")))?;
                b.name = name.clone();
                Ok(())
            }
            Command::AssignMedia { media, bin } => {
                if !project.media.iter().any(|m| m.id == *media) {
                    return Err(Error::NotFound(format!("media {media:?}")));
                }
                if let Some(b) = bin {
                    match project.bins.iter().find(|x| x.id == *b) {
                        None => return Err(Error::NotFound(format!("bin {b:?}"))),
                        Some(x) if x.is_smart() => {
                            return Err(Error::InvalidArgument(
                                "smart bins are rule-based; media cannot be assigned".into(),
                            ))
                        }
                        _ => {}
                    }
                }
                for b in project.bins.iter_mut().filter(|b| !b.is_smart()) {
                    b.items.retain(|m| m != media);
                    if Some(b.id) == *bin {
                        b.items.push(*media);
                    }
                }
                Ok(())
            }
            Command::SetMediaPath { media, path } => {
                let m = project
                    .media
                    .iter_mut()
                    .find(|m| m.id == *media)
                    .ok_or_else(|| Error::NotFound(format!("media {media:?}")))?;
                m.path = path.clone();
                m.online = true;
                Ok(())
            }
            Command::SetMediaTags {
                media,
                keywords,
                rating,
            } => {
                if *rating > 5 {
                    return Err(Error::InvalidArgument("rating is 0..=5".into()));
                }
                let m = project
                    .media
                    .iter_mut()
                    .find(|m| m.id == *media)
                    .ok_or_else(|| Error::NotFound(format!("media {media:?}")))?;
                m.keywords = keywords
                    .iter()
                    .map(|k| k.trim().to_string())
                    .filter(|k| !k.is_empty())
                    .collect();
                m.rating = *rating;
                Ok(())
            }
            Command::AddMedia(m) => {
                if project.media.iter().any(|x| x.id == m.id) {
                    return Err(Error::InvalidArgument("media id already exists".into()));
                }
                project.media.push(m.clone());
                Ok(())
            }
            Command::RemoveMedia(id) => {
                let i = project
                    .media
                    .iter()
                    .position(|m| m.id == *id)
                    .ok_or_else(|| Error::NotFound(format!("media {id:?}")))?;
                project.media.remove(i);
                Ok(())
            }
            Command::AddSequence(seq) => {
                if project.sequence(seq.id).is_some() {
                    return Err(Error::InvalidArgument("sequence id already exists".into()));
                }
                project.sequences.push(seq.clone());
                Ok(())
            }
            Command::RemoveSequence(id) => {
                let i = project
                    .sequences
                    .iter()
                    .position(|s| s.id == *id)
                    .ok_or_else(|| Error::NotFound(format!("sequence {id:?}")))?;
                project.sequences.remove(i);
                Ok(())
            }
            Command::AddTrack {
                sequence,
                track,
                index,
            } => {
                let seq = project
                    .sequence_mut(*sequence)
                    .ok_or_else(|| Error::NotFound(format!("sequence {sequence:?}")))?;
                if seq.track(track.id).is_some() {
                    return Err(Error::InvalidArgument("track id already exists".into()));
                }
                let at = index.unwrap_or(seq.tracks.len()).min(seq.tracks.len());
                seq.tracks.insert(at, track.clone());
                Ok(())
            }
            Command::RemoveTrack { sequence, track } => {
                let seq = project
                    .sequence_mut(*sequence)
                    .ok_or_else(|| Error::NotFound(format!("sequence {sequence:?}")))?;
                let i = seq
                    .tracks
                    .iter()
                    .position(|t| t.id == *track)
                    .ok_or_else(|| Error::NotFound(format!("track {track:?}")))?;
                seq.tracks.remove(i);
                Ok(())
            }
            Command::Group(cmds) => {
                for c in cmds {
                    c.apply(project)?;
                }
                Ok(())
            }
            Command::Noop => Ok(()),
        }
    }

    /// The command that undoes `self`, computed against `project` *before* `self`
    /// is applied.
    pub fn invert(&self, project: &Project) -> Result<Command> {
        match self {
            Command::Replace {
                target,
                start,
                end,
                clips,
                split_id,
            } => {
                let tr = track(project, *target)?;
                let (s, e) = span_with(*start, *end, clips);
                let originals: Vec<Clip> = tr
                    .clips
                    .iter()
                    .filter(|c| c.timeline_in < e && c.timeline_out() > s)
                    .cloned()
                    .collect();
                Ok(Command::Replace {
                    target: *target,
                    start: s,
                    end: e,
                    clips: originals,
                    split_id: *split_id,
                })
            }
            Command::Shift { target, from, by } => Ok(Command::Shift {
                target: *target,
                from: *from + *by,
                by: -*by,
            }),
            Command::Blade { target, at, .. } => {
                let tr = track(project, *target)?;
                let acts = tr
                    .clips
                    .iter()
                    .any(|c| c.timeline_in < *at && *at < c.timeline_out());
                Ok(if acts {
                    Command::Join {
                        target: *target,
                        at: *at,
                    }
                } else {
                    Command::Noop
                })
            }
            Command::Join { target, at } => {
                let tr = track(project, *target)?;
                Ok(match joinable_at(tr, *at) {
                    Some(i) => Command::Blade {
                        target: *target,
                        at: *at,
                        tail_id: tr.clips[i + 1].id,
                    },
                    None => Command::Noop,
                })
            }
            Command::TrimHead {
                target,
                clip,
                delta,
            } => Ok(Command::TrimHead {
                target: *target,
                clip: *clip,
                delta: -*delta,
            }),
            Command::TrimTail {
                target,
                clip,
                delta,
            } => Ok(Command::TrimTail {
                target: *target,
                clip: *clip,
                delta: -*delta,
            }),
            Command::Slip {
                target,
                clip,
                delta,
            } => Ok(Command::Slip {
                target: *target,
                clip: *clip,
                delta: -*delta,
            }),
            Command::Move {
                target,
                clip,
                delta,
            } => Ok(Command::Move {
                target: *target,
                clip: *clip,
                delta: -*delta,
            }),
            Command::AddEffect {
                target,
                clip,
                index,
                ..
            } => {
                let c = find_clip(project, *target, *clip)?;
                Ok(Command::RemoveEffect {
                    target: *target,
                    clip: *clip,
                    index: index.unwrap_or(c.effects.len()).min(c.effects.len()),
                })
            }
            Command::ReplaceEffect {
                target,
                clip,
                index,
                ..
            } => {
                let c = find_clip(project, *target, *clip)?;
                let e = c
                    .effects
                    .get(*index)
                    .ok_or_else(|| Error::InvalidArgument("no such effect".into()))?;
                Ok(Command::ReplaceEffect {
                    target: *target,
                    clip: *clip,
                    index: *index,
                    effect: e.clone(),
                })
            }
            Command::RemoveEffect {
                target,
                clip,
                index,
            } => {
                let c = find_clip(project, *target, *clip)?;
                let e = c
                    .effects
                    .get(*index)
                    .ok_or_else(|| Error::NotFound(format!("effect {index}")))?;
                Ok(Command::AddEffect {
                    target: *target,
                    clip: *clip,
                    effect: e.clone(),
                    index: Some(*index),
                })
            }
            Command::SetParam {
                target,
                clip,
                effect,
                param,
                ..
            }
            | Command::SetCurve {
                target,
                clip,
                effect,
                param,
                ..
            } => {
                let c = find_clip(project, *target, *clip)?;
                let e = c
                    .effects
                    .get(*effect)
                    .ok_or_else(|| Error::NotFound(format!("effect {effect}")))?;
                let curve = e.curve(*param).ok_or_else(|| {
                    Error::InvalidArgument(format!("{param:?} is not a parameter of {}", e.kind()))
                })?;
                Ok(Command::SetCurve {
                    target: *target,
                    clip: *clip,
                    effect: *effect,
                    param: *param,
                    curve: curve.clone(),
                })
            }
            Command::SetTrackAudio { target, .. } => Ok(Command::SetTrackAudio {
                target: *target,
                effects: track(project, *target)?.audio_effects.clone(),
            }),
            Command::SetTrackMix { target, .. } => Ok(Command::SetTrackMix {
                target: *target,
                mix: track(project, *target)?.mix,
            }),
            Command::SetClipSource { target, clip, .. } => Ok(Command::SetClipSource {
                target: *target,
                clip: *clip,
                source: find_clip(project, *target, *clip)?.source.clone(),
            }),
            Command::AddMarker { target, marker } => Ok(Command::RemoveMarker {
                target: *target,
                id: marker.id,
            }),
            Command::RemoveMarker { target, id }
            | Command::UpdateMarker {
                target,
                marker: Marker { id, .. },
            } => {
                let m = markers(project, *target)?
                    .iter()
                    .find(|m| m.id == *id)
                    .ok_or_else(|| Error::NotFound(format!("marker {id:?}")))?;
                Ok(match self {
                    Command::RemoveMarker { .. } => Command::AddMarker {
                        target: *target,
                        marker: m.clone(),
                    },
                    _ => Command::UpdateMarker {
                        target: *target,
                        marker: m.clone(),
                    },
                })
            }
            Command::AddCaption { sequence, caption } => Ok(Command::RemoveCaption {
                sequence: *sequence,
                id: caption.id,
            }),
            Command::RemoveCaption { sequence, id }
            | Command::UpdateCaption {
                sequence,
                caption: Caption { id, .. },
            } => {
                let c = project
                    .sequence(*sequence)
                    .ok_or_else(|| Error::NotFound(format!("sequence {sequence:?}")))?
                    .captions
                    .iter()
                    .find(|c| c.id == *id)
                    .ok_or_else(|| Error::NotFound(format!("caption {id:?}")))?;
                Ok(match self {
                    Command::RemoveCaption { .. } => Command::AddCaption {
                        sequence: *sequence,
                        caption: c.clone(),
                    },
                    _ => Command::UpdateCaption {
                        sequence: *sequence,
                        caption: c.clone(),
                    },
                })
            }
            Command::SetCaptionSettings { sequence, .. } => Ok(Command::SetCaptionSettings {
                sequence: *sequence,
                settings: project
                    .sequence(*sequence)
                    .ok_or_else(|| Error::NotFound(format!("sequence {sequence:?}")))?
                    .caption_settings
                    .clone(),
            }),
            Command::SetTransition { target, clip, .. } => Ok(Command::SetTransition {
                target: *target,
                clip: *clip,
                transition: find_clip(project, *target, *clip)?.transition_in,
            }),
            Command::AddBin(b) => Ok(Command::RemoveBin(b.id)),
            Command::RemoveBin(id) => project
                .bins
                .iter()
                .find(|b| b.id == *id)
                .map(|b| Command::AddBin(b.clone()))
                .ok_or_else(|| Error::NotFound(format!("bin {id:?}"))),
            Command::RenameBin { id, .. } => project
                .bins
                .iter()
                .find(|b| b.id == *id)
                .map(|b| Command::RenameBin {
                    id: *id,
                    name: b.name.clone(),
                })
                .ok_or_else(|| Error::NotFound(format!("bin {id:?}"))),
            Command::AssignMedia { media, .. } => Ok(Command::AssignMedia {
                media: *media,
                bin: project
                    .bins
                    .iter()
                    .find(|b| !b.is_smart() && b.items.contains(media))
                    .map(|b| b.id),
            }),
            Command::SetMediaPath { media, .. } => project
                .media
                .iter()
                .find(|m| m.id == *media)
                .map(|m| Command::SetMediaPath {
                    media: *media,
                    path: m.path.clone(),
                })
                .ok_or_else(|| Error::NotFound(format!("media {media:?}"))),
            Command::SetMediaTags { media, .. } => project
                .media
                .iter()
                .find(|m| m.id == *media)
                .map(|m| Command::SetMediaTags {
                    media: *media,
                    keywords: m.keywords.clone(),
                    rating: m.rating,
                })
                .ok_or_else(|| Error::NotFound(format!("media {media:?}"))),
            Command::AddMedia(m) => Ok(Command::RemoveMedia(m.id)),
            Command::RemoveMedia(id) => {
                let m = project
                    .media
                    .iter()
                    .find(|m| m.id == *id)
                    .ok_or_else(|| Error::NotFound(format!("media {id:?}")))?;
                Ok(Command::AddMedia(m.clone()))
            }
            Command::AddSequence(seq) => Ok(Command::RemoveSequence(seq.id)),
            Command::RemoveSequence(id) => {
                let s = project
                    .sequence(*id)
                    .ok_or_else(|| Error::NotFound(format!("sequence {id:?}")))?;
                Ok(Command::AddSequence(s.clone()))
            }
            Command::AddTrack {
                sequence, track, ..
            } => Ok(Command::RemoveTrack {
                sequence: *sequence,
                track: track.id,
            }),
            Command::RemoveTrack { sequence, track } => {
                let seq = project
                    .sequence(*sequence)
                    .ok_or_else(|| Error::NotFound(format!("sequence {sequence:?}")))?;
                let i = seq
                    .tracks
                    .iter()
                    .position(|t| t.id == *track)
                    .ok_or_else(|| Error::NotFound(format!("track {track:?}")))?;
                Ok(Command::AddTrack {
                    sequence: *sequence,
                    track: seq.tracks[i].clone(),
                    index: Some(i),
                })
            }
            Command::Group(cmds) => {
                let mut scratch = project.clone();
                let mut inverses = Vec::with_capacity(cmds.len());
                for c in cmds {
                    inverses.push(c.invert(&scratch)?);
                    c.apply(&mut scratch)?;
                }
                inverses.reverse();
                Ok(Command::Group(inverses))
            }
            Command::Noop => Ok(Command::Noop),
        }
    }

    // ---- TL-03 constructors -------------------------------------------------

    /// Overwrite: place `clip`, replacing whatever it covers. Nothing moves.
    pub fn overwrite(target: Target, clip: Clip, ids: &mut IdGen) -> Command {
        Command::Replace {
            target,
            start: clip.timeline_in,
            end: clip.timeline_out(),
            clips: vec![clip],
            split_id: ids.fresh(),
        }
    }

    /// Lift: clear `[start, end)`, leaving a gap.
    pub fn lift(target: Target, start: Rational, end: Rational, ids: &mut IdGen) -> Command {
        Command::Replace {
            target,
            start,
            end,
            clips: Vec::new(),
            split_id: ids.fresh(),
        }
    }

    /// Insert: open a gap at `at` long enough for `clips` (which are placed starting
    /// at `at`, in order) and ripple everything after it to the right.
    pub fn insert(target: Target, at: Rational, mut clips: Vec<Clip>, ids: &mut IdGen) -> Command {
        let mut cursor = at;
        for c in &mut clips {
            c.timeline_in = cursor;
            cursor += c.duration;
        }
        let len = cursor - at;
        Command::Group(vec![
            Command::Blade {
                target,
                at,
                tail_id: ids.fresh(),
            },
            Command::Shift {
                target,
                from: at,
                by: len,
            },
            Command::Replace {
                target,
                start: at,
                end: cursor,
                clips,
                split_id: ids.fresh(),
            },
        ])
    }

    /// Extract: remove `[start, end)` and ripple everything after it to the left.
    pub fn extract(target: Target, start: Rational, end: Rational, ids: &mut IdGen) -> Command {
        Command::Group(vec![
            Command::Blade {
                target,
                at: start,
                tail_id: ids.fresh(),
            },
            Command::Blade {
                target,
                at: end,
                tail_id: ids.fresh(),
            },
            Command::Replace {
                target,
                start,
                end,
                clips: Vec::new(),
                split_id: ids.fresh(),
            },
            Command::Shift {
                target,
                from: end,
                by: start - end,
            },
        ])
    }
}

fn markers(project: &Project, t: MarkerTarget) -> Result<&Vec<Marker>> {
    let seq = project
        .sequence(t.sequence)
        .ok_or_else(|| Error::NotFound(format!("sequence {:?}", t.sequence)))?;
    match t.clip {
        None => Ok(&seq.markers),
        Some((track, clip)) => Ok(&find_clip(
            project,
            Target {
                sequence: t.sequence,
                track,
            },
            clip,
        )?
        .markers),
    }
}

fn markers_mut(project: &mut Project, t: MarkerTarget) -> Result<&mut Vec<Marker>> {
    let seq = project
        .sequence_mut(t.sequence)
        .ok_or_else(|| Error::NotFound(format!("sequence {:?}", t.sequence)))?;
    match t.clip {
        None => Ok(&mut seq.markers),
        Some((track, clip)) => {
            let tr = seq
                .track_mut(track)
                .ok_or_else(|| Error::NotFound(format!("track {track:?}")))?;
            let i = tr
                .clip_index(clip)
                .ok_or_else(|| Error::NotFound(format!("clip {clip:?}")))?;
            Ok(&mut tr.clips[i].markers)
        }
    }
}

fn find_clip(project: &Project, target: Target, clip: ClipId) -> Result<&Clip> {
    track(project, target)?
        .clip(clip)
        .ok_or_else(|| Error::NotFound(format!("clip {clip:?}")))
}

/// Apply `f` to one clip, then re-check the track's layout invariants.
fn edit_clip(
    project: &mut Project,
    target: Target,
    clip: ClipId,
    f: impl FnOnce(&mut Clip) -> Result<()>,
) -> Result<()> {
    let tr = track_mut(project, target)?;
    let i = tr
        .clip_index(clip)
        .ok_or_else(|| Error::NotFound(format!("clip {clip:?}")))?;
    let mut next = tr.clone();
    f(&mut next.clips[i])?;
    if next.clips[i].timeline_in.is_negative() {
        return Err(Error::InvalidArgument(
            "clip would start before zero".into(),
        ));
    }
    next.sort();
    if !next.is_consistent() {
        return Err(Error::InvalidArgument(
            "edit produced overlapping clips".into(),
        ));
    }
    tr.clips = next.clips;
    Ok(())
}

/// Index of the first clip of a joinable pair meeting exactly at `at`.
fn joinable_at(track: &Track, at: Rational) -> Option<usize> {
    track
        .clips
        .windows(2)
        .position(|w| w[0].timeline_out() == at && w[0].is_continuous_with(&w[1]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::{FrameRate, MediaId};
    use debut_project::{ClipSource, Sequence, TrackKind};

    struct Fx {
        project: Project,
        target: Target,
        ids: IdGen,
        media: MediaId,
    }

    fn sec(n: i64) -> Rational {
        Rational::from_int(n)
    }

    /// One video track with clips A[0,10) B[10,20) C[25,30), each starting at source 0.
    fn fixture() -> Fx {
        let mut ids = IdGen::new(42);
        let mut project = Project::new(ids.fresh(), "fx");
        let mut seq = Sequence::new(ids.fresh(), "seq", FrameRate::FPS_25, 1920, 1080);
        let mut track = Track::new(ids.fresh(), TrackKind::Video);
        let media = ids.fresh();
        for (i, d) in [(0, 10), (10, 10), (25, 5)] {
            track.clips.push(Clip::new(
                ids.fresh(),
                ClipSource::Media(media),
                sec(i),
                sec(d),
                Rational::ZERO,
            ));
        }
        let target = Target {
            sequence: seq.id,
            track: track.id,
        };
        seq.tracks.push(track);
        project.sequences.push(seq);
        Fx {
            project,
            target,
            ids,
            media,
        }
    }

    impl Fx {
        fn clip(&mut self, at: i64, dur: i64, src: i64) -> Clip {
            Clip::new(
                self.ids.fresh(),
                ClipSource::Media(self.media),
                sec(at),
                sec(dur),
                sec(src),
            )
        }
        fn layout(&self) -> Vec<(i64, i64, i64)> {
            track(&self.project, self.target)
                .unwrap()
                .clips
                .iter()
                .map(|c| (c.timeline_in.num, c.timeline_out().num, c.source_in.num))
                .collect()
        }
        /// Apply, then undo via the computed inverse; assert the project is restored.
        fn apply_and_undo(&mut self, cmd: &Command, expected_after: &[(i64, i64, i64)]) {
            let before = self.project.clone();
            let inv = cmd.invert(&self.project).unwrap();
            cmd.apply(&mut self.project).unwrap();
            assert_eq!(self.layout(), expected_after, "layout after apply");
            assert!(track(&self.project, self.target).unwrap().is_consistent());
            inv.apply(&mut self.project).unwrap();
            assert_eq!(self.project, before, "project after undo");
        }
    }

    #[test]
    fn overwrite_in_the_middle_splits_the_clip() {
        let mut fx = fixture();
        let new = fx.clip(3, 4, 100);
        let cmd = Command::overwrite(fx.target, new, &mut fx.ids);
        fx.apply_and_undo(
            &cmd,
            &[(0, 3, 0), (3, 7, 100), (7, 10, 7), (10, 20, 0), (25, 30, 0)],
        );
    }

    #[test]
    fn overwrite_across_a_cut_trims_both_sides() {
        let mut fx = fixture();
        let new = fx.clip(8, 4, 0);
        let cmd = Command::overwrite(fx.target, new, &mut fx.ids);
        fx.apply_and_undo(&cmd, &[(0, 8, 0), (8, 12, 0), (12, 20, 2), (25, 30, 0)]);
    }

    #[test]
    fn overwrite_can_swallow_whole_clips_and_gaps() {
        let mut fx = fixture();
        let new = fx.clip(5, 25, 0);
        let cmd = Command::overwrite(fx.target, new, &mut fx.ids);
        fx.apply_and_undo(&cmd, &[(0, 5, 0), (5, 30, 0)]);
    }

    #[test]
    fn lift_leaves_a_gap() {
        let mut fx = fixture();
        let cmd = Command::lift(fx.target, sec(5), sec(15), &mut fx.ids);
        fx.apply_and_undo(&cmd, &[(0, 5, 0), (15, 20, 5), (25, 30, 0)]);
    }

    #[test]
    fn insert_at_a_cut_ripples_later_clips() {
        let mut fx = fixture();
        let new = fx.clip(0, 3, 50);
        let cmd = Command::insert(fx.target, sec(10), vec![new], &mut fx.ids);
        fx.apply_and_undo(&cmd, &[(0, 10, 0), (10, 13, 50), (13, 23, 0), (28, 33, 0)]);
    }

    #[test]
    fn insert_inside_a_clip_splits_and_ripples() {
        let mut fx = fixture();
        let new = fx.clip(0, 2, 50);
        let cmd = Command::insert(fx.target, sec(4), vec![new], &mut fx.ids);
        fx.apply_and_undo(
            &cmd,
            &[(0, 4, 0), (4, 6, 50), (6, 12, 4), (12, 22, 0), (27, 32, 0)],
        );
    }

    #[test]
    fn extract_closes_the_gap() {
        let mut fx = fixture();
        let cmd = Command::extract(fx.target, sec(5), sec(15), &mut fx.ids);
        fx.apply_and_undo(&cmd, &[(0, 5, 0), (5, 10, 5), (15, 20, 0)]);
    }

    #[test]
    fn extract_across_a_gap_pulls_in_the_tail() {
        let mut fx = fixture();
        let cmd = Command::extract(fx.target, sec(18), sec(27), &mut fx.ids);
        fx.apply_and_undo(&cmd, &[(0, 10, 0), (10, 18, 0), (18, 21, 2)]);
    }

    #[test]
    fn blade_then_join_restores_the_clip_id() {
        let mut fx = fixture();
        let original = track(&fx.project, fx.target).unwrap().clips[0].clone();
        let tail_id: ClipId = fx.ids.fresh();
        let blade = Command::Blade {
            target: fx.target,
            at: sec(4),
            tail_id,
        };
        fx.apply_and_undo(&blade, &[(0, 4, 0), (4, 10, 4), (10, 20, 0), (25, 30, 0)]);
        assert_eq!(track(&fx.project, fx.target).unwrap().clips[0], original);
    }

    #[test]
    fn join_does_not_merge_a_pre_existing_discontinuity() {
        // A[0,10) and B[10,20) both start at source 0, so B does not continue A.
        let mut fx = fixture();
        let join = Command::Join {
            target: fx.target,
            at: sec(10),
        };
        assert_eq!(join.invert(&fx.project).unwrap(), Command::Noop);
        fx.apply_and_undo(&join, &[(0, 10, 0), (10, 20, 0), (25, 30, 0)]);
    }

    #[test]
    fn blade_at_an_existing_cut_is_a_noop_whose_inverse_does_not_join() {
        let mut fx = fixture();
        // Make B a true continuation of A, then cut at 10 (already a cut).
        track_mut(&mut fx.project, fx.target).unwrap().clips[1].source_in = sec(10);
        let blade = Command::Blade {
            target: fx.target,
            at: sec(10),
            tail_id: fx.ids.fresh(),
        };
        assert_eq!(blade.invert(&fx.project).unwrap(), Command::Noop);
    }

    #[test]
    fn a_rejected_apply_leaves_the_project_untouched() {
        let mut fx = fixture();
        let before = fx.project.clone();
        let b = track(&fx.project, fx.target).unwrap().clips[1].id;
        assert!(Command::Move {
            target: fx.target,
            clip: b,
            delta: sec(6)
        }
        .apply(&mut fx.project)
        .is_err());
        assert!(Command::Shift {
            target: fx.target,
            from: sec(25),
            by: sec(-20)
        }
        .apply(&mut fx.project)
        .is_err());
        let big = fx.clip(-5, 3, 0);
        assert!(Command::overwrite(fx.target, big, &mut fx.ids)
            .apply(&mut fx.project)
            .is_err());
        assert_eq!(fx.project, before);
    }

    #[test]
    fn structure_commands_round_trip_through_undo() {
        let mut fx = fixture();
        let before = fx.project.clone();
        let media = MediaRef {
            id: fx.ids.fresh(),
            path: "a.mp4".into(),
            online: true,
            metadata: Default::default(),
            proxies: vec![],
            keywords: vec![],
            rating: 0,
        };
        let seq = Sequence::new(fx.ids.fresh(), "second", FrameRate::FPS_25, 16, 9);
        let track = Track::new(fx.ids.fresh(), TrackKind::Audio);
        let cmds = [
            Command::AddMedia(media.clone()),
            Command::AddSequence(seq.clone()),
            Command::AddTrack {
                sequence: fx.target.sequence,
                track: track.clone(),
                index: Some(0),
            },
        ];
        let mut inverses = Vec::new();
        for c in &cmds {
            inverses.push(c.invert(&fx.project).unwrap());
            c.apply(&mut fx.project).unwrap();
        }
        assert_eq!(fx.project.media.len(), 1);
        assert_eq!(fx.project.sequences.len(), 2);
        assert_eq!(fx.project.sequences[0].tracks[0].id, track.id);
        // Removing the middle track and undoing restores its position.
        let rm = Command::RemoveTrack {
            sequence: fx.target.sequence,
            track: track.id,
        };
        let undo_rm = rm.invert(&fx.project).unwrap();
        rm.apply(&mut fx.project).unwrap();
        undo_rm.apply(&mut fx.project).unwrap();
        assert_eq!(fx.project.sequences[0].tracks[0].id, track.id);
        assert!(
            Command::AddMedia(media).apply(&mut fx.project).is_err(),
            "duplicate id"
        );
        for inv in inverses.iter().rev() {
            inv.apply(&mut fx.project).unwrap();
        }
        assert_eq!(fx.project, before);
    }

    #[test]
    fn effect_commands_round_trip_through_undo() {
        use debut_project::{GradeFx, TransformFx};
        let mut fx = fixture();
        let before = fx.project.clone();
        let a = track(&fx.project, fx.target).unwrap().clips[0].id;
        let t = fx.target;
        let cmds = [
            Command::AddEffect {
                target: t,
                clip: a,
                effect: Effect::Transform(TransformFx::default()),
                index: None,
            },
            Command::AddEffect {
                target: t,
                clip: a,
                effect: Effect::Grade(GradeFx::default()),
                index: Some(0),
            },
            Command::SetParam {
                target: t,
                clip: a,
                effect: 1,
                param: Param::Opacity,
                at: None,
                value: 0.5,
            },
            Command::SetParam {
                target: t,
                clip: a,
                effect: 1,
                param: Param::Opacity,
                at: Some(sec(2)),
                value: 1.0,
            },
            Command::SetParam {
                target: t,
                clip: a,
                effect: 0,
                param: Param::Exposure,
                at: Some(sec(0)),
                value: -1.0,
            },
        ];
        let mut inverses = Vec::new();
        for c in &cmds {
            inverses.push(c.invert(&fx.project).unwrap());
            c.apply(&mut fx.project).unwrap();
        }
        let clip = track(&fx.project, fx.target).unwrap().clips[0].clone();
        assert_eq!(clip.effects[0].kind(), "grade");
        assert_eq!(
            clip.param_at(1, Param::Opacity, sec(1)),
            Some(0.75),
            "linear between the constant key and t=2"
        );
        assert_eq!(clip.param_at(0, Param::Exposure, sec(5)), Some(-1.0));
        assert!(Command::SetParam {
            target: t,
            clip: a,
            effect: 0,
            param: Param::Scale,
            at: None,
            value: 2.0
        }
        .apply(&mut fx.project)
        .is_err());
        assert!(Command::RemoveEffect {
            target: t,
            clip: a,
            index: 5
        }
        .apply(&mut fx.project)
        .is_err());
        for inv in inverses.iter().rev() {
            inv.apply(&mut fx.project).unwrap();
        }
        assert_eq!(fx.project, before);
    }

    #[test]
    fn transitions_need_an_adjacent_clip_and_handles() {
        use debut_project::{Transition, TransitionKind};
        let mut fx = fixture();
        let t = fx.target;
        let (a, b, c) = {
            let cl = &track(&fx.project, t).unwrap().clips;
            (cl[0].id, cl[1].id, cl[2].id)
        };
        let d = Transition {
            kind: TransitionKind::Dissolve,
            duration: sec(2),
        };
        // A has no predecessor; C's predecessor is not adjacent (gap 20..25); B has no head handle.
        assert!(Command::SetTransition {
            target: t,
            clip: a,
            transition: Some(d)
        }
        .apply(&mut fx.project)
        .is_err());
        assert!(Command::SetTransition {
            target: t,
            clip: c,
            transition: Some(d)
        }
        .apply(&mut fx.project)
        .is_err());
        assert!(Command::SetTransition {
            target: t,
            clip: b,
            transition: Some(d)
        }
        .apply(&mut fx.project)
        .is_err());
        // Slip B 1 s into its source: now it has a 1 s handle, enough for a 2 s dissolve.
        Command::Slip {
            target: t,
            clip: b,
            delta: sec(1),
        }
        .apply(&mut fx.project)
        .unwrap();
        let before = fx.project.clone();
        let set = Command::SetTransition {
            target: t,
            clip: b,
            transition: Some(d),
        };
        let inv = set.invert(&fx.project).unwrap();
        set.apply(&mut fx.project).unwrap();
        let tr = track(&fx.project, t).unwrap();
        assert_eq!(tr.clips[1].transition_in, Some(d));
        match tr.layer_at(sec(10)) {
            Some(debut_project::Layer::Transition { from, to, progress }) => {
                assert_eq!((from.id, to.id), (a, b));
                assert!((progress - 0.5).abs() < 1e-6);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(tr.layer_at(sec(8)), Some(debut_project::Layer::Single(c)) if c.id == a));
        assert!(
            matches!(tr.layer_at(sec(9)), Some(debut_project::Layer::Transition { progress, .. }) if progress == 0.0)
        );
        assert!(matches!(tr.layer_at(sec(12)), Some(debut_project::Layer::Single(c)) if c.id == b));
        inv.apply(&mut fx.project).unwrap();
        assert_eq!(fx.project, before);
    }

    #[test]
    fn blade_splits_clip_markers_and_join_merges_them_back() {
        let mut fx = fixture();
        let a = track(&fx.project, fx.target).unwrap().clips[0].id;
        let t = MarkerTarget {
            sequence: fx.target.sequence,
            clip: Some((fx.target.track, a)),
        };
        for at in [2, 7] {
            Command::AddMarker {
                target: t,
                marker: Marker::new(fx.ids.fresh(), sec(at), format!("m{at}")),
            }
            .apply(&mut fx.project)
            .unwrap();
        }
        let before = fx.project.clone();
        let blade = Command::Blade {
            target: fx.target,
            at: sec(4),
            tail_id: fx.ids.fresh(),
        };
        let join = blade.invert(&fx.project).unwrap();
        blade.apply(&mut fx.project).unwrap();
        let clips = &track(&fx.project, fx.target).unwrap().clips;
        assert_eq!(
            clips[0].markers.iter().map(|m| m.at).collect::<Vec<_>>(),
            vec![sec(2)]
        );
        assert_eq!(
            clips[1].markers.iter().map(|m| m.at).collect::<Vec<_>>(),
            vec![sec(3)],
            "re-based onto the tail"
        );
        join.apply(&mut fx.project).unwrap();
        assert_eq!(fx.project, before);
    }

    #[test]
    fn marker_commands_round_trip_and_stay_sorted() {
        let mut fx = fixture();
        let before = fx.project.clone();
        let seq_t = MarkerTarget {
            sequence: fx.target.sequence,
            clip: None,
        };
        let b = track(&fx.project, fx.target).unwrap().clips[1].id;
        let clip_t = MarkerTarget {
            sequence: fx.target.sequence,
            clip: Some((fx.target.track, b)),
        };
        let m1 = Marker::new(fx.ids.fresh(), sec(8), "late");
        let m2 = Marker::new(fx.ids.fresh(), sec(2), "early");
        let m3 = Marker::new(fx.ids.fresh(), sec(1), "on clip B");
        let cmds = [
            Command::AddMarker {
                target: seq_t,
                marker: m1.clone(),
            },
            Command::AddMarker {
                target: seq_t,
                marker: m2.clone(),
            },
            Command::AddMarker {
                target: clip_t,
                marker: m3.clone(),
            },
            Command::UpdateMarker {
                target: seq_t,
                marker: Marker {
                    note: "renamed".into(),
                    at: sec(9),
                    ..m2.clone()
                },
            },
            Command::RemoveMarker {
                target: seq_t,
                id: m1.id,
            },
        ];
        let mut inverses = Vec::new();
        for c in &cmds {
            inverses.push(c.invert(&fx.project).unwrap());
            c.apply(&mut fx.project).unwrap();
        }
        let seq = fx.project.sequence(fx.target.sequence).unwrap();
        assert_eq!(
            seq.markers
                .iter()
                .map(|m| m.note.as_str())
                .collect::<Vec<_>>(),
            vec!["renamed"]
        );
        assert_eq!(seq.markers[0].at, sec(9));
        assert_eq!(
            track(&fx.project, fx.target).unwrap().clips[1]
                .markers
                .len(),
            1
        );
        assert!(
            Command::AddMarker {
                target: seq_t,
                marker: m2.clone()
            }
            .apply(&mut fx.project)
            .is_err(),
            "duplicate id"
        );
        for inv in inverses.iter().rev() {
            inv.apply(&mut fx.project).unwrap();
        }
        assert_eq!(fx.project, before);
    }

    #[test]
    fn commands_round_trip_through_json() {
        let mut fx = fixture();
        let new = fx.clip(0, 2, 50);
        let cmd = Command::insert(fx.target, sec(4), vec![new], &mut fx.ids);
        let json = serde_json::to_string(&cmd).unwrap();
        let back: Command = serde_json::from_str(&json).unwrap();
        assert_eq!(cmd, back);
    }
}
