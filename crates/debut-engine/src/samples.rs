//! Decoder-backed [`SampleSource`]: pulls audio blocks from a platform decoder,
//! converts to the engine rate, and serves contiguous reads with a small
//! look-back buffer so the renderer's sequential blocks never re-seek.

use debut_audio::SampleSource;
use debut_core::{Error, MediaId, Rational, Result, SequenceId};
use debut_platform::Decoder;
use std::collections::HashMap;
use std::sync::Arc;

struct Source {
    decoder: Box<dyn Decoder>,
    channels: u16,
    /// Interleaved samples at the engine rate, starting at `buf_start` (engine samples).
    buf: Vec<f32>,
    buf_start: i64,
    /// Engine-sample index right after the last decoded sample (for sequential reads).
    decoded_to: i64,
    eof: bool,
}

pub struct SampleCache {
    rate: u32,
    sources: HashMap<MediaId, Source>,
    /// Keep this many engine frames behind the read point.
    keep_back: usize,
    sequences: HashMap<SequenceId, Arc<debut_project::Sequence>>,
}

impl SampleCache {
    pub fn new(rate: u32) -> Self {
        Self {
            rate,
            sources: HashMap::new(),
            keep_back: rate as usize / 4,
            sequences: HashMap::new(),
        }
    }

    /// Replace the set of sequences compound clips can refer to (TL-07).
    pub fn set_sequences(&mut self, all: &[debut_project::Sequence]) {
        self.sequences = all.iter().map(|s| (s.id, Arc::new(s.clone()))).collect();
    }

    pub fn add(&mut self, media: MediaId, decoder: Box<dyn Decoder>) -> Result<()> {
        let info = decoder
            .audio_info()
            .ok_or_else(|| Error::InvalidArgument("media has no audio stream".into()))?;
        let channels = info.channels;
        self.sources.insert(
            media,
            Source {
                decoder,
                channels,
                buf: Vec::new(),
                buf_start: 0,
                decoded_to: 0,
                eof: false,
            },
        );
        Ok(())
    }

    /// Drop a media's decoder and buffer (before relinking it).
    pub fn remove(&mut self, media: MediaId) {
        self.sources.remove(&media);
    }

    pub fn channels(&self, media: MediaId) -> Option<u16> {
        self.sources.get(&media).map(|s| s.channels)
    }
}

/// Linear resample interleaved `input` from `from` Hz to `to` Hz.
fn resample(input: &[f32], channels: usize, from: u32, to: u32) -> Vec<f32> {
    if from == to {
        return input.to_vec();
    }
    let in_frames = input.len() / channels;
    let out_frames = (in_frames as u64 * to as u64 / from as u64) as usize;
    let ratio = from as f64 / to as f64;
    let mut out = Vec::with_capacity(out_frames * channels);
    for i in 0..out_frames {
        let p = i as f64 * ratio;
        let p0 = (p.floor() as usize).min(in_frames.saturating_sub(1));
        let p1 = (p0 + 1).min(in_frames.saturating_sub(1));
        let f = (p - p0 as f64) as f32;
        for c in 0..channels {
            let a = input[p0 * channels + c];
            let b = input[p1 * channels + c];
            out.push(a + (b - a) * f);
        }
    }
    out
}

impl SampleSource for SampleCache {
    fn sequence(&self, id: SequenceId) -> Option<Arc<debut_project::Sequence>> {
        self.sequences.get(&id).cloned()
    }

    fn read(
        &mut self,
        media: MediaId,
        start: Rational,
        frames: usize,
        out: &mut Vec<f32>,
    ) -> Result<u16> {
        let rate = self.rate;
        let keep_back = self.keep_back;
        let Some(src) = self.sources.get_mut(&media) else {
            // Offline or video-only media: silence, not an error (MED-05).
            out.clear();
            out.resize(frames, 0.0);
            return Ok(1);
        };
        let ch = src.channels as usize;
        let s0 = (start * Rational::from_int(rate as i64)).round();
        let s1 = s0 + frames as i64;

        let buf_end = src.buf_start + (src.buf.len() / ch) as i64;
        if s0 < src.buf_start || s0 > buf_end + rate as i64 {
            // Outside what we have: seek and restart the buffer there.
            src.decoder.seek(start)?;
            src.buf.clear();
            src.buf_start = s0;
            src.decoded_to = s0;
            src.eof = false;
            // Decoded blocks begin at or before `start`; drop the lead-in below.
            let mut lead_in = true;
            while src.decoded_to < s1 && !src.eof {
                match src.decoder.next_audio()? {
                    Some(block) => {
                        let mut samples =
                            resample(&block.samples, ch, block.sample_rate.max(1), rate);
                        let block_start = (block.pts * Rational::from_int(rate as i64)).round();
                        if lead_in {
                            let skip = ((s0 - block_start).max(0) as usize * ch).min(samples.len());
                            samples.drain(..skip);
                            let actual_start = block_start.max(s0);
                            // Silence between the requested start and the first decoded sample.
                            let gap = ((actual_start - s0).max(0) as usize) * ch;
                            src.buf.resize(gap, 0.0);
                            lead_in = false;
                        }
                        src.buf.extend_from_slice(&samples);
                        src.decoded_to = src.buf_start + (src.buf.len() / ch) as i64;
                    }
                    None => src.eof = true,
                }
            }
        } else {
            while src.decoded_to < s1 && !src.eof {
                match src.decoder.next_audio()? {
                    Some(block) => {
                        let samples = resample(&block.samples, ch, block.sample_rate.max(1), rate);
                        src.buf.extend_from_slice(&samples);
                        src.decoded_to = src.buf_start + (src.buf.len() / ch) as i64;
                    }
                    None => src.eof = true,
                }
            }
        }

        // Serve, padding with silence past the end.
        out.clear();
        out.resize(frames * ch, 0.0);
        let off = (s0 - src.buf_start) as usize * ch;
        let avail = src.buf.len().saturating_sub(off).min(frames * ch);
        if avail > 0 {
            out[..avail].copy_from_slice(&src.buf[off..off + avail]);
        }

        // Trim the look-back.
        let behind = (s0 - src.buf_start).max(0) as usize;
        if behind > keep_back * 2 {
            let drop = (behind - keep_back) * ch;
            src.buf.drain(..drop);
            src.buf_start += (behind - keep_back) as i64;
        }
        Ok(src.channels)
    }
}
