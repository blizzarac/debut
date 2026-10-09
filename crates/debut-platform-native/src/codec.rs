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
                // r_frame_rate is the container's nominal rate; fall back to the
                // measured average for streams that don't declare one.
                let rate = stream.rate();
                let rate = if rate.numerator() > 0 {
                    rate
                } else {
                    stream.avg_frame_rate()
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

/// Encode settings for one output file (EXP-01). Video is H.264 via libx264 and
/// audio AAC for now; HEVC/AV1/ProRes/DNxHR and hardware encoders are a matter of
/// codec selection on the same path.
#[derive(Clone, Debug)]
pub struct EncodeSettings {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    /// Constant rate factor for x264 (lower = better, 18–28 is typical).
    pub crf: u8,
    pub audio: Option<AudioEncodeSettings>,
}

#[derive(Clone, Debug)]
pub struct AudioEncodeSettings {
    pub channels: u16,
    pub sample_rate: u32,
    pub bitrate: usize,
}

struct VideoEnc {
    stream_index: usize,
    encoder: ff::encoder::Video,
    scaler: ff::software::scaling::Context,
    time_base: ff::Rational,
    next_pts: i64,
}

struct AudioEnc {
    stream_index: usize,
    encoder: ff::encoder::Audio,
    resampler: ff::software::resampling::Context,
    time_base: ff::Rational,
    frame_size: usize,
    channels: u16,
    /// Interleaved f32 waiting to fill a whole encoder frame.
    pending: Vec<f32>,
    next_pts: i64,
}

pub struct FfmpegEncoder {
    output: ff::format::context::Output,
    video: VideoEnc,
    audio: Option<AudioEnc>,
    settings: EncodeSettings,
}

unsafe impl Send for FfmpegEncoder {}

impl FfmpegEncoder {
    pub fn create(path: impl AsRef<Path>, settings: EncodeSettings) -> Result<Self> {
        init();
        let mut output = ff::format::output(&path).map_err(err)?;
        let global_header = output
            .format()
            .flags()
            .contains(ff::format::Flags::GLOBAL_HEADER);

        let fr = settings.frame_rate.0;
        let video = {
            let codec = ff::encoder::find(ff::codec::Id::H264)
                .ok_or_else(|| Error::Unsupported("no H.264 encoder".into()))?;
            let mut stream = output.add_stream(codec).map_err(err)?;
            let ctx = ff::codec::context::Context::new_with_codec(codec);
            let mut enc = ctx.encoder().video().map_err(err)?;
            enc.set_width(settings.width);
            enc.set_height(settings.height);
            enc.set_format(ff::format::Pixel::YUV420P);
            let time_base = ff::Rational::new(fr.den as i32, fr.num as i32);
            enc.set_time_base(time_base);
            enc.set_frame_rate(Some(ff::Rational::new(fr.num as i32, fr.den as i32)));
            enc.set_gop(12);
            if global_header {
                enc.set_flags(ff::codec::Flags::GLOBAL_HEADER);
            }
            let mut opts = ff::Dictionary::new();
            opts.set("preset", "medium");
            opts.set("crf", &settings.crf.to_string());
            let encoder = enc.open_with(opts).map_err(err)?;
            stream.set_parameters(&encoder);
            stream.set_time_base(time_base);
            stream.set_rate(ff::Rational::new(fr.num as i32, fr.den as i32));
            stream.set_avg_frame_rate(ff::Rational::new(fr.num as i32, fr.den as i32));
            let scaler = ff::software::scaling::Context::get(
                ff::format::Pixel::RGBA,
                settings.width,
                settings.height,
                ff::format::Pixel::YUV420P,
                settings.width,
                settings.height,
                ff::software::scaling::Flags::BILINEAR,
            )
            .map_err(err)?;
            VideoEnc {
                stream_index: stream.index(),
                encoder,
                scaler,
                time_base,
                next_pts: 0,
            }
        };

        let audio = match &settings.audio {
            Some(a) => {
                let codec = ff::encoder::find(ff::codec::Id::AAC)
                    .ok_or_else(|| Error::Unsupported("no AAC encoder".into()))?;
                let mut stream = output.add_stream(codec).map_err(err)?;
                let ctx = ff::codec::context::Context::new_with_codec(codec);
                let mut enc = ctx.encoder().audio().map_err(err)?;
                let layout = ff::ChannelLayout::default(a.channels as i32);
                enc.set_rate(a.sample_rate as i32);
                enc.set_channel_layout(layout);
                enc.set_format(ff::format::Sample::F32(ff::format::sample::Type::Planar));
                enc.set_bit_rate(a.bitrate);
                let time_base = ff::Rational::new(1, a.sample_rate as i32);
                enc.set_time_base(time_base);
                if global_header {
                    enc.set_flags(ff::codec::Flags::GLOBAL_HEADER);
                }
                let encoder = enc.open().map_err(err)?;
                stream.set_parameters(&encoder);
                stream.set_time_base(time_base);
                let resampler = ff::software::resampling::Context::get(
                    ff::format::Sample::F32(ff::format::sample::Type::Packed),
                    layout,
                    a.sample_rate,
                    ff::format::Sample::F32(ff::format::sample::Type::Planar),
                    layout,
                    a.sample_rate,
                )
                .map_err(err)?;
                let frame_size = encoder.frame_size() as usize;
                Some(AudioEnc {
                    stream_index: stream.index(),
                    encoder,
                    resampler,
                    time_base,
                    frame_size: if frame_size == 0 { 1024 } else { frame_size },
                    channels: a.channels,
                    pending: Vec::new(),
                    next_pts: 0,
                })
            }
            None => None,
        };

        output.write_header().map_err(err)?;
        Ok(Self {
            output,
            video,
            audio,
            settings,
        })
    }

    fn write_video_packets(&mut self) -> Result<()> {
        let mut packet = ff::Packet::empty();
        while self.video.encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(self.video.stream_index);
            let out_tb = self
                .output
                .stream(self.video.stream_index)
                .unwrap()
                .time_base();
            packet.rescale_ts(self.video.time_base, out_tb);
            packet.write_interleaved(&mut self.output).map_err(err)?;
        }
        Ok(())
    }

    fn write_audio_packets(&mut self) -> Result<()> {
        let Some(a) = self.audio.as_mut() else {
            return Ok(());
        };
        let mut packet = ff::Packet::empty();
        while a.encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(a.stream_index);
            let out_tb = self.output.stream(a.stream_index).unwrap().time_base();
            packet.rescale_ts(a.time_base, out_tb);
            packet.write_interleaved(&mut self.output).map_err(err)?;
        }
        Ok(())
    }

    /// Encode one full audio frame from `pending` (or a short final one).
    fn encode_audio_frame(&mut self, flush_partial: bool) -> Result<bool> {
        let Some(a) = self.audio.as_mut() else {
            return Ok(false);
        };
        let ch = a.channels as usize;
        let have = a.pending.len() / ch;
        let n = if have >= a.frame_size {
            a.frame_size
        } else if flush_partial && have > 0 {
            have
        } else {
            return Ok(false);
        };
        let mut packed = ff::frame::Audio::new(
            ff::format::Sample::F32(ff::format::sample::Type::Packed),
            n,
            a.encoder.channel_layout(),
        );
        packed.set_rate(a.encoder.rate());
        let bytes: Vec<u8> = a.pending[..n * ch]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        packed.data_mut(0)[..bytes.len()].copy_from_slice(&bytes);
        a.pending.drain(..n * ch);
        let mut planar = ff::frame::Audio::empty();
        a.resampler.run(&packed, &mut planar).map_err(err)?;
        planar.set_pts(Some(a.next_pts));
        a.next_pts += n as i64;
        a.encoder.send_frame(&planar).map_err(err)?;
        self.write_audio_packets()?;
        Ok(true)
    }
}

