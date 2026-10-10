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
use crate::ducking::Ducker;
use crate::effects::{build, Processor};
use crate::graph::{mix_into, TrackMix};
use crate::ring::{ring, Consumer, Producer};
use debut_core::{MediaId, Rational, Result, SequenceId};
use debut_platform::audio_out::AudioCallback;
use debut_platform::PluginHost;
use debut_project::AudioEffect;
use debut_project::{Clip, ClipSource, Sequence, TrackKind};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub const CHANNELS: usize = 2;

/// A track's insert description and the processors built from it.
type Chain = (Vec<AudioEffect>, Vec<Box<dyn Processor>>);

/// Stateful insert chains per track, rebuilt when a track's description changes.
#[derive(Default)]
pub struct Inserts {
    chains: HashMap<debut_core::TrackId, Chain>,
    /// Auto-ducking state per ducked track (AUD-08).
    duckers: HashMap<debut_core::TrackId, Ducker>,
    /// Tracks of nested sequences seen while rendering; their chains survive
    /// `sync` of the top-level sequence.
    nested: HashSet<debut_core::TrackId>,
    /// Where CLAP inserts run (AUD-09); without it they pass audio through.
    plugins: Option<Arc<dyn PluginHost>>,
}

impl Inserts {
    /// The plugin host CLAP inserts run in. Chains holding a plugin insert
    /// are rebuilt when it changes.
    pub fn set_plugins(&mut self, host: Option<Arc<dyn PluginHost>>) {
        let same = match (&self.plugins, &host) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.plugins = host;
            self.chains.retain(|_, (desc, _)| {
                !desc.iter().any(|e| matches!(e, AudioEffect::Plugin { .. }))
            });
        }
    }

    /// Make the chains match `seq`'s audio tracks; unchanged chains keep their state.
    pub fn sync(&mut self, seq: &Sequence, sample_rate: u32) {
        self.sync_tracks(seq, sample_rate);
        let nested = &self.nested;
        self.chains
            .retain(|id, _| nested.contains(id) || seq.tracks.iter().any(|t| t.id == *id));
        self.duckers
            .retain(|id, _| nested.contains(id) || seq.tracks.iter().any(|t| t.id == *id));
    }

    /// Chains for a nested sequence's tracks, kept across top-level syncs.
    fn sync_nested(&mut self, seq: &Sequence, sample_rate: u32) {
        self.sync_tracks(seq, sample_rate);
        self.nested.extend(
            seq.tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Audio)
                .map(|t| t.id),
        );
    }

    fn sync_tracks(&mut self, seq: &Sequence, sample_rate: u32) {
        for t in seq.tracks.iter().filter(|t| t.kind == TrackKind::Audio) {
            let stale = self
                .chains
                .get(&t.id)
                .is_none_or(|(desc, _)| *desc != t.audio_effects);
            if stale {
                let procs = t
                    .audio_effects
                    .iter()
                    .map(|e| match e {
                        AudioEffect::Plugin { .. } => {
                            crate::plugin::build(e, sample_rate, self.plugins.clone())
                        }
                        _ => build(e, sample_rate),
                    })
                    .collect();
                self.chains.insert(t.id, (t.audio_effects.clone(), procs));
            }
            match t.duck {
                Some(d) if self.duckers.get(&t.id).is_none_or(|x| *x.settings() != d) => {
                    self.duckers.insert(t.id, Ducker::new(d, sample_rate));
                }
                Some(_) => {}
                None => {
                    self.duckers.remove(&t.id);
                }
            }
        }
    }

    pub fn reset(&mut self) {
        for (_, procs) in self.chains.values_mut() {
            procs.iter_mut().for_each(|p| p.reset());
        }
        self.duckers.values_mut().for_each(Ducker::reset);
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

    /// Resolve a nested sequence (TL-07) so its audio can be mixed in place of a
    /// compound clip. `None` renders the clip silent.
    fn sequence(&self, _id: SequenceId) -> Option<Arc<Sequence>> {
        None
    }

    /// Where CLAP track inserts run (AUD-09); `None` passes them through.
    fn plugins(&self) -> Option<Arc<dyn PluginHost>> {
        None
    }
}

