//! One export: walk the frames of a sequence range through the render graph and
//! the audio mixer into an [`Encoder`] (EXP-01, EXP-04). Playback and export share
//! `compose`/`render`/`render_span`, so the file matches the viewer.

use crate::hdr::{encode_rgba16, LightLevels};
use debut_audio::{render_span, Inserts, LoudnessMeter, SampleSource, CHANNELS};
use debut_core::{Rational, Result};
use debut_platform::codec::{AudioBlock, Encoder, HdrTransfer, VideoFrame};
use debut_project::Sequence;
use debut_render::compose::{compose, SourceInfo};
use debut_render::{Backend, FrameProvider, Rgba};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct ExportJob {
    pub sequence: Sequence,
    /// Inclusive start, exclusive end on the timeline.
    pub range: (Rational, Rational),
    pub sample_rate: u32,
    /// Master gain applied to the mixed audio (loudness normalization, AUD-06).
    pub gain_db: f32,
    /// Write 16-bit Rec.2020 PQ/HLG frames for an HDR encoder (EXP-06).
    pub hdr: Option<HdrTransfer>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Progress {
    pub frames_done: u64,
    pub frames_total: u64,
    /// Integrated loudness of what was written so far, once measurable.
    pub loudness_lufs: Option<f32>,
    pub true_peak_db: f32,
    /// MaxCLL / MaxFALL of the frames written so far (HDR exports only).
    pub light: Option<LightLevels>,
}

/// Cooperative control shared with the queue / UI (EXP-03): the job checks it
/// between frames.
#[derive(Clone, Default)]
pub struct Control(Arc<AtomicU8>);

const RUN: u8 = 0;
const PAUSE: u8 = 1;
const CANCEL: u8 = 2;

impl Control {
    pub fn pause(&self) {
        self.0.store(PAUSE, Ordering::Release);
    }
    pub fn resume(&self) {
        self.0.store(RUN, Ordering::Release);
    }
    pub fn cancel(&self) {
        self.0.store(CANCEL, Ordering::Release);
    }
    pub fn is_paused(&self) -> bool {
        self.0.load(Ordering::Acquire) == PAUSE
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire) == CANCEL
    }
}

/// Linear premultiplied f32 -> display-encoded straight RGBA8 (the CPU reference
/// for the GPU output pass).
pub fn to_rgba8(px: &[Rgba]) -> Vec<u8> {
    debut_render::encode_rgba8(px, debut_render::Transfer::Srgb)
}