impl debut_platform::Encoder for FfmpegEncoder {
    fn push_video(&mut self, frame: &VideoFrame) -> Result<()> {
        if (frame.width, frame.height) != (self.settings.width, self.settings.height) {
            return Err(Error::InvalidArgument(
                "frame size does not match encoder".into(),
            ));
        }
        let mut rgba = ff::frame::Video::new(ff::format::Pixel::RGBA, frame.width, frame.height);
        let stride = rgba.stride(0);
        let row = (frame.width * 4) as usize;
        {
            let data = rgba.data_mut(0);
            for y in 0..frame.height as usize {
                data[y * stride..y * stride + row]
                    .copy_from_slice(&frame.rgba8[y * row..(y + 1) * row]);
            }
        }
        let mut yuv = ff::frame::Video::empty();
        self.video.scaler.run(&rgba, &mut yuv).map_err(err)?;
        yuv.set_pts(Some(self.video.next_pts));
        self.video.next_pts += 1;
        self.video.encoder.send_frame(&yuv).map_err(err)?;
        self.write_video_packets()
    }

    fn push_audio(&mut self, block: &AudioBlock) -> Result<()> {
        let Some(a) = self.audio.as_mut() else {
            return Ok(());
        };
        if block.channels != a.channels || block.sample_rate != a.encoder.rate() {
            return Err(Error::InvalidArgument(
                "audio block format does not match encoder".into(),
            ));
        }
        a.pending.extend_from_slice(&block.samples);
        while self.encode_audio_frame(false)? {}
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<()> {
        self.encode_audio_frame(true)?;
        self.video.encoder.send_eof().map_err(err)?;
        self.write_video_packets()?;
        if let Some(a) = self.audio.as_mut() {
            a.encoder.send_eof().map_err(err)?;
        }
        self.write_audio_packets()?;
        self.output.write_trailer().map_err(err)
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

    #[test]
    fn encodes_frames_and_audio_that_decode_back() {
        use debut_platform::Encoder;
        let path = std::env::temp_dir().join(format!("debut-enc-{}.mp4", std::process::id()));
        let settings = EncodeSettings {
            width: 64,
            height: 36,
            frame_rate: FrameRate::FPS_25,
            crf: 20,
            audio: Some(AudioEncodeSettings {
                channels: 2,
                sample_rate: 48_000,
                bitrate: 64_000,
            }),
        };
        let mut enc = Box::new(FfmpegEncoder::create(&path, settings).unwrap());
        // 30 frames: left half red, right half ramps from black to white.
        for n in 0..30 {
            let mut rgba8 = Vec::with_capacity(64 * 36 * 4);
            for _y in 0..36 {
                for x in 0..64 {
                    let v = (n * 255 / 29) as u8;
                    let px = if x < 32 {
                        [255, 0, 0, 255]
                    } else {
                        [v, v, v, 255]
                    };
                    rgba8.extend_from_slice(&px);
                }
            }
            enc.push_video(&VideoFrame {
                pts: FrameRate::FPS_25.frame_to_time(n),
                width: 64,
                height: 36,
                rgba8,
            })
            .unwrap();
            // 1 920 stereo frames of a 1 kHz tone per video frame.
            let samples: Vec<f32> = (0..1920)
                .flat_map(|i| {
                    let s = ((n as usize * 1920 + i) as f32 * 1000.0 * std::f32::consts::TAU
                        / 48_000.0)
                        .sin()
                        * 0.5;
                    [s, s]
                })
                .collect();
            enc.push_audio(&AudioBlock {
                pts: FrameRate::FPS_25.frame_to_time(n),
                channels: 2,
                sample_rate: 48_000,
                samples,
            })
            .unwrap();
        }
        enc.finish().unwrap();

        let mut dec = FfmpegDecoder::open(&path).unwrap();
        let v = dec.video_info().unwrap();
        assert_eq!(
            (v.width, v.height, v.frame_rate),
            (64, 36, FrameRate::FPS_25)
        );
        let mut n = 0;
        let mut last_right = -1i32;
        while let Some(f) = dec.next_video().unwrap() {
            assert_eq!(f.pts, FrameRate::FPS_25.frame_to_time(n));
            let left = &f.rgba8[(18 * 64 + 8) * 4..][..3];
            assert!(
                left[0] > 200 && left[1] < 60 && left[2] < 60,
                "frame {n} left {left:?}"
            );
            let right = f.rgba8[(18 * 64 + 56) * 4] as i32;
            assert!(
                right >= last_right - 8,
                "ramp must not go backwards: {right} after {last_right}"
            );
            last_right = right;
            n += 1;
        }
        assert_eq!(n, 30);
        assert!(last_right > 200);

        let mut total = 0;
        let mut peak = 0.0f32;
        while let Some(b) = dec.next_audio().unwrap() {
            total += b.samples.len() / 2;
            peak = b.samples.iter().fold(peak, |p, s| p.max(s.abs()));
        }
        assert!(
            (50_000..=62_000).contains(&total),
            "{total} audio frames (expected ~57 600)"
        );
        assert!(peak > 0.3, "peak {peak}");
        std::fs::remove_file(path).ok();
    }
}
