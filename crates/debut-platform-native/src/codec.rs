//! FFmpeg-backed decoder (MED-01, MED-03, MED-04), in software or through a
//! hardware device (`hwdecode`, NFR-09).

use crate::hwdecode;
use debut_core::{Error, FrameRate, Rational, Result};
use debut_platform::codec::{
    AudioBlock, AudioInfo, DecodePath, Decoder, SourceTags, VideoFrame, VideoInfo,
};
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
    /// To RGBA, for the last frame's (format, width, height): a hardware
    /// frame arrives as the device's download format, not the stream's.
    scaler: Option<(ff::format::Pixel, u32, u32, ff::software::scaling::Context)>,
    info: VideoInfo,
    /// The device the decoder was given, if any (kept alive with it).
    device: Option<hwdecode::Device>,
    path: DecodePath,
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
    /// Streams to decode; packets of a switched-off stream are dropped.
    want_video: bool,
    want_audio: bool,
    tags: SourceTags,
}

// The FFmpeg contexts are only ever touched from the thread that owns the decoder.
unsafe impl Send for FfmpegDecoder {}

impl FfmpegDecoder {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, false)
    }

    /// Open, decoding video on the first hardware device that takes the
    /// stream when `hardware` is set (else, or failing that, in software).
    pub fn open_with(path: impl AsRef<Path>, hardware: bool) -> Result<Self> {
        init();
        let input = ff::format::input(&path).map_err(err)?;

        let video = match input.streams().best(ff::media::Type::Video) {
            Some(stream) => {
                let mut ctx = ff::codec::context::Context::from_parameters(stream.parameters())
                    .map_err(err)?;
                let (device, path) = if hardware {
                    attach_device(&mut ctx)
                } else {
                    (None, DecodePath::Software)
                };
                let decoder = ctx.decoder().video().map_err(err)?;
                // r_frame_rate is the container's nominal rate; fall back to the
                // measured average for streams that don't declare one.
                let rate = stream.rate();
                let rate = if rate.numerator() > 0 {
                    rate
                } else {
                    stream.avg_frame_rate()
                };
                let info = VideoInfo {
                    variable_frame_rate: false, // measured below
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
                    scaler: None,
                    info,
                    device,
                    path,
                })
            }
            None => None,
        };

        let audio = match input.streams().best(ff::media::Type::Audio) {
            Some(stream) => {
                let ctx = ff::codec::context::Context::from_parameters(stream.parameters())
                    .map_err(err)?;
                let decoder = ctx.decoder().audio().map_err(err)?;
                // WAV and some other files leave the layout unspecified;
                // the resampler wants one, so use the usual one for the count.
                let layout = match decoder.channel_layout() {
                    l if l.is_empty() || l.bits() == 0 => {
                        ff::ChannelLayout::default(decoder.channels() as i32)
                    }
                    l => l,
                };
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

        let tags = read_tags(&input);
        let mut input = input;
        let mut video = video;
        if let Some(v) = video.as_mut() {
            v.info.variable_frame_rate = scan_variable_rate(&mut input, v.index);
        }
        Ok(Self {
            input,
            video,
            audio,
            tags,
            video_queue: VecDeque::new(),
            audio_queue: VecDeque::new(),
            want_video: true,
            want_audio: true,
            eof: false,
        })
    }

    fn drain_video(&mut self) -> Result<()> {
        let Some(v) = self.video.as_mut() else {
            return Ok(());
        };
        let mut decoded = ff::frame::Video::empty();
        while v.decoder.receive_frame(&mut decoded).is_ok() {
            // SAFETY: `decoded` is a frame the decoder just filled.
            let on_device = unsafe { hwdecode::on_device(decoded.as_ptr()) };
            if let Some(device) = &v.device {
                v.path = if on_device {
                    DecodePath::Hardware(hwdecode::type_name(device))
                } else {
                    DecodePath::Fallback {
                        wanted: device.name.clone(),
                        reason: format!(
                            "the {} driver does not decode {}",
                            device.name, v.info.codec
                        ),
                    }
                };
            }
            let mut system = ff::frame::Video::empty();
            let frame = if on_device {
                // SAFETY: a hardware frame into an empty system-memory frame.
                unsafe { hwdecode::download(decoded.as_ptr(), system.as_mut_ptr()) }
                    .map_err(err)?;
                &system
            } else {
                &decoded
            };
            let key = (frame.format(), frame.width(), frame.height());
            if v.scaler.as_ref().map(|s| (s.0, s.1, s.2)) != Some(key) {
                let scaler = ff::software::scaling::Context::get(
                    key.0,
                    key.1,
                    key.2,
                    ff::format::Pixel::RGBA,
                    key.1,
                    key.2,
                    ff::software::scaling::Flags::BILINEAR,
                )
                .map_err(err)?;
                v.scaler = Some((key.0, key.1, key.2, scaler));
            }
            let mut rgba = ff::frame::Video::empty();
            let scaler = &mut v.scaler.as_mut().expect("just made").3;
            scaler.run(frame, &mut rgba).map_err(err)?;
            let pts = frame.pts().or(frame.timestamp()).unwrap_or(0);
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
            if decoded.channel_layout().is_empty() || decoded.channel_layout().bits() == 0 {
                decoded.set_channel_layout(ff::ChannelLayout::default(decoded.channels() as i32));
            }
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
            if self.want_video && self.video.as_ref().is_some_and(|v| v.index == idx) {
                self.video
                    .as_mut()
                    .unwrap()
                    .decoder
                    .send_packet(&packet)
                    .map_err(err)?;
                self.drain_video()?;
            } else if self.want_audio && self.audio.as_ref().is_some_and(|a| a.index == idx) {
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

/// Timecode, reel and camera from the container and every stream (the
/// timecode often sits on a `tmcd` data stream, the reel next to it).
fn read_tags(input: &ff::format::context::Input) -> SourceTags {
    let mut tags = SourceTags::default();
    let mut look = |dict: ff::DictionaryRef| {
        for (k, v) in dict.iter() {
            let v = v.trim();
            if v.is_empty() {
                continue;
            }
            let slot = match k.to_ascii_lowercase().as_str() {
                "timecode" => &mut tags.timecode,
                "reel_name" | "reel" | "com.apple.quicktime.reel" => &mut tags.reel,
                "com.apple.quicktime.model" | "model" | "camera_model" => &mut tags.camera,
                _ => continue,
            };
            slot.get_or_insert_with(|| v.to_string());
        }
    };
    look(input.metadata());
    for stream in input.streams() {
        look(stream.metadata());
    }
    tags
}

/// Read the first video packets' timestamps (demuxing only) to tell a
/// variable frame rate, then rewind.
fn scan_variable_rate(input: &mut ff::format::context::Input, index: usize) -> bool {
    let mut pts = Vec::new();
    for (stream, packet) in input.packets().take(2000) {
        if stream.index() == index {
            if let Some(p) = packet.pts() {
                pts.push(p);
            }
            if pts.len() >= 240 {
                break;
            }
        }
    }
    let _ = input.seek(0, ..);
    debut_platform::codec::uneven_timestamps(&pts)
}

/// Give the codec context the first candidate device its codec can decode
/// through; the path reported until the first frame says what was tried.
fn attach_device(ctx: &mut ff::codec::context::Context) -> (Option<hwdecode::Device>, DecodePath) {
    let Some(codec) = ff::decoder::find(ctx.id()) else {
        return (None, DecodePath::Software);
    };
    let mut tried = Vec::new();
    for name in hwdecode::candidates() {
        match hwdecode::open_device(&name) {
            // SAFETY: a valid codec, and a context not yet opened.
            Ok(device) => unsafe {
                if hwdecode::surface_format(codec.as_ptr(), &device).is_some() {
                    hwdecode::attach(ctx.as_mut_ptr(), &device);
                    return (Some(device), DecodePath::Requested(name));
                }
                tried.push(format!("{name} cannot decode {}", codec.name()));
            },
            Err(e) => tried.push(e),
        }
    }
    (
        None,
        DecodePath::Fallback {
            wanted: "hardware".into(),
            reason: tried.join("; "),
        },
    )
}

impl Decoder for FfmpegDecoder {
    fn decode_path(&self) -> DecodePath {
        self.video
            .as_ref()
            .map_or(DecodePath::Software, |v| v.path.clone())
    }

    fn tags(&self) -> SourceTags {
        self.tags.clone()
    }

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

    fn select(&mut self, video: bool, audio: bool) {
        self.want_video = video;
        self.want_audio = audio;
        if !video {
            self.video_queue.clear();
        }
        if !audio {
            self.audio_queue.clear();
        }
    }

    fn next_video(&mut self) -> Result<Option<VideoFrame>> {
        if self.video.is_none() || !self.want_video {
            return Ok(None);
        }
        self.pump(true)?;
        Ok(self.video_queue.pop_front())
    }

    fn next_audio(&mut self) -> Result<Option<AudioBlock>> {
        if self.audio.is_none() || !self.want_audio {
            return Ok(None);
        }
        self.pump(false)?;
        Ok(self.audio_queue.pop_front())
    }
}

pub use debut_platform::codec::{AudioEncodeSettings, EncodeSettings, HwEncoder};

/// Candidates by (name, codec, api). VAAPI needs hardware frame contexts and is
/// left out until the encoder can upload them.
const HW_ENCODERS: &[(&str, &str, &str)] = &[
    ("h264_videotoolbox", "h264", "VideoToolbox"),
    ("hevc_videotoolbox", "hevc", "VideoToolbox"),
    ("h264_nvenc", "h264", "NVENC"),
    ("hevc_nvenc", "hevc", "NVENC"),
    ("h264_qsv", "h264", "Quick Sync"),
    ("hevc_qsv", "hevc", "Quick Sync"),
    ("h264_amf", "h264", "AMF"),
    ("hevc_amf", "hevc", "AMF"),
];

/// Pixel format a named encoder takes from the scaler.
fn input_format(encoder: &str) -> ff::format::Pixel {
    if encoder.contains("qsv") || encoder.contains("amf") {
        ff::format::Pixel::NV12
    } else {
        ff::format::Pixel::YUV420P
    }
}

/// Try to open `name` on a tiny frame; true when the driver is really there,
/// not just compiled in.
fn probe_encoder(name: &str) -> bool {
    let Some(codec) = ff::encoder::find_by_name(name) else {
        return false;
    };
    let ctx = ff::codec::context::Context::new_with_codec(codec);
    let Ok(mut enc) = ctx.encoder().video() else {
        return false;
    };
    enc.set_width(128);
    enc.set_height(128);
    enc.set_format(input_format(name));
    enc.set_time_base(ff::Rational::new(1, 25));
    enc.set_frame_rate(Some(ff::Rational::new(25, 1)));
    enc.open_with(ff::Dictionary::new()).is_ok()
}

/// Hardware encoders available on this machine, in preference order.
pub fn hardware_encoders() -> Vec<HwEncoder> {
    init();
    HW_ENCODERS
        .iter()
        .filter(|(name, _, _)| probe_encoder(name))
        .map(|(name, codec, api)| HwEncoder {
            name: name.to_string(),
            codec: codec.to_string(),
            api: api.to_string(),
        })
        .collect()
}

/// Hardware decoders compiled into this FFmpeg (by name; drivers are not probed).
pub fn hardware_decoders() -> Vec<String> {
    init();
    [
        "h264_cuvid",
        "hevc_cuvid",
        "h264_qsv",
        "hevc_qsv",
        "h264_videotoolbox",
        "hevc_videotoolbox",
    ]
    .iter()
    .filter(|n| ff::decoder::find_by_name(n).is_some())
    .map(|n| n.to_string())
    .collect()
}

/// Rate-control options for an encoder at a CRF-like `quality`.
fn rate_options(
    encoder: &str,
    quality: u8,
    width: u32,
    height: u32,
    fps: f64,
) -> ff::Dictionary<'static> {
    let mut opts = ff::Dictionary::new();
    if encoder == "libx264" || encoder == "libx265" {
        opts.set("preset", "medium");
        opts.set("crf", &quality.to_string());
    } else if encoder.contains("nvenc") {
        opts.set("rc", "vbr");
        opts.set("cq", &quality.to_string());
        opts.set("preset", "p4");
    } else if encoder.contains("qsv") {
        opts.set("global_quality", &quality.to_string());
    } else {
        // VideoToolbox, AMF: no CRF; target a bitrate from quality
        // (0.1 bit per pixel per frame at quality 23, halving every 6 steps).
        let bpp = 0.1 * 2f64.powf((23.0 - quality as f64) / 6.0);
        let bitrate = (bpp * width as f64 * height as f64 * fps) as i64;
        opts.set("b", &bitrate.to_string());
    }
    opts
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
    /// The encoder actually opened (requested one, or the software fallback).
    encoder_name: String,
}

impl FfmpegEncoder {
    pub fn encoder_name(&self) -> &str {
        &self.encoder_name
    }

    /// True when a requested hardware encoder could not open and software
    /// H.264 was used instead.
    pub fn used_fallback(&self) -> bool {
        self.settings
            .encoder
            .as_deref()
            .is_some_and(|e| e != self.encoder_name)
    }
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
        // Requested hardware encoder first, software H.264 as the fallback
        // (NFR-09): the first one that opens wins.
        let software = ff::encoder::find(ff::codec::Id::H264)
            .ok_or_else(|| Error::Unsupported("no H.264 encoder".into()))?;
        let mut candidates: Vec<(String, ff::Codec)> = Vec::new();
        if let Some(name) = &settings.encoder {
            if let Some(codec) = ff::encoder::find_by_name(name) {
                if probe_encoder(name) {
                    candidates.push((name.clone(), codec));
                }
            }
        }
        candidates.push((software.name().to_string(), software));
        let mut opened: Option<(String, ff::encoder::Video, ff::format::Pixel, ff::Rational)> =
            None;
        let mut last_err = None;
        for (name, codec) in candidates {
            let ctx = ff::codec::context::Context::new_with_codec(codec);
            let Ok(mut enc) = ctx.encoder().video() else {
                continue;
            };
            let format = input_format(&name);
            enc.set_width(settings.width);
            enc.set_height(settings.height);
            enc.set_format(format);
            let time_base = ff::Rational::new(fr.den as i32, fr.num as i32);
            enc.set_time_base(time_base);
            enc.set_frame_rate(Some(ff::Rational::new(fr.num as i32, fr.den as i32)));
            enc.set_gop(12);
            if global_header {
                enc.set_flags(ff::codec::Flags::GLOBAL_HEADER);
            }
            let opts = rate_options(
                &name,
                settings.crf,
                settings.width,
                settings.height,
                fr.as_f64(),
            );
            match enc.open_with(opts) {
                Ok(e) => {
                    opened = Some((name, e, format, time_base));
                    break;
                }
                Err(e) => last_err = Some(e),
            }
        }
        let (encoder_name, encoder, format, time_base) = opened.ok_or_else(|| {
            err(last_err.map_or("no encoder could open".to_string(), |e| e.to_string()))
        })?;
        let video = {
            let mut stream = output
                .add_stream(ff::encoder::find_by_name(&encoder_name).unwrap_or(software))
                .map_err(err)?;
            stream.set_parameters(&encoder);
            stream.set_time_base(time_base);
            stream.set_rate(ff::Rational::new(fr.num as i32, fr.den as i32));
            stream.set_avg_frame_rate(ff::Rational::new(fr.num as i32, fr.den as i32));
            let scaler = ff::software::scaling::Context::get(
                ff::format::Pixel::RGBA,
                settings.width,
                settings.height,
                format,
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
            encoder_name,
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

    fn encoder_name(&self) -> &str {
        FfmpegEncoder::encoder_name(self)
    }

    fn used_fallback(&self) -> bool {
        FfmpegEncoder::used_fallback(self)
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
    fn hardware_decode_matches_software_or_falls_back() {
        let frames = |mut d: FfmpegDecoder| {
            let mut out = Vec::new();
            while let Some(f) = d.next_video().unwrap() {
                out.push((f.pts, f.rgba8));
            }
            (out, d.decode_path())
        };
        let (soft, path) = frames(FfmpegDecoder::open(FIXTURE).unwrap());
        assert_eq!(path, DecodePath::Software);
        // Vulkan is the one device API a machine without a GPU can open
        // (Mesa's software driver); it has no video decode queue, so this
        // exercises the device set-up and libavcodec's fall back to software.
        // On a machine with a decoding GPU the frames come from hardware.
        std::env::set_var("DEBUT_HWACCEL", "vulkan");
        let d = FfmpegDecoder::open_with(FIXTURE, true).unwrap();
        // Until a frame is decoded only the request is known.
        assert!(matches!(
            d.decode_path(),
            DecodePath::Requested(_) | DecodePath::Fallback { .. }
        ));
        let (hard, path) = frames(d);
        assert_eq!(hard.len(), soft.len());
        for ((pa, a), (pb, b)) in hard.iter().zip(&soft) {
            assert_eq!(pa, pb);
            // A hardware decoder may round chroma differently.
            let worst = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
            assert!(worst <= 8, "frame at {pa:?} differs by {worst}");
        }
        match path {
            DecodePath::Hardware(name) => assert_eq!(name, "vulkan"),
            DecodePath::Fallback { wanted, reason } => {
                assert_eq!(
                    wanted == "vulkan",
                    reason.contains("does not decode"),
                    "{reason}"
                );
            }
            other => panic!("after decoding: {other:?}"),
        }
        // An API FFmpeg lacks: software, with the reason.
        std::env::set_var("DEBUT_HWACCEL", "teleport");
        let (again, path) = frames(FfmpegDecoder::open_with(FIXTURE, true).unwrap());
        std::env::remove_var("DEBUT_HWACCEL");
        assert_eq!(again, soft);
        assert_eq!(
            path,
            DecodePath::Fallback {
                wanted: "hardware".into(),
                reason: "FFmpeg has no teleport support".into()
            }
        );
    }

    #[test]
    fn tells_variable_frame_rate_and_still_decodes_from_the_start() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/");
        assert!(
            !FfmpegDecoder::open(FIXTURE)
                .unwrap()
                .video_info()
                .unwrap()
                .variable_frame_rate
        );
        let mut d = FfmpegDecoder::open(format!("{dir}vfr_64x36_2s.mp4")).unwrap();
        assert!(d.video_info().unwrap().variable_frame_rate);
        // The scan rewound: every frame comes, from the first.
        let mut pts = Vec::new();
        while let Some(f) = d.next_video().unwrap() {
            pts.push(f.pts);
        }
        assert_eq!(pts.len(), 45);
        assert_eq!(pts[0], Rational::ZERO);
        // The gap where frames 10..19 were dropped.
        assert_eq!(
            (pts[9], pts[10]),
            (Rational::new(9, 30), Rational::new(20, 30))
        );
    }

    #[test]
    fn opens_stills_and_audio_only_files() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/");
        // A PNG: one frame, no duration; any seek still yields it.
        let mut png = FfmpegDecoder::open(format!("{dir}still_64x36.png")).unwrap();
        let v = png.video_info().unwrap();
        assert_eq!((v.width, v.height, v.duration), (64, 36, Rational::ZERO));
        png.seek(Rational::from_int(2)).unwrap();
        assert!(png.next_video().unwrap().is_some());
        // A mono WAV with no channel layout in its header.
        let mut wav = FfmpegDecoder::open(format!("{dir}tone_16k_1500ms.wav")).unwrap();
        assert!(wav.video_info().is_none());
        let a = wav.audio_info().unwrap();
        assert_eq!(
            (a.channels, a.sample_rate, a.duration),
            (1, 16_000, Rational::new(3, 2))
        );
        let mut samples = 0;
        while let Some(b) = wav.next_audio().unwrap() {
            samples += b.samples.len();
        }
        assert_eq!(samples, 24_000);
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
    fn selecting_one_stream_leaves_the_other_unbuffered() {
        let mut d = FfmpegDecoder::open(FIXTURE).unwrap();
        d.select(true, false);
        let mut frames = 0;
        while d.next_video().unwrap().is_some() {
            frames += 1;
        }
        assert_eq!(frames, 50);
        assert!(d.audio_queue.is_empty());
        assert!(d.next_audio().unwrap().is_none());
        let mut d = FfmpegDecoder::open(FIXTURE).unwrap();
        d.select(false, true);
        let mut blocks = 0;
        while d.next_audio().unwrap().is_some() {
            blocks += 1;
        }
        assert!(blocks > 0 && d.video_queue.is_empty());
    }

    #[test]
    fn reads_timecode_and_reel_tags() {
        let tc = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/tc_b_25fps_2s.mov"
        );
        let tags = FfmpegDecoder::open(tc).unwrap().tags();
        assert_eq!(tags.timecode.as_deref(), Some("10:00:00:12"));
        assert_eq!(tags.reel.as_deref(), Some("CAMB"));
        assert_eq!(FfmpegDecoder::open(FIXTURE).unwrap().tags().timecode, None);
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
            encoder: None,
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

    #[test]
    fn hardware_encoder_detection_and_software_fallback() {
        // Whatever this machine has, the probe must not panic and every entry
        // must name a known codec family.
        for hw in hardware_encoders() {
            assert!(hw.codec == "h264" || hw.codec == "hevc", "{hw:?}");
            assert!(hw.name.contains(&hw.codec));
        }
        let _ = hardware_decoders();
        // Asking for an encoder that is not usable here lands on software H.264.
        let dir = std::env::temp_dir().join(format!("debut-hw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fallback.mp4");
        let enc = FfmpegEncoder::create(
            &path,
            EncodeSettings {
                width: 64,
                height: 36,
                frame_rate: FrameRate::FPS_25,
                crf: 23,
                audio: None,
                encoder: Some("h264_definitely_not_an_encoder".into()),
            },
        )
        .unwrap();
        assert!(enc.used_fallback());
        assert_eq!(enc.encoder_name(), "libx264");
        drop(enc);
        std::fs::remove_dir_all(dir).ok();
    }
}