/// Run `job` to completion (or cancellation) on the calling thread. `on_progress`
/// is called after every frame; while paused the job sleeps between checks.
#[allow(clippy::too_many_arguments)]
pub fn export<B: Backend, S: SourceInfo + FrameProvider>(
    job: &ExportJob,
    backend: &mut B,
    frames: &mut S,
    samples: &mut dyn SampleSource,
    encoder: &mut dyn Encoder,
    control: &Control,
    mut on_progress: impl FnMut(Progress),
) -> Result<Progress> {
    let fr = job.sequence.frame_rate;
    let first = fr.time_to_frame(job.range.0);
    let last = fr.time_to_frame(job.range.1); // exclusive
    let total = (last - first).max(0) as u64;
    let sr = job.sample_rate;
    let spf = Rational::from_int(sr as i64) * fr.frame_duration();
    let mut audio_cursor = (job.range.0 * Rational::from_int(sr as i64)).round();
    let mut progress = Progress {
        frames_done: 0,
        frames_total: total,
        loudness_lufs: None,
        true_peak_db: f32::NEG_INFINITY,
        light: job.hdr.map(|_| LightLevels::default()),
    };
    let mut bus = Vec::new();
    let mut inserts = Inserts::default();
    inserts.set_plugins(samples.plugins());
    inserts.sync(&job.sequence, sr);
    let mut meter = LoudnessMeter::new(sr);
    let gain = 10f32.powf(job.gain_db / 20.0);

    for n in first..last {
        while control.is_paused() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if control.is_cancelled() {
            break;
        }
        let t = fr.frame_to_time(n);
        let graph = compose(&job.sequence, t, frames);
        let img = graph.render(backend, frames)?;
        let (w, h) = backend.size(&img);
        let frame = match job.hdr {
            Some(transfer) => {
                let px = backend.download(&img);
                if let Some(light) = progress.light.as_mut() {
                    light.add_frame(&px);
                }
                VideoFrame {
                    pts: t,
                    width: w,
                    height: h,
                    rgba16: encode_rgba16(&px, transfer),
                    ..Default::default()
                }
            }
            None => VideoFrame {
                pts: t,
                width: w,
                height: h,
                rgba8: backend.download_rgba8(&img, debut_render::Transfer::Srgb),
                ..Default::default()
            },
        };
        encoder.push_video(&frame)?;

        // Audio up to the end of this frame; exact per-frame counts at any rate.
        let next_cursor = (Rational::from_int(n + 1) * spf
            + job.range.0 * Rational::from_int(sr as i64)
            - Rational::from_int(first) * spf)
            .round();
        let count = (next_cursor - audio_cursor).max(0) as usize;
        if count > 0 {
            bus.clear();
            bus.resize(count * CHANNELS, 0.0);
            render_span(
                &job.sequence,
                &mut inserts,
                samples,
                audio_cursor,
                count,
                sr,
                &mut bus,
            )?;
            if gain != 1.0 {
                bus.iter_mut().for_each(|s| *s *= gain);
            }
            meter.push(&bus);
            progress.loudness_lufs = meter.integrated();
            progress.true_peak_db = meter.true_peak_db();
            encoder.push_audio(&AudioBlock {
                pts: Rational::new(audio_cursor, sr as i64),
                channels: CHANNELS as u16,
                sample_rate: sr,
                samples: bus.clone(),
            })?;
            audio_cursor = next_cursor;
        }

        progress.frames_done += 1;
        on_progress(progress);
    }
    Ok(progress)
}

/// Loudness of the job's audio alone (no video, no encoder): the first pass of a
/// normalize-to-target export. Returns (integrated LUFS, true peak dBTP).
pub fn measure_loudness(
    job: &ExportJob,
    samples: &mut dyn SampleSource,
) -> Result<(Option<f32>, f32)> {
    let sr = job.sample_rate;
    let start = (job.range.0 * Rational::from_int(sr as i64)).round();
    let end = (job.range.1 * Rational::from_int(sr as i64)).round();
    let mut inserts = Inserts::default();
    inserts.set_plugins(samples.plugins());
    inserts.sync(&job.sequence, sr);
    let mut meter = LoudnessMeter::new(sr);
    let gain = 10f32.powf(job.gain_db / 20.0);
    let block = 4800usize;
    let mut bus = vec![0.0f32; block * CHANNELS];
    let mut cursor = start;
    while cursor < end {
        let n = ((end - cursor) as usize).min(block);
        bus.clear();
        bus.resize(n * CHANNELS, 0.0);
        render_span(
            &job.sequence,
            &mut inserts,
            samples,
            cursor,
            n,
            sr,
            &mut bus,
        )?;
        if gain != 1.0 {
            bus.iter_mut().for_each(|s| *s *= gain);
        }
        meter.push(&bus);
        cursor += n as i64;
    }
    Ok((meter.integrated(), meter.true_peak_db()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba8_conversion_unpremultiplies_and_encodes_srgb() {
        let px = to_rgba8(&[
            [1.0, 0.0, 0.2158605, 1.0],
            [0.25, 0.25, 0.25, 0.5],
            [0.0; 4],
        ]);
        assert_eq!(&px[..4], &[255, 0, 128, 255]);
        // 0.25 / 0.5 = 0.5 linear -> 188 sRGB
        assert_eq!(&px[4..8], &[188, 188, 188, 128]);
        assert_eq!(&px[8..], &[0, 0, 0, 0]);
    }
}
