//! Plays the fixture through the whole stack with a simulated audio device:
//! Player::tick on the "UI thread", RtSink::fill as the "device callback".

use debut_audio::CHANNELS;
use debut_core::{FrameRate, IdGen, MediaId, Rational};
use debut_engine::Player;
use debut_platform::audio_out::AudioCallback;
use debut_platform_native::codec::FfmpegDecoder;
use debut_project::{Clip, ClipSource, Sequence, Track, TrackKind};
use debut_render::{Backend, CpuBackend};
use std::sync::atomic::Ordering;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../debut-platform-native/tests/fixtures/test_25fps_2s.mp4"
);

fn sequence(ids: &mut IdGen, media: MediaId) -> Sequence {
    let mut seq = Sequence::new(ids.fresh(), "play", FrameRate::FPS_25, 64, 36);
    let mut v = Track::new(ids.fresh(), TrackKind::Video);
    let mut a = Track::new(ids.fresh(), TrackKind::Audio);
    // Both tracks: the 2 s clip placed at 0.5 s.
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
    seq
}

#[test]
fn plays_video_and_audio_in_sync() {
    let mut ids = IdGen::new(21);
    let media: MediaId = ids.fresh();
    let (mut player, mut sink) = Player::new(sequence(&mut ids, media));
    player
        .add_media(
            media,
            Some(Box::new(FfmpegDecoder::open(FIXTURE).unwrap())),
            Some(Box::new(FfmpegDecoder::open(FIXTURE).unwrap())),
        )
        .unwrap();
    let underruns = sink.underruns();
    let mut be = CpuBackend;

    player.play();
    // Simulate 1.2 s: device pulls 480 frames per callback (10 ms), UI ticks every 4 callbacks (40 ms = 1 frame).
    let mut device = vec![0.0f32; 480 * CHANNELS];
    let mut peak_before_clip = 0.0f32;
    let mut peak_in_clip = 0.0f32;
    let mut presented = Vec::new();
    for i in 0..120 {
        if i % 4 == 0 {
            if let Some(graph) = player.tick().unwrap() {
                let img = graph.render(&mut be, &mut player.frames).unwrap();
                assert_eq!(be.size(&img), (64, 36));
                presented.push(player.transport.current_frame());
            }
        }
        sink.fill(&mut device, CHANNELS as u16, 48_000);
        let peak = device.iter().fold(0.0f32, |p, s| p.max(s.abs()));
        if i < 45 {
            peak_before_clip = peak_before_clip.max(peak);
        } else if i >= 60 {
            peak_in_clip = peak_in_clip.max(peak);
        }
    }

    assert_eq!(
        underruns.load(Ordering::Relaxed),
        0,
        "audio must never glitch"
    );
    assert_eq!(
        peak_before_clip, 0.0,
        "silence before the clip starts at 0.5 s"
    );
    assert!(
        peak_in_clip > 0.1,
        "the 440 Hz tone plays inside the clip: peak {peak_in_clip}"
    );
    // 1.2 s at 25 fps = 30 frames presented in order without gaps.
    assert_eq!(presented.first(), Some(&0));
    assert!(
        presented.windows(2).all(|w| w[1] == w[0] + 1),
        "{presented:?}"
    );
    assert_eq!(player.stats().dropped, 0);
    assert_eq!(
        player.transport.position(),
        Rational::new(120 * 480, 48_000)
    );

    // Pause: no more audio, position frozen on a frame boundary.
    player.pause();
    sink.fill(&mut device, CHANNELS as u16, 48_000);
    assert!(device.iter().all(|s| *s == 0.0));
    assert_eq!(
        player.transport.position(),
        FrameRate::FPS_25.snap(Rational::new(120 * 480, 48_000))
    );
}
