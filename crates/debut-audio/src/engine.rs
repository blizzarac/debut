//! The audio engine: renders timeline audio ahead of the playhead into a ring, and
//! the real-time sink that drains it, advances the master [`Clock`] and never
//! glitches — an empty ring plays silence and counts an underrun (PB-02, AUD).
//!
//! ```text
//!  UI/decode thread                     real-time thread
//!  AudioRenderer::fill_ahead  --ring-->  RtSink::fill  --> device
//!         |                                   |
//!    SampleSource (decoders)             Clock::advance
//! ```

use crate::clock::Clock;
use crate::effects::{build, Processor};
use crate::graph::{mix_into, TrackMix};
use crate::ring::{ring, Consumer, Producer};
use debut_core::{MediaId, Rational, Result};
use debut_platform::audio_out::AudioCallback;
use debut_project::AudioEffect;
use debut_project::{ClipSource, Sequence, TrackKind};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub const CHANNELS: usize = 2;

/// A track's insert description and the processors built from it.
type Chain = (Vec<AudioEffect>, Vec<Box<dyn Processor>>);

/// Stateful insert chains per track, rebuilt when a track's description changes.
#[derive(Default)]
pub struct Inserts {
    chains: HashMap<debut_core::TrackId, Chain>,
}

impl Inserts {
    /// Make the chains match `seq`'s audio tracks; unchanged chains keep their state.
    pub fn sync(&mut self, seq: &Sequence, sample_rate: u32) {
        for t in seq.tracks.iter().filter(|t| t.kind == TrackKind::Audio) {
            let stale = self
                .chains
                .get(&t.id)
                .is_none_or(|(desc, _)| *desc != t.audio_effects);
            if stale {
                let procs = t
                    .audio_effects
                    .iter()
                    .map(|e| build(e, sample_rate))
                    .collect();
                self.chains.insert(t.id, (t.audio_effects.clone(), procs));
            }
        }
        self.chains
            .retain(|id, _| seq.tracks.iter().any(|t| t.id == *id));
    }

    pub fn reset(&mut self) {
        for (_, procs) in self.chains.values_mut() {
            procs.iter_mut().for_each(|p| p.reset());
        }
    }

    fn process(&mut self, track: debut_core::TrackId, buf: &mut [f32]) {
        if let Some((_, procs)) = self.chains.get_mut(&track) {
            for p in procs {
                p.process(buf);
            }
        }
    }
}

/// Decoded audio for one media, at the engine sample rate.
pub trait SampleSource {
    /// Write `frames` frames of `media` starting at source time `start` into `out`
    /// (interleaved, native channel count, which is returned). Silence past the end.
    fn read(
        &mut self,
        media: MediaId,
        start: Rational,
        frames: usize,
        out: &mut Vec<f32>,
    ) -> Result<u16>;
}

/// Producer side. Owns the ring's write end and knows where the next sample goes.
pub struct AudioRenderer {
    clock: Arc<Clock>,
    producer: Producer,
    /// Timeline sample index of the next frame to render.
    write_pos: i64,
    /// Rate the queued audio was rendered at; a change flushes the ring.
    rate: Rational,
    mixes: HashMap<debut_core::TrackId, TrackMix>,
    inserts: Inserts,
    scratch: Vec<f32>,
    block: Vec<f32>,
}

/// Consumer side, moved into the device callback.
pub struct RtSink {
    consumer: Consumer,
    clock: Arc<Clock>,
    underruns: Arc<AtomicU64>,
}

impl AudioRenderer {
    /// `capacity_frames` of look-ahead (e.g. 0.25 s = 12 000 frames at 48 kHz).
    pub fn new(clock: Arc<Clock>, capacity_frames: usize) -> (Self, RtSink) {
        let (producer, consumer) = ring(capacity_frames * CHANNELS);
        let underruns = Arc::new(AtomicU64::new(0));
        let renderer = Self {
            clock: Arc::clone(&clock),
            producer,
            write_pos: 0,
            rate: Rational::ONE,
            mixes: HashMap::new(),
            inserts: Inserts::default(),
            scratch: Vec::new(),
            block: Vec::new(),
        };
        (
            renderer,
            RtSink {
                consumer,
                clock,
                underruns,
            },
        )
    }

    pub fn set_track_mix(&mut self, track: debut_core::TrackId, mix: TrackMix) {
        self.mixes.insert(track, mix);
    }

    /// Discard queued audio and continue from the clock's current position.
    /// Call after a seek or a rate change.
    pub fn resync(&mut self) {
        self.inserts.reset();
        self.producer.clear();
        self.write_pos = self.clock.position_samples();
        self.rate = self.clock.rate();
    }

