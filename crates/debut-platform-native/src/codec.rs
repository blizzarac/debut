//! FFmpeg-backed decoder (MED-01, MED-03, MED-04). Software decode for now; hardware
//! paths (VideoToolbox, NVDEC, QSV, AMF — NFR-09) slot in as `hwaccel` on the same
//! codec context.

use debut_core::{Error, FrameRate, Rational, Result};
use debut_platform::codec::{AudioBlock, AudioInfo, Decoder, VideoFrame, VideoInfo};
use ffmpeg_next as ff;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Once;

fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        ff::init().expect("ffmpeg init");
        ff::log::set_level(ff::log::Level::Error);
    });
}

fn err(e: impl std::fmt::Display) -> Error {
    Error::Other(format!("ffmpeg: {e}"))
}

fn rational(r: ff::Rational) -> Rational {
    if r.denominator() == 0 {
        Rational::ZERO
    } else {
        Rational::new(r.numerator() as i64, r.denominator() as i64)
    }
}

struct VideoStream {
    index: usize,
    time_base: Rational,
    decoder: ff::decoder::Video,
    scaler: ff::software::scaling::Context,
    info: VideoInfo,
}

struct AudioStream {
    index: usize,
    time_base: Rational,
    decoder: ff::decoder::Audio,
    resampler: ff::software::resampling::Context,
    info: AudioInfo,
}

pub struct FfmpegDecoder {
    input: ff::format::context::Input,
    video: Option<VideoStream>,
    audio: Option<AudioStream>,
    video_queue: VecDeque<VideoFrame>,
    audio_queue: VecDeque<AudioBlock>,
    eof: bool,
}

// The FFmpeg contexts are only ever touched from the thread that owns the decoder.
unsafe impl Send for FfmpegDecoder {}

impl FfmpegDecoder {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        init();
        let input = ff::format::input(&path).map_err(err)?;

        let video = match input.streams().best(ff::media::Type::Video) {
            Some(stream) => {
                let ctx = ff::codec::context::Context::from_parameters(stream.parameters())
                    .map_err(err)?;
                let decoder = ctx.decoder().video().map_err(err)?;
                let scaler = ff::software::scaling::Context::get(
                    decoder.format(),
                    decoder.width(),
                    decoder.height(),
                    ff::format::Pixel::RGBA,
                    decoder.width(),
                    decoder.height(),
                    ff::software::scaling::Flags::BILINEAR,
                )
                .map_err(err)?;
                let rate = stream.avg_frame_rate();
                let rate = if rate.numerator() > 0 {
                    rate
                } else {
                    stream.rate()
                };
                let info = VideoInfo {
                    width: decoder.width(),
                    height: decoder.height(),
                    frame_rate: FrameRate::new(
                        rate.numerator() as i64,
                        rate.denominator().max(1) as i64,
                    ),
                    duration: rational(stream.time_base())
                        * Rational::from_int(stream.duration().max(0)),
                    codec: decoder
                        .codec()
                        .map(|c| c.name().to_string())
                        .unwrap_or_default(),
                };
                Some(VideoStream {
                    index: stream.index(),
                    time_base: rational(stream.time_base()),
                    decoder,
                    scaler,
                    info,
                })
            }
            None => None,
        };

        let audio = match input.streams().best(ff::media::Type::Audio) {
            Some(stream) => {
                let ctx = ff::codec::context::Context::from_parameters(stream.parameters())
                    .map_err(err)?;
                let decoder = ctx.decoder().audio().map_err(err)?;
                let layout = decoder.channel_layout();
                let resampler = ff::software::resampling::Context::get(
                    decoder.format(),
                    layout,
                    decoder.rate(),
                    ff::format::Sample::F32(ff::format::sample::Type::Packed),
                    layout,
                    decoder.rate(),
                )
                .map_err(err)?;
                let info = AudioInfo {
                    channels: decoder.channels(),
                    sample_rate: decoder.rate(),
                    duration: rational(stream.time_base())
                        * Rational::from_int(stream.duration().max(0)),
                    codec: decoder
                        .codec()
                        .map(|c| c.name().to_string())
                        .unwrap_or_default(),
                };
                Some(AudioStream {
                    index: stream.index(),
                    time_base: rational(stream.time_base()),
                    decoder,
                    resampler,
                    info,
                })
            }
            None => None,
        };

