//! Sequences, tracks and clip instances (TL-01, TL-02, TL-07, TL-08).

use debut_core::id::{ClipId, MediaId, SequenceId, TrackId};
use debut_core::{FrameRate, Rational};

use crate::effect::{Effect, Param};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sequence {
    pub id: SequenceId,
    pub name: String,
    pub frame_rate: FrameRate,
    pub width: u32,
    pub height: u32,
    pub tracks: Vec<Track>,
    /// Timeline markers in sequence time (TL-10).
    #[serde(default)]
    pub markers: Vec<crate::marker::Marker>,
    /// Captions, kept sorted by start (GFX-05).
    #[serde(default)]
    pub captions: Vec<crate::caption::Caption>,
    /// How captions are burned in (GFX-06).
    #[serde(default)]
    pub caption_settings: crate::caption::CaptionSettings,
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
            markers: Vec::new(),
            captions: Vec::new(),
            caption_settings: Default::default(),
        }
    }

    /// The caption showing at `t`, if any.
    pub fn caption_at(&self, t: Rational) -> Option<&crate::caption::Caption> {
        self.captions.iter().find(|c| c.contains(t))
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

/// What a track shows at one instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Layer<'a> {
    Single(&'a Clip),
    Transition {
        from: &'a Clip,
        to: &'a Clip,
        progress: f32,
    },
}

/// A track holds non-overlapping clips, kept sorted by `timeline_in`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub kind: TrackKind,
    pub clips: Vec<Clip>,
    /// Audio inserts, in order (AUD-05). Ignored on video tracks.
    #[serde(default)]
    pub audio_effects: Vec<crate::audio_fx::AudioEffect>,
    /// Mixer strip state (AUD-02).
    #[serde(default)]
    pub mix: TrackMix,
    /// Auto-ducking under another track (AUD-08). Ignored on video tracks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duck: Option<crate::audio_fx::Duck>,
}

/// Per-track mixer settings (AUD-02, AUD-03).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackMix {
    /// Fader in dB; 0 = unity.
    pub gain_db: f32,
    /// -1 = hard left, 0 = centre, +1 = hard right.
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
}

impl Default for TrackMix {
    fn default() -> Self {
        Self {
            gain_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
        }
    }
}

impl Track {
    pub fn new(id: TrackId, kind: TrackKind) -> Self {
        Self {
            id,
            kind,
            clips: Vec::new(),
            audio_effects: Vec::new(),
            mix: TrackMix::default(),
            duck: None,
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

    /// What plays at `t`: a single clip, or two clips mid-transition with the
    /// dissolve progress in `0..1`.
    pub fn layer_at(&self, t: Rational) -> Option<Layer<'_>> {
        let i = self
            .clips
            .iter()
            .position(|c| c.timeline_in <= t && t < c.timeline_out())?;
        let cur = &self.clips[i];
        // Transition into `cur` from its predecessor, still in the first half?
        if let (Some(tr), Some(prev)) =
            (cur.transition_in, i.checked_sub(1).map(|p| &self.clips[p]))
        {
            if prev.timeline_out() == cur.timeline_in && t < cur.timeline_in + tr.half() {
                let start = cur.timeline_in - tr.half();
                return Some(Layer::Transition {
                    from: prev,
                    to: cur,
                    progress: ((t - start).as_f64() / tr.duration.as_f64()) as f32,
                });
            }
        }
        // Transition out of `cur` into its successor, already in the second half?
        if let Some(next) = self.clips.get(i + 1) {
            if let Some(tr) = next.transition_in {
                if next.timeline_in == cur.timeline_out() && t >= next.timeline_in - tr.half() {
                    let start = next.timeline_in - tr.half();
                    return Some(Layer::Transition {
                        from: cur,
                        to: next,
                        progress: ((t - start).as_f64() / tr.duration.as_f64()) as f32,
                    });
                }
            }
        }
        Some(Layer::Single(cur))
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
    /// 1 = normal speed, 0 = freeze frame, negative = reverse (TL-09).
    pub speed: Rational,
    /// Speed ramp in clip-local time; when present it replaces `speed`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ramp: Vec<crate::retime::SpeedKey>,
    /// Applied in order after the input color transform (FX-01, FX-02).
    #[serde(default)]
    pub effects: Vec<crate::effect::Effect>,
    /// Transition from the previous adjacent clip into this one, centred on the
    /// cut (FX-03). Both clips extend by half the duration into their handles.
    #[serde(default)]
    pub transition_in: Option<Transition>,
    /// Clip markers in clip-local time (TL-10).
    #[serde(default)]
    pub markers: Vec<crate::marker::Marker>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub kind: TransitionKind,
    pub duration: Rational,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    Dissolve,
}

impl Transition {
    pub fn half(&self) -> Rational {
        self.duration * Rational::new(1, 2)
    }
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
            ramp: Vec::new(),
            effects: Vec::new(),
            transition_in: None,
            markers: Vec::new(),
        }
    }