    /// Frames queued ahead of the device.
    pub fn queued_frames(&self) -> usize {
        self.producer.len() / CHANNELS
    }

    /// Render until the ring holds at least `target_frames`, in blocks of
    /// `block_frames`. Returns the number of frames rendered.
    pub fn fill_ahead(
        &mut self,
        seq: &Sequence,
        source: &mut dyn SampleSource,
        target_frames: usize,
        block_frames: usize,
    ) -> Result<usize> {
        if self.clock.rate() != self.rate {
            self.resync();
        }
        let mut rendered = 0;
        while self.queued_frames() < target_frames
            && self.producer.free() >= block_frames * CHANNELS
        {
            self.render_block(seq, source, block_frames)?;
            let written = self.producer.push(&self.block);
            debug_assert_eq!(written, self.block.len());
            rendered += block_frames;
        }
        Ok(rendered)
    }

    /// Mix `frames` output frames at the current rate into `self.block`.
    fn render_block(
        &mut self,
        seq: &Sequence,
        source: &mut dyn SampleSource,
        frames: usize,
    ) -> Result<()> {
        let sr = self.clock.sample_rate() as i64;
        let rate = self.rate;
        // Timeline span this block covers, in samples (may run backwards).
        let span = (Rational::from_int(frames as i64) * rate).round();
        let (start, end) = if span >= 0 {
            (self.write_pos, self.write_pos + span)
        } else {
            (self.write_pos + span, self.write_pos)
        };
        let src_frames = (end - start).max(1) as usize;

        self.scratch.clear();
        self.scratch.resize(src_frames * CHANNELS, 0.0);
        self.inserts.sync(seq, sr as u32);
        render_span(
            seq,
            &self.mixes,
            &mut self.inserts,
            source,
            start,
            src_frames,
            sr as u32,
            &mut self.scratch,
        )?;

        // Resample the timeline span to `frames` output frames at `rate`
        // (naive linear; a proper varispeed resampler replaces this).
        self.block.clear();
        self.block.resize(frames * CHANNELS, 0.0);
        let step = rate.as_f64();
        let origin = if span >= 0 {
            0.0
        } else {
            src_frames as f64 - 1.0
        };
        for i in 0..frames {
            let p = origin + i as f64 * step;
            let p0 = p.floor().clamp(0.0, src_frames as f64 - 1.0) as usize;
            let p1 = (p0 + 1).min(src_frames - 1);
            let f = (p - p0 as f64).clamp(0.0, 1.0) as f32;
            for c in 0..CHANNELS {
                let s0 = self.scratch[p0 * CHANNELS + c];
                let s1 = self.scratch[p1 * CHANNELS + c];
                self.block[i * CHANNELS + c] = s0 + (s1 - s0) * f;
            }
        }
        self.write_pos += span;
        Ok(())
    }
}

/// Mix every audible audio track of `seq` over `[start, start + frames)` timeline
/// samples (at `sample_rate`) into the stereo `bus`, which must already be
/// `frames * CHANNELS` long and zeroed. Each track is summed into its own buffer,
/// run through its inserts, then added to the bus. Shared by playback and export.
#[allow(clippy::too_many_arguments)]
pub fn render_span(
    seq: &Sequence,
    mixes: &HashMap<debut_core::TrackId, TrackMix>,
    inserts: &mut Inserts,
    source: &mut dyn SampleSource,
    start: i64,
    frames: usize,
    sample_rate: u32,
    bus: &mut [f32],
) -> Result<()> {
    debug_assert_eq!(bus.len(), frames * CHANNELS);
    let sr = sample_rate as i64;
    let end = start + frames as i64;
    let any_solo = seq
        .tracks
        .iter()
        .any(|t| mixes.get(&t.id).is_some_and(|m| m.solo));
    let mut clip_buf = Vec::new();
    let mut track_buf = vec![0.0f32; frames * CHANNELS];
    for track in seq.tracks.iter().filter(|t| t.kind == TrackKind::Audio) {
        let mix = mixes.get(&track.id).copied().unwrap_or_default();
        if !mix.audible(any_solo) {
            continue;
        }
        track_buf.fill(0.0);
        let mut touched = false;
        for clip in &track.clips {
            let cin = (clip.timeline_in * Rational::from_int(sr)).round();
            let cout = (clip.timeline_out() * Rational::from_int(sr)).round();
            let (a, b) = (cin.max(start), cout.min(end));
            if b <= a {
                continue;
            }
            let media = match &clip.source {
                ClipSource::Media(m) => *m,
                ClipSource::Multicam { angles, active } => match angles.get(*active) {
                    Some(m) => *m,
                    None => continue,
                },
                ClipSource::Sequence(_) => continue,
            };
            let n = (b - a) as usize;
            let t = Rational::new(a, sr);
            let ch = source.read(media, clip.source_at(t), n, &mut clip_buf)?;
            let off = (a - start) as usize;
            mix_into(
                &mut track_buf[off * CHANNELS..(off + n) * CHANNELS],
                &clip_buf,
                ch,
                &mix,
            );
            touched = true;
        }
        if touched || !track.audio_effects.is_empty() {
            inserts.process(track.id, &mut track_buf);
            for (b, t) in bus.iter_mut().zip(&track_buf) {
                *b += *t;
            }
        }
    }
    Ok(())
}