        Ok(Self {
            input,
            video,
            audio,
            video_queue: VecDeque::new(),
            audio_queue: VecDeque::new(),
            eof: false,
        })
    }

    fn drain_video(&mut self) -> Result<()> {
        let Some(v) = self.video.as_mut() else {
            return Ok(());
        };
        let mut decoded = ff::frame::Video::empty();
        while v.decoder.receive_frame(&mut decoded).is_ok() {
            let mut rgba = ff::frame::Video::empty();
            v.scaler.run(&decoded, &mut rgba).map_err(err)?;
            let pts = decoded.pts().or(decoded.timestamp()).unwrap_or(0);
            let (w, h) = (rgba.width(), rgba.height());
            let stride = rgba.stride(0);
            let data = rgba.data(0);
            let row = (w * 4) as usize;
            let mut rgba8 = Vec::with_capacity(row * h as usize);
            for y in 0..h as usize {
                rgba8.extend_from_slice(&data[y * stride..y * stride + row]);
            }
            self.video_queue.push_back(VideoFrame {
                pts: v.time_base * Rational::from_int(pts),
                width: w,
                height: h,
                rgba8,
            });
        }
        Ok(())
    }

    fn drain_audio(&mut self) -> Result<()> {
        let Some(a) = self.audio.as_mut() else {
            return Ok(());
        };
        let mut decoded = ff::frame::Audio::empty();
        while a.decoder.receive_frame(&mut decoded).is_ok() {
            let mut packed = ff::frame::Audio::empty();
            a.resampler.run(&decoded, &mut packed).map_err(err)?;
            let pts = decoded.pts().or(decoded.timestamp()).unwrap_or(0);
            let n = packed.samples() * packed.channels() as usize;
            let bytes = &packed.data(0)[..n * 4];
            let samples: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            self.audio_queue.push_back(AudioBlock {
                pts: a.time_base * Rational::from_int(pts),
                channels: packed.channels(),
                sample_rate: packed.rate(),
                samples,
            });
        }
        Ok(())
    }

    /// Read packets until something lands in the requested queue or the file ends.
    fn pump(&mut self, want_video: bool) -> Result<()> {
        while !self.eof {
            let ready = if want_video {
                !self.video_queue.is_empty()
            } else {
                !self.audio_queue.is_empty()
            };
            if ready {
                return Ok(());
            }
            let mut packet = ff::Packet::empty();
            match packet.read(&mut self.input) {
                Ok(()) => {}
                Err(ff::Error::Eof) => {
                    self.eof = true;
                    if let Some(v) = self.video.as_mut() {
                        v.decoder.send_eof().ok();
                    }
                    if let Some(a) = self.audio.as_mut() {
                        a.decoder.send_eof().ok();
                    }
                    self.drain_video()?;
                    self.drain_audio()?;
                    return Ok(());
                }
                Err(ff::Error::Other {
                    errno: ff::util::error::EAGAIN,
                }) => continue,
                Err(e) => return Err(err(e)),
            }
            let idx = packet.stream();
            if self.video.as_ref().is_some_and(|v| v.index == idx) {
                self.video
                    .as_mut()
                    .unwrap()
                    .decoder
                    .send_packet(&packet)
                    .map_err(err)?;
                self.drain_video()?;
            } else if self.audio.as_ref().is_some_and(|a| a.index == idx) {
                self.audio
                    .as_mut()
                    .unwrap()
                    .decoder
                    .send_packet(&packet)
                    .map_err(err)?;
                self.drain_audio()?;
            }
        }
        Ok(())
    }
}

impl Decoder for FfmpegDecoder {
    fn video_info(&self) -> Option<&VideoInfo> {
        self.video.as_ref().map(|v| &v.info)
    }

    fn audio_info(&self) -> Option<&AudioInfo> {
        self.audio.as_ref().map(|a| &a.info)
    }