/// Nesting depth after which compound clips render silent (matches the video side).
pub const MAX_NESTING: usize = 8;

/// Producer side. Owns the ring's write end and knows where the next sample goes.
pub struct AudioRenderer {
    clock: Arc<Clock>,
    producer: Producer,
    /// Timeline sample index of the next frame to render.
    write_pos: i64,
    /// Rate the queued audio was rendered at; a change flushes the ring.
    rate: Rational,
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
        self.inserts.set_plugins(source.plugins());
        self.inserts.sync(seq, sr as u32);
        render_span(
            seq,
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
    inserts: &mut Inserts,
    source: &mut dyn SampleSource,
    start: i64,
    frames: usize,
    sample_rate: u32,
    bus: &mut [f32],
) -> Result<()> {
    render_span_depth(seq, inserts, source, start, frames, sample_rate, bus, 0)
}

#[allow(clippy::too_many_arguments)]
fn render_span_depth(
    seq: &Sequence,
    inserts: &mut Inserts,
    source: &mut dyn SampleSource,
    start: i64,
    frames: usize,
    sample_rate: u32,
    bus: &mut [f32],
    depth: usize,
) -> Result<()> {
    debug_assert_eq!(bus.len(), frames * CHANNELS);
    let sr = sample_rate as i64;
    let end = start + frames as i64;
    let any_solo = seq
        .tracks
        .iter()
        .any(|t| t.kind == TrackKind::Audio && t.mix.solo);
    let mut clip_buf = Vec::new();
    let mut retimed = Vec::new();
    // Each audible track's post-fader, post-insert block; summed after
    // ducking, which needs every key track's level first.
    let mut rendered: Vec<(debut_core::TrackId, Vec<f32>)> = Vec::new();
    for track in seq.tracks.iter().filter(|t| t.kind == TrackKind::Audio) {
        let mix = TrackMix::from(track.mix);
        if !mix.audible(any_solo) {
            continue;
        }
        let mut track_buf = vec![0.0f32; frames * CHANNELS];
        let mut touched = false;
        for clip in &track.clips {
            let cin = (clip.timeline_in * Rational::from_int(sr)).round();
            let cout = (clip.timeline_out() * Rational::from_int(sr)).round();
            let (a, b) = (cin.max(start), cout.min(end));
            if b <= a {
                continue;
            }
            let n = (b - a) as usize;
            let t = Rational::new(a, sr);
            let off = (a - start) as usize;
            let ch = match &clip.source {
                ClipSource::Media(_) | ClipSource::Multicam { .. } => match clip.media_at(t) {
                    Some((m, _)) if clip.is_retimed() => {
                        read_retimed(source, clip, m, t, n, sr, &mut retimed, &mut clip_buf)?
                    }
                    Some((m, st)) => source.read(m, st, n, &mut clip_buf)?,
                    None => continue,
                },
                ClipSource::Sequence(id) => {
                    if depth >= MAX_NESTING {
                        continue;
                    }
                    let Some(nested) = source.sequence(*id) else {
                        continue;
                    };
                    inserts.sync_nested(&nested, sample_rate);
                    clip_buf.clear();
                    clip_buf.resize(n * CHANNELS, 0.0);
                    let nested_start = (clip.source_at(t) * Rational::from_int(sr)).round();
                    render_span_depth(
                        &nested,
                        inserts,
                        source,
                        nested_start,
                        n,
                        sample_rate,
                        &mut clip_buf,
                        depth + 1,
                    )?;
                    CHANNELS as u16
                }
                ClipSource::Title(_) => continue,
            };
            mix_into(
                &mut track_buf[off * CHANNELS..(off + n) * CHANNELS],
                &clip_buf,
                ch,
                &mix,
            );
            touched = true;
        }
        if touched || !track.audio_effects.is_empty() || track.duck.is_some() {
            inserts.process(track.id, &mut track_buf);
            rendered.push((track.id, track_buf));
        }
    }
    for track in seq.tracks.iter().filter(|t| t.duck.is_some()) {
        let Some(i) = rendered.iter().position(|(id, _)| *id == track.id) else {
            continue;
        };
        let Some(ducker) = inserts.duckers.get_mut(&track.id) else {
            continue;
        };
        let key_id = ducker.settings().key;
        // Key tracks are read as rendered (post-fader); a silent or muted key
        // simply never ducks.
        let mut buf = std::mem::take(&mut rendered[i].1);
        let key = rendered
            .iter()
            .find(|(id, _)| *id == key_id)
            .map(|(_, b)| b.as_slice());
        ducker.process(key, &mut buf, CHANNELS);
        rendered[i].1 = buf;
    }
    for (_, buf) in &rendered {
        for (b, t) in bus.iter_mut().zip(buf) {
            *b += *t;
        }
    }
    Ok(())
}

/// Read `n` samples of a retimed clip starting at timeline `t` into `out`
/// (TL-09): the source span the speed curve covers is read once, then each
/// output sample interpolates at its own source position. Pitch follows speed
/// (varispeed); a frozen clip is silent.
#[allow(clippy::too_many_arguments)]
fn read_retimed(
    source: &mut dyn SampleSource,
    clip: &Clip,
    media: MediaId,
    t: Rational,
    n: usize,
    sr: i64,
    span: &mut Vec<f32>,
    out: &mut Vec<f32>,
) -> Result<u16> {
    // Source seconds at the clip's start (multicam offset included).
    let Some((_, base)) = clip.media_at(clip.timeline_in) else {
        return Ok(1);
    };
    let base = base.as_f64();
    let local0 = (t - clip.timeline_in).as_f64();
    let rate = sr as f64;
    let pos: Vec<f64> = (0..n)
        .map(|i| (base + clip.source_offset_f64(local0 + i as f64 / rate)) * rate)
        .collect();
    let lo = pos.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = pos.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    out.clear();
    if n == 0 || hi - lo < 1e-6 {
        out.resize(n, 0.0);
        return Ok(1);
    }
    let first = lo.floor() as i64;
    let count = (hi.ceil() as i64 - first + 2) as usize;
    let ch = source.read(media, Rational::new(first, sr), count, span)?;
    let c = ch.max(1) as usize;
    let avail = span.len() / c;
    if avail == 0 {
        out.resize(n * c, 0.0);
        return Ok(ch);
    }
    out.reserve(n * c);
    for p in pos {
        let x = (p - first as f64).max(0.0);
        let i0 = (x.floor() as usize).min(avail - 1);
        let i1 = (i0 + 1).min(avail - 1);
        let f = (x - i0 as f64).clamp(0.0, 1.0) as f32;
        for k in 0..c {
            let (a, b) = (span[i0 * c + k], span[i1 * c + k]);
            out.push(a + (b - a) * f);
        }
    }
    Ok(ch)
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
    fn retimed_clips_resample_their_source() {
        let mut ids = IdGen::new(9);
        let (mut seq, _) = seq(&mut ids);
        let c = std::f32::consts::FRAC_1_SQRT_2;
        let render = |seq: &Sequence, at: i64| {
            let mut bus = vec![0.0; 2 * 4];
            let mut inserts = Inserts::default();
            render_span(seq, &mut inserts, &mut Ramp, at, 4, 48_000, &mut bus).unwrap();
            bus.iter().step_by(2).map(|s| s / c).collect::<Vec<f32>>()
        };
        // Double speed: 100 samples into the clip plays source sample 200.
        seq.tracks[0].clips[0].speed = Rational::from_int(2);
        let got = render(&seq, 48_100);
        for (i, v) in got.iter().enumerate() {
            assert!((v - (480_200.0 + 2.0 * i as f32)).abs() < 0.5, "{got:?}");
        }
        // Reverse: plays down from the source in-point.
        seq.tracks[0].clips[0].speed = Rational::from_int(-1);
        let got = render(&seq, 48_100);
        assert!(
            (got[0] - 479_900.0).abs() < 0.5 && got[1] < got[0],
            "{got:?}"
        );
        // Freeze is silent.
        seq.tracks[0].clips[0].speed = Rational::ZERO;
        assert!(render(&seq, 48_100).iter().all(|v| *v == 0.0));
        // A ramp at a constant 1/2: source advances half a sample per sample.
        let clip = &mut seq.tracks[0].clips[0];
        clip.speed = Rational::ONE;
        clip.ramp = vec![debut_project::SpeedKey {
            at: Rational::ZERO,
            speed: 0.5,
        }];
        let got = render(&seq, 48_100);
        assert!((got[0] - 480_050.0).abs() < 0.5 && (got[1] - got[0] - 0.5).abs() < 1e-3);
    }

    /// Constant-level media: the id's low bits pick the level.
    struct Level;
    impl SampleSource for Level {
        fn read(&mut self, m: MediaId, _: Rational, n: usize, out: &mut Vec<f32>) -> Result<u16> {
            let v = if m.0.is_multiple_of(2) { 0.5 } else { 0.25 };
            out.clear();
            out.resize(n, v);
            Ok(1)
        }
    }

    #[test]
    fn music_ducks_under_the_voice_track() {
        let mut ids = IdGen::new(11);
        let mut seq = Sequence::new(ids.fresh(), "d", FrameRate::FPS_25, 16, 9);
        let mut voice = Track::new(ids.fresh(), TrackKind::Audio);
        let mut music = Track::new(ids.fresh(), TrackKind::Audio);
        // Voice (0.25) from 1 s to 2 s; music (0.5) from 0 to 4 s.
        voice.clips.push(Clip::new(
            ids.fresh(),
            ClipSource::Media(MediaId(1)),
            Rational::from_int(1),
            Rational::from_int(1),
            Rational::ZERO,
        ));
        music.clips.push(Clip::new(
            ids.fresh(),
            ClipSource::Media(MediaId(2)),
            Rational::ZERO,
            Rational::from_int(4),
            Rational::ZERO,
        ));
        music.duck = Some(debut_project::Duck::under(voice.id));
        seq.tracks.push(voice);
        seq.tracks.push(music);
        let c = std::f32::consts::FRAC_1_SQRT_2;
        let mut inserts = Inserts::default();
        inserts.sync(&seq, 48_000);
        // Render 0..4 s in 1024-frame blocks and sample the left channel.
        let mut left = Vec::new();
        let mut pos = 0i64;
        while pos < 4 * 48_000 {
            let mut bus = vec![0.0; 1024 * CHANNELS];
            render_span(&seq, &mut inserts, &mut Level, pos, 1024, 48_000, &mut bus).unwrap();
            left.extend(bus.iter().step_by(2).map(|s| s / c));
            pos += 1024;
        }
        let at = |t: f64| left[(t * 48_000.0) as usize];
        assert!((at(0.5) - 0.5).abs() < 1e-4, "music alone at full level");
        // Late in the voice: voice + music 12 dB down.
        let ducked = 0.25 + 0.5 * 10f32.powf(-12.0 / 20.0);
        assert!((at(1.9) - ducked).abs() < 0.01, "{}", at(1.9));
        assert!((at(3.9) - 0.5).abs() < 0.01, "recovered: {}", at(3.9));
        // Muting the voice removes the key: nothing ducks.
        seq.tracks[0].mix.mute = true;
        let mut inserts = Inserts::default();
        inserts.sync(&seq, 48_000);
        let mut bus = vec![0.0; 1024 * CHANNELS];
        for p in (0..2 * 48_000).step_by(1024) {
            bus.fill(0.0);
            render_span(&seq, &mut inserts, &mut Level, p, 1024, 48_000, &mut bus).unwrap();
        }
        assert!((bus[0] / c - 0.5).abs() < 1e-4);
    }

    /// Ramp media plus one nested sequence.
    struct RampWith(Arc<Sequence>);
    impl SampleSource for RampWith {
        fn read(
            &mut self,
            m: MediaId,
            start: Rational,
            frames: usize,
            out: &mut Vec<f32>,
        ) -> Result<u16> {
            Ramp.read(m, start, frames, out)
        }
        fn sequence(&self, id: SequenceId) -> Option<Arc<Sequence>> {
            (self.0.id == id).then(|| Arc::clone(&self.0))
        }
    }

    #[test]
    fn compound_clip_mixes_the_nested_sequence_audio() {
        let mut ids = IdGen::new(6);
        // Inner: ramp clip 1..2 s at source 10 s. Outer: inner placed at 5 s,
        // starting from inner time 0.5 s, so inner's clip starts at outer 5.5 s.
        let (inner, _) = seq(&mut ids);
        let mut outer = Sequence::new(ids.fresh(), "outer", FrameRate::FPS_25, 16, 9);
        let mut track = Track::new(ids.fresh(), TrackKind::Audio);
        track.clips.push(Clip::new(
            ids.fresh(),
            ClipSource::Sequence(inner.id),
            Rational::from_int(5),
            Rational::from_int(2),
            Rational::new(1, 2),
        ));
        outer.tracks.push(track);
        let mut source = RampWith(Arc::new(inner.clone()));
        let mut inserts = Inserts::default();
        inserts.sync(&outer, 48_000);

        let start = 48_000 * 5 + 24_000 - 10; // 10 samples before inner's clip
        let mut bus = vec![0.0; 2 * 30];
        render_span(
            &outer,
            &mut inserts,
            &mut source,
            start,
            30,
            48_000,
            &mut bus,
        )
        .unwrap();
        assert!(bus[..20].iter().all(|s| *s == 0.0));
        // Inner clip starts at source 10 s = sample 480 000. The mono ramp is
        // centre-panned once on the inner track (1/sqrt2 per side); the outer
        // track passes the stereo result through at unity.
        let c = std::f32::consts::FRAC_1_SQRT_2;
        assert!((bus[20] - 480_000.0 * c).abs() < 0.5, "{}", bus[20]);
        assert!((bus[2 * 29] - 480_019.0 * c).abs() < 0.5);

        // Without a resolver the compound clip is silent, not an error.
        let mut bus = vec![0.0; 2 * 30];
        render_span(&outer, &mut inserts, &mut Ramp, start, 30, 48_000, &mut bus).unwrap();
        assert!(bus.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn self_nesting_audio_terminates() {
        let mut ids = IdGen::new(7);
        let mut seq = Sequence::new(ids.fresh(), "loop", FrameRate::FPS_25, 16, 9);
        let mut track = Track::new(ids.fresh(), TrackKind::Audio);
        track.clips.push(Clip::new(
            ids.fresh(),
            ClipSource::Sequence(seq.id),
            Rational::ZERO,
            Rational::from_int(10),
            Rational::ZERO,
        ));
        seq.tracks.push(track);
        let mut source = RampWith(Arc::new(seq.clone()));
        let mut inserts = Inserts::default();
        let mut bus = vec![0.0; 2 * 16];
        render_span(&seq, &mut inserts, &mut source, 100, 16, 48_000, &mut bus).unwrap();
        assert!(bus.iter().all(|s| *s == 0.0));
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
        let (mut seq, track) = seq(&mut ids);
        let clock = Clock::new(48_000);
        let (mut r, mut sink) = AudioRenderer::new(Arc::clone(&clock), 4096);
        clock.seek(Rational::from_int(1));
        clock.play();
        seq.tracks.iter_mut().find(|t| t.id == track).unwrap().mix = debut_project::TrackMix {
            gain_db: -6.0206,
            pan: -1.0,
            mute: false,
            solo: false,
        };
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
        let mut inserts = Inserts::default();
        inserts.sync(&seq, 48_000);
        let mut bus = vec![0.0; 2 * 4800];
        render_span(
            &seq,
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