impl RtSink {
    pub fn underruns(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.underruns)
    }
}

impl AudioCallback for RtSink {
    fn fill(&mut self, buffer: &mut [f32], channels: u16, _sample_rate: u32) {
        let frames = buffer.len() / channels.max(1) as usize;
        if !self.clock.is_playing() {
            buffer.fill(0.0);
            return;
        }
        let got = if channels as usize == CHANNELS {
            self.consumer.pop(buffer)
        } else {
            // Device isn't stereo: pop stereo frames and spread/fold per frame.
            let mut n = 0;
            let mut pair = [0.0; CHANNELS];
            for frame in buffer.chunks_exact_mut(channels as usize) {
                if self.consumer.pop(&mut pair) < CHANNELS {
                    break;
                }
                frame[0] = pair[0];
                if channels > 1 {
                    frame[1] = pair[1];
                }
                for s in frame.iter_mut().skip(2) {
                    *s = 0.0;
                }
                n += channels as usize;
            }
            n
        };
        if got < buffer.len() {
            buffer[got..].fill(0.0);
            self.underruns.fetch_add(1, Ordering::Relaxed);
        }
        self.clock.advance(frames as u64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::{FrameRate, IdGen, TrackId};
    use debut_project::{Clip, Track};

    /// Every media is a mono ramp: sample value = source sample index.
    struct Ramp;
    impl SampleSource for Ramp {
        fn read(
            &mut self,
            _: MediaId,
            start: Rational,
            frames: usize,
            out: &mut Vec<f32>,
        ) -> Result<u16> {
            let s0 = (start * Rational::from_int(48_000)).round();
            out.clear();
            out.extend((0..frames).map(|i| (s0 + i as i64) as f32));
            Ok(1)
        }
    }

    fn seq(ids: &mut IdGen) -> (Sequence, TrackId) {
        let mut seq = Sequence::new(ids.fresh(), "a", FrameRate::FPS_25, 16, 9);
        let mut track = Track::new(ids.fresh(), TrackKind::Audio);
        // One clip from 1 s to 2 s, source offset 10 s.
        track.clips.push(Clip::new(
            ids.fresh(),
            ClipSource::Media(ids.fresh()),
            Rational::from_int(1),
            Rational::from_int(1),
            Rational::from_int(10),
        ));
        let id = track.id;
        seq.tracks.push(track);
        (seq, id)
    }

    #[test]
    fn renders_clip_audio_in_place_and_silence_elsewhere() {
        let mut ids = IdGen::new(5);
        let (seq, _) = seq(&mut ids);
        let clock = Clock::new(48_000);
        let (mut r, mut sink) = AudioRenderer::new(Arc::clone(&clock), 4096);
        // Start 100 samples before the clip.
        clock.seek(Rational::new(48_000 - 100, 48_000));
        clock.play();
        r.resync();
        r.fill_ahead(&seq, &mut Ramp, 1000, 256).unwrap();

        let mut out = vec![0.0; 2 * 300];
        sink.fill(&mut out, 2, 48_000);
        // First 100 frames silent, then the ramp from source sample 480 000 (10 s),
        // panned centre (equal power => 1/sqrt2 each side).
        assert!(out[..200].iter().all(|s| *s == 0.0));
        let c = std::f32::consts::FRAC_1_SQRT_2;
        assert!((out[200] - 480_000.0 * c).abs() < 0.5, "{}", out[200]);
        assert!((out[201] - 480_000.0 * c).abs() < 0.5);
        assert!((out[2 * 299] - 480_199.0 * c).abs() < 0.5);
        assert_eq!(sink.underruns.load(Ordering::Relaxed), 0);
        assert_eq!(clock.position_samples(), 48_000 - 100 + 300);
    }

    #[test]
    fn underrun_plays_silence_and_is_counted_but_clock_still_advances() {
        let clock = Clock::new(48_000);
        let (_r, mut sink) = AudioRenderer::new(Arc::clone(&clock), 64);
        clock.play();
        let mut out = vec![1.0; 20];
        sink.fill(&mut out, 2, 48_000);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(sink.underruns.load(Ordering::Relaxed), 1);
        assert_eq!(clock.position_samples(), 10);
    }

    #[test]
    fn paused_outputs_silence_without_consuming() {
        let mut ids = IdGen::new(6);
        let (seq, _) = seq(&mut ids);
        let clock = Clock::new(48_000);
        let (mut r, mut sink) = AudioRenderer::new(Arc::clone(&clock), 4096);
        clock.seek(Rational::from_int(1));
        r.resync();
        r.fill_ahead(&seq, &mut Ramp, 512, 256).unwrap();
        let queued = r.queued_frames();
        let mut out = vec![1.0; 64];
        sink.fill(&mut out, 2, 48_000);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(r.queued_frames(), queued);
    }

    #[test]
    fn mute_and_gain_apply_and_rate_change_resyncs() {
        let mut ids = IdGen::new(7);
        let (seq, track) = seq(&mut ids);
        let clock = Clock::new(48_000);
        let (mut r, mut sink) = AudioRenderer::new(Arc::clone(&clock), 4096);
        clock.seek(Rational::from_int(1));
        clock.play();
        r.set_track_mix(
            track,
            TrackMix {
                gain: 0.5,
                pan: -1.0,
                ..Default::default()
            },
        );
        r.resync();
        r.fill_ahead(&seq, &mut Ramp, 512, 256).unwrap();
        let mut out = vec![0.0; 4];
        sink.fill(&mut out, 2, 48_000);
        assert!(
            (out[0] - 480_000.0 * 0.5).abs() < 0.5 && out[1].abs() < 1e-3,
            "{out:?}"
        );

        // Reverse at 2x from the middle of the clip: rendered backwards from the clock.
        clock.seek(Rational::new(3, 2));
        clock.set_rate(Rational::from_int(-2));
        let pos = clock.position_samples();
        r.fill_ahead(&seq, &mut Ramp, 512, 256).unwrap();
        let mut out = vec![0.0; 8];
        sink.fill(&mut out, 2, 48_000);
        let expect0 = (480_000 + (pos - 48_000)) as f32 * 0.5;
        assert!((out[0] - expect0).abs() < 1.5, "{} vs {expect0}", out[0]);
        assert!(out[2] < out[0], "reverse playback should descend: {out:?}");
        assert!(
            (out[0] - out[2] - 1.0).abs() < 0.01,
            "2x: step of two source samples per frame: {out:?}"
        );
    }

    #[test]
    fn track_inserts_shape_the_track_before_the_bus() {
        use debut_project::{AudioEffect, EqBand, EqKind};
        let mut ids = IdGen::new(8);
        let (mut seq, track) = seq(&mut ids);
        // A low-pass at 100 Hz on a track playing a 4 kHz tone silences it.
        seq.tracks[0].audio_effects = vec![AudioEffect::Eq {
            bands: vec![EqBand {
                kind: EqKind::LowPass,
                frequency_hz: 100.0,
                gain_db: 0.0,
                q: 0.707,
            }],
        }];
        struct Tone;
        impl SampleSource for Tone {
            fn read(
                &mut self,
                _: MediaId,
                start: Rational,
                frames: usize,
                out: &mut Vec<f32>,
            ) -> Result<u16> {
                let s0 = (start * Rational::from_int(48_000)).round();
                out.clear();
                out.extend((0..frames).map(|i| {
                    ((s0 + i as i64) as f32 * 4000.0 * std::f32::consts::TAU / 48_000.0).sin()
                }));
                Ok(1)
            }
        }
        let mixes = HashMap::new();
        let mut inserts = Inserts::default();
        inserts.sync(&seq, 48_000);
        let mut bus = vec![0.0; 2 * 4800];
        render_span(
            &seq,
            &mixes,
            &mut inserts,
            &mut Tone,
            48_000 + 24_000,
            4800,
            48_000,
            &mut bus,
        )
        .unwrap();
        let peak = bus[2 * 2400..].iter().fold(0.0f32, |p, s| p.max(s.abs()));
        assert!(peak < 0.01, "filtered tone peak {peak}");
        // Dropping the insert restores the tone; sync picks the change up.
        seq.tracks[0].audio_effects.clear();
        inserts.sync(&seq, 48_000);
        let mut bus = vec![0.0; 2 * 4800];
        render_span(
            &seq,
            &mixes,
            &mut inserts,
            &mut Tone,
            48_000 + 24_000,
            4800,
            48_000,
            &mut bus,
        )
        .unwrap();
        let peak = bus.iter().fold(0.0f32, |p, s| p.max(s.abs()));
        assert!(peak > 0.6, "unfiltered tone peak {peak}");
        let _ = track;
    }
}
