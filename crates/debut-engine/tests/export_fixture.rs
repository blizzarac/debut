//! Export a sequence built on the fixture to an MP4 and decode it back.

use debut_core::{FrameRate, IdGen, MediaId, Rational};
use debut_engine::{FrameSource, SampleCache};
use debut_export::{export, Control, ExportJob};
use debut_platform::{Decoder, Encoder};
use debut_platform_native::codec::{
    AudioEncodeSettings, EncodeSettings, FfmpegDecoder, FfmpegEncoder,
};
use debut_project::{Clip, ClipSource, Sequence, Track, TrackKind};
use debut_render::CpuBackend;
use std::collections::HashMap;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../debut-platform-native/tests/fixtures/test_25fps_2s.mp4"
);

#[test]
fn exports_a_range_with_audio_and_video() {
    let mut ids = IdGen::new(31);
    let media: MediaId = ids.fresh();
    let mut seq = Sequence::new(ids.fresh(), "export", FrameRate::FPS_25, 64, 36);
    let mut v = Track::new(ids.fresh(), TrackKind::Video);
    let mut a = Track::new(ids.fresh(), TrackKind::Audio);
    v.clips.push(Clip::new(
        ids.fresh(),
        ClipSource::Media(media),
        Rational::new(1, 2),
        Rational::from_int(2),
        Rational::ZERO,
    ));
    a.clips.push(Clip::new(
        ids.fresh(),
        ClipSource::Media(media),
        Rational::new(1, 2),
        Rational::from_int(2),
        Rational::ZERO,
    ));
    seq.tracks.push(v);
    seq.tracks.push(a);

    let mut frames = FrameSource::new(4);
    frames
        .add(media, Box::new(FfmpegDecoder::open(FIXTURE).unwrap()))
        .unwrap();
    let mut samples = SampleCache::new(48_000);
    samples
        .add(media, Box::new(FfmpegDecoder::open(FIXTURE).unwrap()))
        .unwrap();

    // Export 0.2 s .. 1.2 s: 25 frames, the first 7-8 black/silent, the rest from the clip.
    let job = ExportJob {
        sequence: seq,
        range: (Rational::new(1, 5), Rational::new(6, 5)),
        sample_rate: 48_000,
        mixes: HashMap::new(),
    };
    let path = std::env::temp_dir().join(format!("debut-export-{}.mp4", std::process::id()));
    let mut encoder = FfmpegEncoder::create(
        &path,
        EncodeSettings {
            width: 64,
            height: 36,
            frame_rate: FrameRate::FPS_25,
            crf: 20,
            audio: Some(AudioEncodeSettings {
                channels: 2,
                sample_rate: 48_000,
                bitrate: 64_000,
            }),
        },
    )
    .unwrap();
    let mut ticks = 0;
    let progress = export(
        &job,
        &mut CpuBackend,
        &mut frames,
        &mut samples,
        &mut encoder,
        &Control::default(),
        |_| ticks += 1,
    )
    .unwrap();
    Box::new(encoder).finish().unwrap();
    assert_eq!(
        (progress.frames_done, progress.frames_total, ticks),
        (25, 25, 25)
    );

    let mut dec = FfmpegDecoder::open(&path).unwrap();
    let info = dec.video_info().unwrap().clone();
    assert_eq!(
        (info.width, info.height, info.frame_rate),
        (64, 36, FrameRate::FPS_25)
    );
    let mut n = 0;
    let mut lit_after_start = 0;
    while let Some(f) = dec.next_video().unwrap() {
        let bright = f
            .rgba8
            .chunks(4)
            .filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 60)
            .count();
        if n < 7 {
            assert!(
                bright < 20,
                "frame {n} should be black before the clip ({bright} lit)"
            );
        } else if n >= 9 {
            lit_after_start += (bright > 500) as u32;
        }
        n += 1;
    }
    assert_eq!(n, 25);
    assert!(
        lit_after_start >= 14,
        "{lit_after_start} lit frames after the clip starts"
    );

    let mut total = 0;
    let mut peak_first = 0.0f32;
    let mut peak_later = 0.0f32;
    while let Some(b) = dec.next_audio().unwrap() {
        let frames_in = b.samples.len() / 2;
        let peak = b.samples.iter().fold(0.0f32, |p, s| p.max(s.abs()));
        if total + frames_in < 12_000 {
            peak_first = peak_first.max(peak);
        } else if total > 20_000 {
            peak_later = peak_later.max(peak);
        }
        total += frames_in;
    }
    assert!(
        (44_000..=52_000).contains(&total),
        "{total} audio frames (expected ~48 000)"
    );
    assert!(
        peak_first < 0.02,
        "silence before the clip, got {peak_first}"
    );
    assert!(peak_later > 0.05, "tone inside the clip, got {peak_later}");
    std::fs::remove_file(path).ok();
}