    fn seek(&mut self, to: Rational) -> Result<()> {
        // AV_TIME_BASE units; seek to the keyframe at or before `to`.
        let ts = (to * Rational::from_int(ff::ffi::AV_TIME_BASE as i64)).floor();
        self.input.seek(ts, ..ts).map_err(err)?;
        if let Some(v) = self.video.as_mut() {
            v.decoder.flush();
        }
        if let Some(a) = self.audio.as_mut() {
            a.decoder.flush();
        }
        self.video_queue.clear();
        self.audio_queue.clear();
        self.eof = false;
        Ok(())
    }

    fn next_video(&mut self) -> Result<Option<VideoFrame>> {
        if self.video.is_none() {
            return Ok(None);
        }
        self.pump(true)?;
        Ok(self.video_queue.pop_front())
    }

    fn next_audio(&mut self) -> Result<Option<AudioBlock>> {
        if self.audio.is_none() {
            return Ok(None);
        }
        self.pump(false)?;
        Ok(self.audio_queue.pop_front())
    }
}

/// Placeholder until the FFmpeg encoder lands (EXP-01).
pub struct UnimplementedEncoder;

impl debut_platform::Encoder for UnimplementedEncoder {
    fn push_video(&mut self, _: &VideoFrame) -> Result<()> {
        Err(Error::Unsupported("encoding not implemented".into()))
    }
    fn push_audio(&mut self, _: &AudioBlock) -> Result<()> {
        Err(Error::Unsupported("encoding not implemented".into()))
    }
    fn finish(self: Box<Self>) -> Result<()> {
        Err(Error::Unsupported("encoding not implemented".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/test_25fps_2s.mp4"
    );

    #[test]
    fn probes_stream_info() {
        let d = FfmpegDecoder::open(FIXTURE).unwrap();
        let v = d.video_info().unwrap();
        assert_eq!((v.width, v.height), (64, 36));
        assert_eq!(v.frame_rate, FrameRate::FPS_25);
        assert_eq!(v.codec, "h264");
        assert_eq!(v.duration, Rational::from_int(2));
        let a = d.audio_info().unwrap();
        assert_eq!(a.sample_rate, 48_000);
        assert_eq!(a.codec, "aac");
    }

    #[test]
    fn decodes_every_frame_in_order_with_exact_timestamps() {
        let mut d = FfmpegDecoder::open(FIXTURE).unwrap();
        let mut n = 0;
        while let Some(f) = d.next_video().unwrap() {
            assert_eq!(f.pts, FrameRate::FPS_25.frame_to_time(n), "frame {n}");
            assert_eq!(f.rgba8.len(), 64 * 36 * 4);
            assert!(f.rgba8.chunks(4).all(|p| p[3] == 255));
            n += 1;
        }
        assert_eq!(n, 50);
        assert!(d.next_video().unwrap().is_none());
    }

    #[test]
    fn decodes_audio_as_interleaved_f32() {
        let mut d = FfmpegDecoder::open(FIXTURE).unwrap();
        let mut total = 0usize;
        let mut peak = 0.0f32;
        while let Some(b) = d.next_audio().unwrap() {
            assert_eq!(b.sample_rate, 48_000);
            total += b.samples.len() / b.channels as usize;
            peak = b.samples.iter().fold(peak, |p, s| p.max(s.abs()));
        }
        // ~2 s of audio (encoder priming may add or trim a few frames).
        assert!((90_000..=100_000).contains(&total), "{total} samples");
        assert!(peak > 0.1 && peak <= 1.0, "peak {peak}");
    }

    #[test]
    fn seek_lands_at_or_before_the_target() {
        let mut d = FfmpegDecoder::open(FIXTURE).unwrap();
        d.seek(Rational::new(3, 2)).unwrap();
        let f = d.next_video().unwrap().unwrap();
        assert!(f.pts <= Rational::new(3, 2), "{:?}", f.pts);
        // Frames keep coming from there to the end.
        let mut last = f.pts;
        while let Some(f) = d.next_video().unwrap() {
            assert!(f.pts > last);
            last = f.pts;
        }
        assert_eq!(last, FrameRate::FPS_25.frame_to_time(49));
    }
}