    pub fn timeline_out(&self) -> Rational {
        self.timeline_in + self.duration
    }

    /// Source time that plays at timeline time `t` (no bounds check).
    pub fn source_at(&self, t: Rational) -> Rational {
        if self.ramp.is_empty() {
            self.source_in + (t - self.timeline_in) * self.speed
        } else {
            let x = (t - self.timeline_in).as_f64();
            self.source_in + crate::retime::to_rational(crate::retime::offset(&self.ramp, x))
        }
    }

    /// `source_at` in seconds for clip-local `x`, without rationals; for
    /// per-sample audio resampling.
    pub fn source_offset_f64(&self, x: f64) -> f64 {
        if self.ramp.is_empty() {
            x * self.speed.as_f64()
        } else {
            crate::retime::offset(&self.ramp, x)
        }
    }

    /// Plays at anything other than constant normal speed.
    pub fn is_retimed(&self) -> bool {
        !self.ramp.is_empty() || self.speed != Rational::ONE
    }

    /// The media that plays at `t` and its source time, with the active
    /// multicam angle's offset applied. `None` for titles, nested sequences and
    /// multicam clips without a valid active angle.
    pub fn media_at(&self, t: Rational) -> Option<(MediaId, Rational)> {
        match &self.source {
            ClipSource::Media(m) => Some((*m, self.source_at(t))),
            ClipSource::Multicam {
                angles,
                active,
                offsets,
            } => {
                let m = *angles.get(*active)?;
                let off = offsets.get(*active).copied().unwrap_or(Rational::ZERO);
                Some((m, self.source_at(t) + off))
            }
            _ => None,
        }
    }

    pub fn source_out(&self) -> Rational {
        self.source_at(self.timeline_out())
    }

    /// Trim the head to start at `t` (keeps the tail in sync with the source).
    pub fn trim_head_to(&mut self, t: Rational) {
        debug_assert!(t >= self.timeline_in && t < self.timeline_out());
        self.set_head(t);
    }

    /// Move the head to `t` (earlier extends into the handle, later trims),
    /// keeping the tail and the source under it in place.
    pub fn set_head(&mut self, t: Rational) {
        self.source_in = self.source_at(t);
        crate::retime::rebase(&mut self.ramp, t - self.timeline_in);
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
        // Clip markers follow the material they sit on (clip-local times).
        let cut = t - self.timeline_in;
        self.markers.retain(|m| m.at < cut);
        tail.markers.retain(|m| m.at >= cut);
        for m in &mut tail.markers {
            m.at -= cut;
        }
        tail
    }

    /// Absorb `tail` (its material continues this clip): extend and re-base its
    /// markers onto this clip's local time.
    pub fn join(&mut self, tail: Clip) {
        let offset = self.duration;
        self.duration += tail.duration;
        self.markers.extend(tail.markers.into_iter().map(|mut m| {
            m.at += offset;
            m
        }));
        self.markers.sort_by_key(|m| m.at);
    }

    /// Evaluate a parameter of the `effect`-th effect at sequence time `t`.
    pub fn param_at(&self, effect: usize, p: Param, t: Rational) -> Option<f64> {
        self.effects.get(effect)?.value(p, t - self.timeline_in)
    }

    /// First effect of a kind, if any.
    pub fn effect_index(&self, kind: &str) -> Option<usize> {
        self.effects.iter().position(|e| e.kind() == kind)
    }

    pub fn effect(&self, kind: &str) -> Option<&Effect> {
        self.effect_index(kind).map(|i| &self.effects[i])
    }

    /// True when `next` is the uninterrupted continuation of `self` (same source,
    /// same speed, source and timeline both contiguous), so the two can be joined.
    pub fn is_continuous_with(&self, next: &Clip) -> bool {
        self.source == next.source
            && self.effects == next.effects
            && next.transition_in.is_none()
            && self.speed == next.speed
            && self.ramp.is_empty()
            && next.ramp.is_empty()
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
    /// `offsets[i]` is added to the source time of angle `i` so all angles line
    /// up (from audio sync or slate); missing entries mean zero.
    Multicam {
        angles: Vec<MediaId>,
        active: usize,
        #[serde(default)]
        offsets: Vec<Rational>,
    },
    /// Generated text clip (GFX-01); rendered by the engine's title cache.
    Title(crate::title::Title),
    /// Generated vector shape (GFX-03); rendered by the engine's graphics cache.
    Shape(crate::shape::Shape),
}
