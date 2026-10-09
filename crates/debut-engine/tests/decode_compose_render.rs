//! End to end on the CPU: FFmpeg decode -> linearize -> compose -> render.

use debut_core::{FrameRate, IdGen, MediaId, Rational};
use debut_engine::FrameSource;
use debut_platform_native::codec::FfmpegDecoder;
use debut_project::{Clip, ClipSource, Sequence, Track, TrackKind};
use debut_render::compose::{compose, SourceInfo};
use debut_render::{Backend, CpuBackend};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../debut-platform-native/tests/fixtures/test_25fps_2s.mp4"
);

struct Dims<'a>(&'a FrameSource);
impl SourceInfo for Dims<'_> {
    fn dimensions(&self, media: MediaId) -> (u32, u32) {
        self.0.dimensions(media).unwrap()
    }
}

#[test]
fn renders_a_decoded_frame_through_the_graph() {
    let mut ids = IdGen::new(11);
    let media: MediaId = ids.fresh();
    let mut frames = FrameSource::new(4);
    frames
        .add(media, Box::new(FfmpegDecoder::open(FIXTURE).unwrap()))
        .unwrap();

    // 128x72 canvas; the 64x36 clip starts 1 s into the timeline, 0.5 s into the source.
    let mut seq = Sequence::new(ids.fresh(), "e2e", FrameRate::FPS_25, 128, 72);
    let mut track = Track::new(ids.fresh(), TrackKind::Video);
    track.clips.push(Clip::new(
        ids.fresh(),
        ClipSource::Media(media),
        Rational::from_int(1),
        Rational::from_int(1),
        Rational::new(1, 2),
    ));
    seq.tracks.push(track);

    let mut be = CpuBackend;

    // Before the clip: black.
    let g = compose(&seq, Rational::new(1, 2), &Dims(&frames));
    let img = g.render(&mut be, &mut frames).unwrap();
    let px = be.download(&img);
    assert!(px.iter().all(|p| *p == [0.0, 0.0, 0.0, 1.0]));

    // Inside the clip: the test pattern, scaled 2x to fill the canvas.
    let g = compose(&seq, Rational::new(5, 4), &Dims(&frames));
    let img = g.render(&mut be, &mut frames).unwrap();
    assert_eq!(be.size(&img), (128, 72));
    let px = be.download(&img);
    let nonblack = px.iter().filter(|p| p[0] + p[1] + p[2] > 0.05).count();
    assert!(
        nonblack > px.len() / 2,
        "{nonblack} of {} pixels lit",
        px.len()
    );
    assert!(px.iter().all(|p| (p[3] - 1.0).abs() < 1e-6));

    // Two consecutive requests one frame apart hit the forward-decode path and differ
    // (testsrc2 animates every frame).
    let a = frames_at(&mut frames, &seq, &mut be, Rational::new(5, 4));
    let b = frames_at(
        &mut frames,
        &seq,
        &mut be,
        Rational::new(5, 4) + FrameRate::FPS_25.frame_duration(),
    );
    assert_ne!(a, b);
    // And going back is served from the cache with identical output.
    let a2 = frames_at(&mut frames, &seq, &mut be, Rational::new(5, 4));
    assert_eq!(a, a2);
}

fn frames_at(
    frames: &mut FrameSource,
    seq: &Sequence,
    be: &mut CpuBackend,
    t: Rational,
) -> Vec<[f32; 4]> {
    let g = compose(seq, t, &Dims(frames));
    let img = g.render(be, frames).unwrap();
    be.download(&img)
}
