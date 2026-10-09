//! Sequences, tracks and clip instances (TL-01, TL-02, TL-07, TL-08).

use debut_core::id::{ClipId, MediaId, SequenceId, TrackId};
use debut_core::{FrameRate, Rational};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sequence {
    pub id: SequenceId,
    pub name: String,
    pub frame_rate: FrameRate,
    pub width: u32,
    pub height: u32,
    pub tracks: Vec<Track>,
}

impl Sequence {
    pub fn new(
        id: SequenceId,
        name: impl Into<String>,
        frame_rate: FrameRate,
        width: u32,
        height: u32,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            frame_rate,
            width,
            height,
            tracks: Vec::new(),
        }
    }

    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_mut(&mut self, id: TrackId) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    /// End of the last clip on any track.
    pub fn duration(&self) -> Rational {
        self.tracks
            .iter()
            .map(Track::duration)
            .fold(Rational::ZERO, Rational::max)
    }
}

/// A track holds non-overlapping clips, kept sorted by `timeline_in`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub kind: TrackKind,
    pub clips: Vec<Clip>,
}

impl Track {
    pub fn new(id: TrackId, kind: TrackKind) -> Self {
        Self {
            id,
            kind,
            clips: Vec::new(),
        }
    }

    pub fn clip(&self, id: ClipId) -> Option<&Clip> {
        self.clips.iter().find(|c| c.id == id)
    }

    pub fn clip_index(&self, id: ClipId) -> Option<usize> {
        self.clips.iter().position(|c| c.id == id)
    }

    /// The clip whose span contains `t` (start inclusive, end exclusive).
    pub fn clip_at(&self, t: Rational) -> Option<&Clip> {
        self.clips
            .iter()
            .find(|c| c.timeline_in <= t && t < c.timeline_out())
    }

    pub fn duration(&self) -> Rational {
        self.clips
            .last()
            .map(Clip::timeline_out)
            .unwrap_or(Rational::ZERO)
    }

    /// Restore the sort invariant after a bulk edit.
    pub fn sort(&mut self) {
        self.clips.sort_by_key(|c| c.timeline_in);
    }

    /// True if no two clips overlap.
    pub fn is_consistent(&self) -> bool {
        self.clips
            .windows(2)
            .all(|w| w[0].timeline_out() <= w[1].timeline_in)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackKind {
    Video,
    Audio,
    /// Applies effects to all tracks below (FX-07).
    Adjustment,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    pub source: ClipSource,
    pub timeline_in: Rational,
    pub duration: Rational,
    pub source_in: Rational,
    /// 1 = normal speed; ramps live in the effect stack (TL-09).
    pub speed: Rational,
}

impl Clip {
    pub fn new(
        id: ClipId,
        source: ClipSource,
        timeline_in: Rational,
        duration: Rational,
        source_in: Rational,
    ) -> Self {
        Self {
            id,
            source,
            timeline_in,
            duration,
            source_in,
            speed: Rational::ONE,
        }
    }

    pub fn timeline_out(&self) -> Rational {
        self.timeline_in + self.duration
    }

    /// Source time that plays at timeline time `t` (no bounds check).
    pub fn source_at(&self, t: Rational) -> Rational {
        self.source_in + (t - self.timeline_in) * self.speed
    }

    pub fn source_out(&self) -> Rational {
        self.source_at(self.timeline_out())
    }

    /// Trim the head to start at `t` (keeps the tail in sync with the source).
    pub fn trim_head_to(&mut self, t: Rational) {
        debug_assert!(t >= self.timeline_in && t < self.timeline_out());
        self.source_in = self.source_at(t);
        self.duration = self.timeline_out() - t;
        self.timeline_in = t;
    }

    /// Trim the tail to end at `t`.
    pub fn trim_tail_to(&mut self, t: Rational) {
        debug_assert!(t > self.timeline_in && t <= self.timeline_out());
        self.duration = t - self.timeline_in;
    }

    /// Split at `t`, which must lie strictly inside; `self` keeps the head and the
    /// returned clip is the tail with `tail_id`.
    pub fn split_at(&mut self, t: Rational, tail_id: ClipId) -> Clip {
        debug_assert!(t > self.timeline_in && t < self.timeline_out());
        let mut tail = self.clone();
        tail.id = tail_id;
        tail.trim_head_to(t);
        self.trim_tail_to(t);
        tail
    }

    /// True when `next` is the uninterrupted continuation of `self` (same source,
    /// same speed, source and timeline both contiguous), so the two can be joined.
    pub fn is_continuous_with(&self, next: &Clip) -> bool {
        self.source == next.source
            && self.speed == next.speed
            && self.timeline_out() == next.timeline_in
            && self.source_out() == next.source_in
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClipSource {
    Media(MediaId),
    /// Nested sequence / compound clip (TL-07).
    Sequence(SequenceId),
    /// Multicam clip; `active` is the currently switched angle (MED-11, TL-08).
    Multicam {
        angles: Vec<MediaId>,
        active: usize,
    },
}
