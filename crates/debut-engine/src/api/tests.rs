//! Session API tests over the native platform (FFmpeg fixture): one focused
//! test per area, each starting from the same small project.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)] // tests use the OS directly
use super::*;
use debut_platform_native::NativePlatform;

fn native() -> Session {
    Session::new(Arc::new(NativePlatform::new()))
}

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../debut-platform-native/tests/fixtures/test_25fps_2s.mp4"
);

/// Base state for every test: a project at its own path with the fixture
/// imported, a sequence sized from it, and the clip on V1 and A1 at 0.4 s.
struct Fx {
    s: Session,
    dir: TempDir,
    path: String,
    v: String,
    a: String,
    m: MediaDto,
    seq: SequenceDto,
    clip_id: String,
}

fn fixture(name: &str) -> Fx {
    let dir = std::env::temp_dir().join(format!("debut-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.debut").to_string_lossy().into_owned();
    let mut s = native();
    let id = s.ids.fresh();
    s.start(Project::new(id, "t"), path.clone()).unwrap();
    assert_eq!(s.file_status().path.as_deref(), Some(path.as_str()));
    let m = s.import_media(FIXTURE.to_string()).unwrap();
    assert_eq!((m.width, m.height, m.has_audio), (64, 36, true));
    let seq = s.ensure_sequence().unwrap();
    assert_eq!((seq.width, seq.height, seq.frame_rate), (64, 36, [25, 1]));
    assert_eq!(seq.tracks.len(), 2);
    let (v, a) = (seq.tracks[0].id.clone(), seq.tracks[1].id.clone());
    s.add_clip(&v, &m.id, 0.4).unwrap();
    s.add_clip(&a, &m.id, 0.4).unwrap();
    let seq = sequence_dto(s.first_sequence().unwrap());
    assert_eq!(seq.tracks[0].clips[0].timeline_in, 0.4);
    assert_eq!(seq.duration, 2.4);
    // Pixel checks index a 64x36 frame; Auto may step down on a loaded machine.
    s.set_preview_quality(PreviewQuality::Full);
    let clip_id = seq.tracks[0].clips[0].id.clone();
    Fx {
        s,
        dir: TempDir(dir),
        path,
        v,
        a,
        m,
        seq,
        clip_id,
    }
}

/// A per-test scratch directory, removed when the test ends.
struct TempDir(std::path::PathBuf);

impl std::ops::Deref for TempDir {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl AsRef<std::path::Path> for TempDir {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// Sum of RGB over a frame.
fn sum(px: &[u8]) -> u64 {
    px.chunks(4)
        .map(|p| p[0] as u64 + p[1] as u64 + p[2] as u64)
        .sum::<u64>()
}

/// RGB sum of one pixel of a 64-wide frame.
fn at(px: &[u8], x: usize, y: usize) -> u32 {
    let i = (y * 64 + x) * 4;
    px[i] as u32 + px[i + 1] as u32 + px[i + 2] as u32
}

/// Number of differing bytes.
fn changed(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).filter(|(x, y)| x != y).count()
}

/// Frames before and inside the clip, GPU/CPU agreement (PLT-04), playback clock.
#[test]
fn playback_shows_black_then_picture_and_the_clock_runs() {
    let Fx { mut s, .. } = fixture("playback_shows_black_then_picture_and_the_clock_runs");
    // Before the clip: black. Inside: picture.
    s.transport(TransportAction::Seek { t: 0.0 }).unwrap();
    let (w, h, px) = s.frame_pixels().unwrap();
    assert_eq!((w, h), (64, 36));
    assert!(px.chunks(4).all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0));
    s.transport(TransportAction::Seek { t: 1.0 }).unwrap();
    let (_, _, px) = s.frame_pixels().unwrap();
    assert!(
        px.chunks(4)
            .filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 60)
            .count()
            > 500
    );
    // Whatever backend was detected, the CPU reference must agree (PLT-04).
    if s.backend.is_gpu() {
        s.backend = AnyBackend::Cpu(debut_render::CpuBackend);
        let (_, _, cpu) = s.frame_pixels().unwrap();
        let worst = px
            .iter()
            .zip(&cpu)
            .map(|(a, b)| (*a as i32 - *b as i32).abs())
            .max()
            .unwrap();
        assert!(worst <= 1, "GPU and CPU frames differ by {worst}/255");
        s.backend = AnyBackend::detect();
    }

    // Play for ~300 ms of wall time against the silent output: the clock advances.
    s.transport(TransportAction::Play).unwrap();
    let t0 = s.tick().unwrap();
    assert!(t0.playing);
    std::thread::sleep(std::time::Duration::from_millis(300));
    let t1 = s.tick().unwrap();
    assert!(
        t1.position > t0.position + 0.2,
        "{} -> {}",
        t0.position,
        t1.position
    );
    s.transport(TransportAction::Pause).unwrap();
    assert!(!s.tick().unwrap().playing);
}

/// Grade constant plus keyframed opacity; removing an effect brightens the frame.
#[test]
fn effects_grade_and_keyframed_opacity() {
    let Fx {
        mut s, v, clip_id, ..
    } = fixture("effects_grade_and_keyframed_opacity");
    // Effects: add a grade, set exposure as a constant, then keyframe opacity.
    s.add_effect(&v, &clip_id, "grade").unwrap();
    s.add_effect(&v, &clip_id, "transform").unwrap();
    s.set_param(&v, &clip_id, 0, Param::Exposure, 1.0, false)
        .unwrap();
    s.transport(TransportAction::Seek { t: 1.0 }).unwrap();
    s.set_param(&v, &clip_id, 1, Param::Opacity, 0.2, true)
        .unwrap();
    s.transport(TransportAction::Seek { t: 2.0 }).unwrap();
    s.set_param(&v, &clip_id, 1, Param::Opacity, 1.0, true)
        .unwrap();
    // 1.4 s = frame 35 exactly: clip-local 1.0, 40% of the way from the 0.2 key to the 1.0 key.
    s.transport(TransportAction::Seek { t: 1.4 }).unwrap();
    let fx = s.clip_effects(&v, &clip_id).unwrap();
    assert_eq!(
        (fx[0].kind.as_str(), fx[1].kind.as_str()),
        ("grade", "transform")
    );
    let exposure = fx[0]
        .params
        .iter()
        .find(|p| p.name == Param::Exposure)
        .unwrap();
    assert_eq!((exposure.value, exposure.animated), (1.0, false));
    let opacity = fx[1]
        .params
        .iter()
        .find(|p| p.name == Param::Opacity)
        .unwrap();
    assert!(
        (opacity.value - 0.52).abs() < 1e-9 && opacity.animated,
        "{:?}",
        opacity.value
    );
    // The graded, half-transparent frame is brighter than black but dimmer than full.
    let (_, _, half) = s.frame_pixels().unwrap();
    assert!(sum(&half) > 0);
    s.remove_effect(&v, &clip_id, 1).unwrap();
    let (_, _, full) = s.frame_pixels().unwrap();
    assert!(
        sum(&full) > sum(&half),
        "removing the opacity ramp brightens the frame"
    );
    assert!(s.remove_effect(&v, &clip_id, 7).is_err());
}

/// Shape and polygon masks (FX-04), tracking (FX-06), keyer (FX-05).
#[test]
fn masks_tracking_and_keying() {
    let Fx {
        mut s, v, clip_id, ..
    } = fixture("masks_tracking_and_keying");
    // Inside the clip, with a +1 stop grade in slot 0: the mask lands in slot 1
    // and the fixture's dark centre is bright enough to tell kept from cut.
    s.transport(TransportAction::Seek { t: 1.4 }).unwrap();
    s.add_effect(&v, &clip_id, "grade").unwrap();
    s.set_param(&v, &clip_id, 0, Param::Exposure, 1.0, false)
        .unwrap();
    // Pixel checks below index a 64x36 frame, so pin the preview (Auto may
    // step down on a loaded machine).
    // Mask (FX-04): a 32x18 rectangle in the 64x36 frame blacks out the
    // corners and keeps the centre; inverting swaps that; options are undoable.
    s.add_effect(&v, &clip_id, "mask").unwrap();
    let mask_ix = s.clip_effects(&v, &clip_id).unwrap().len() - 1;
    s.set_param(&v, &clip_id, mask_ix, Param::MaskWidth, 32.0, false)
        .unwrap();
    s.set_param(&v, &clip_id, mask_ix, Param::MaskHeight, 18.0, false)
        .unwrap();
    s.set_param(&v, &clip_id, mask_ix, Param::Feather, 0.0, false)
        .unwrap();
    let (_, _, masked) = s.frame_pixels().unwrap();
    assert_eq!(at(&masked, 1, 1), 0, "corner is masked out");
    assert!(at(&masked, 32, 18) > 60, "centre is kept");
    s.set_effect_options(
        &v,
        &clip_id,
        mask_ix,
        EffectOptions {
            invert: Some(true),
            shape: Some(MaskShape::Ellipse),
            ..Default::default()
        },
    )
    .unwrap();
    let fx = s.clip_effects(&v, &clip_id).unwrap();
    assert_eq!(
        (fx[mask_ix].options.invert, fx[mask_ix].options.shape),
        (Some(true), Some(MaskShape::Ellipse))
    );
    let (_, _, inverted) = s.frame_pixels().unwrap();
    assert_eq!(at(&inverted, 32, 18), 0, "centre is now cut out");
    assert!(
        at(&inverted, 1, 1) > 0 || at(&inverted, 2, 30) > 0,
        "corners show"
    );
    // Tracking (FX-06): the fixture's pattern is static apart from its
    // digits, so a track over a static patch keeps the mask put and the
    // keys land on frames.
    s.set_effect_options(
        &v,
        &clip_id,
        mask_ix,
        EffectOptions {
            invert: Some(false),
            shape: Some(MaskShape::Rectangle),
            ..Default::default()
        },
    )
    .unwrap();
    s.set_param(&v, &clip_id, mask_ix, Param::MaskX, -10.0, false)
        .unwrap();
    s.transport(TransportAction::Seek { t: 1.0 }).unwrap();
    let tr = s.track_mask(&v, &clip_id, mask_ix, 0.4).unwrap();
    assert_eq!(tr.keys, 10, "{tr:?}");
    assert!(tr.weakest_match > 0.5, "{tr:?}");
    let fx = s.clip_effects(&v, &clip_id).unwrap();
    let mx = fx[mask_ix]
        .params
        .iter()
        .find(|p| p.name == Param::MaskX)
        .unwrap();
    assert!(
        mx.animated && (mx.value + 10.0).abs() < 3.0,
        "{:?}",
        mx.value
    );
    assert!(
        s.track_mask(&v, &clip_id, 0, 1.0).is_err(),
        "grade is not a mask"
    );
    s.workspace_mut().unwrap().undo().unwrap();
    s.sync_player().unwrap();
    assert!(
        !s.clip_effects(&v, &clip_id).unwrap()[mask_ix]
            .params
            .iter()
            .find(|p| p.name == Param::MaskX)
            .unwrap()
            .animated
    );
    // Polygon: switching with no points seeds a diamond; a thin triangle at the
    // left edge leaves the right side black.
    s.set_effect_options(
        &v,
        &clip_id,
        mask_ix,
        EffectOptions {
            shape: Some(MaskShape::Polygon),
            invert: Some(false),
            ..Default::default()
        },
    )
    .unwrap();
    let fx = s.clip_effects(&v, &clip_id).unwrap();
    assert_eq!(
        fx[mask_ix].options.points.as_ref().map(|p| p.len()),
        Some(4)
    );
    s.set_effect_options(
        &v,
        &clip_id,
        mask_ix,
        EffectOptions {
            points: Some(vec![[-32.0, -18.0], [-8.0, 0.0], [-32.0, 18.0]]),
            ..Default::default()
        },
    )
    .unwrap();
    let (_, _, tri) = s.frame_pixels().unwrap();
    assert!(at(&tri, 4, 18) > 0, "inside the triangle");
    assert_eq!(at(&tri, 60, 18), 0, "right side is cut");
    assert!(s
        .set_effect_options(
            &v,
            &clip_id,
            mask_ix,
            EffectOptions {
                points: Some(vec![[0.0, 0.0]; 40]),
                ..Default::default()
            }
        )
        .is_err());
    assert!(s
        .set_effect_options(&v, &clip_id, 0, EffectOptions::default())
        .is_err());
    s.workspace_mut().unwrap().undo().unwrap();
    s.sync_player().unwrap();
    assert_eq!(
        s.clip_effects(&v, &clip_id).unwrap()[mask_ix]
            .options
            .invert,
        Some(false)
    );
    s.remove_effect(&v, &clip_id, mask_ix).unwrap();
    // Keyer (FX-05) attaches and reports its colour.
    s.add_effect(&v, &clip_id, "key").unwrap();
    let fx = s.clip_effects(&v, &clip_id).unwrap();
    assert_eq!(fx.last().unwrap().options.color, Some([0, 255, 0]));
    s.remove_effect(&v, &clip_id, fx.len() - 1).unwrap();
}

/// An edit through the IPC op and its undo; preview divisors (PB-03).
#[test]
fn timeline_edit_undo_and_preview_quality() {
    let Fx { mut s, v, seq, .. } = fixture("timeline_edit_undo_and_preview_quality");
    // Edit through the IPC op, then undo it.
    let clip = seq.tracks[0].clips[0].id.clone();
    s.edit(EditOp::RippleTail {
        track: v.clone(),
        clip,
        delta: -1.0,
    })
    .unwrap();
    assert_eq!(
        sequence_dto(s.first_sequence().unwrap()).tracks[0].clips[0].duration,
        1.0
    );
    s.workspace_mut().unwrap().undo().unwrap();
    s.sync_player().unwrap();
    assert_eq!(
        sequence_dto(s.first_sequence().unwrap()).tracks[0].clips[0].duration,
        2.0
    );
    assert_eq!(s.player.as_ref().unwrap().sequence.duration().as_f64(), 2.4);

    // Preview quality: fixed modes change the frame size; Auto keeps the divisor
    // until it has timings.
    s.set_preview_quality(PreviewQuality::Quarter);
    let (qw, qh, _) = s.frame_pixels().unwrap();
    assert_eq!((qw, qh), (16, 16), "64x36 / 4 clamps to the 16 px floor");
    s.set_preview_quality(PreviewQuality::Half);
    let (hw, hh, _) = s.frame_pixels().unwrap();
    assert_eq!((hw, hh), (32, 18));
    s.set_preview_quality(PreviewQuality::Auto);
    // Auto decides from the measured frame cost (software GPU here), so only the
    // set of divisors is fixed.
    assert!([1, 2, 4].contains(&s.tick().unwrap().preview_divisor));
    s.set_preview_quality(PreviewQuality::Full);
    let (fw, _, _) = s.frame_pixels().unwrap();
    assert_eq!(fw, 64);
}

/// Title clips (GFX-01), built-in and saved templates (GFX-02).
#[test]
fn titles_templates_and_saved_looks() {
    let Fx {
        mut s, v, clip_id, ..
    } = fixture("titles_templates_and_saved_looks");
    // Titles (GFX-01): the video track is busy at 1 s, so the title lands on a new
    // track above it; its text brightens the picture, and an edit re-renders it.
    let before = sequence_dto(s.first_sequence().unwrap());
    s.transport(TransportAction::Seek { t: 1.0 }).unwrap();
    let (_, _, plain) = s.frame_pixels().unwrap();
    let title_clip = s.add_title(1.0, "Hi".into()).unwrap();
    let after = sequence_dto(s.first_sequence().unwrap());
    assert_eq!(after.tracks.len(), before.tracks.len() + 1);
    let title_track = &after.tracks[1];
    assert_eq!(
        (title_track.kind.as_str(), after.tracks[2].kind.as_str()),
        ("video", "audio")
    );
    let t = title_track.clips[0].title.as_ref().unwrap();
    assert_eq!(
        (t.text.as_str(), title_track.clips[0].id.as_str()),
        ("Hi", title_clip.as_str())
    );
    s.transport(TransportAction::Seek { t: 2.0 }).unwrap();
    let (_, _, titled) = s.frame_pixels().unwrap();
    assert!(sum(&titled) > sum(&plain), "white text brightens the frame");
    assert!(titled
        .chunks(4)
        .any(|p| p[0] > 200 && p[1] > 200 && p[2] > 200));
    let mut edited = t.clone();
    edited.text = "A much longer line of text".into();
    edited.style.size_px = 8.0;
    s.set_title(&title_track.id, &title_clip, edited.clone())
        .unwrap();
    let now = sequence_dto(s.first_sequence().unwrap());
    assert_eq!(now.tracks[1].clips[0].title.as_ref(), Some(&edited));
    let (_, _, retitled) = s.frame_pixels().unwrap();
    assert_ne!(retitled, titled, "the new text renders differently");
    assert!(
        s.set_title(&v, &clip_id, edited).is_err(),
        "media clips have no title"
    );
    // A lower-third template lands with its animated Transform; at the clip's
    // start it is still off screen (frame unchanged), parked by 1 s in.
    assert!(s.title_templates().iter().any(|t| t.id == "lower_third"));
    assert!(s
        .add_title_from(0.0, "x".into(), Some("nope".into()))
        .is_err());
    s.transport(TransportAction::Seek { t: 1.0 }).unwrap();
    let (_, _, no_lt) = s.frame_pixels().unwrap();
    let lt = s
        .add_title_from(1.0, "Name".into(), Some("lower_third".into()))
        .unwrap();
    let after = sequence_dto(s.first_sequence().unwrap());
    let (lt_track, lt_clip) = after
        .tracks
        .iter()
        .find_map(|t| {
            t.clips
                .iter()
                .find(|c| c.id == lt)
                .map(|c| (t.id.clone(), c))
        })
        .unwrap();
    assert_eq!(lt_clip.title.as_ref().unwrap().style.min_width_px, 32.0);
    let fx = s.clip_effects(&lt_track, &lt).unwrap();
    assert_eq!(fx[0].kind, "transform");
    let (_, _, at_start) = s.frame_pixels().unwrap();
    assert_eq!(at_start, no_lt, "slides in from off screen");
    s.transport(TransportAction::Seek { t: 2.0 }).unwrap();
    let (_, _, parked) = s.frame_pixels().unwrap();
    assert_ne!(parked, no_lt, "visible once parked");
    // Save that look as a project template and make a new title from it.
    let saved = s
        .save_title_template(&lt_track, &lt, "My lower third".into())
        .unwrap();
    assert!(
        s.save_title_template(&v, &clip_id, "x".into()).is_err(),
        "media clips are not titles"
    );
    let list = s.title_templates();
    let mine = list.iter().find(|t| t.id == saved).unwrap();
    assert!(mine.saved && mine.description.contains("animated"));
    assert_eq!(
        list.iter().filter(|t| !t.saved).count(),
        debut_graphics::TEMPLATES.len()
    );
    let copy = s
        .add_title_from(3.0, "Again".into(), Some(saved.clone()))
        .unwrap();
    let dto = sequence_dto(s.first_sequence().unwrap());
    let (copy_track, copy_clip) = dto
        .tracks
        .iter()
        .find_map(|t| {
            t.clips
                .iter()
                .find(|c| c.id == copy)
                .map(|c| (t.id.clone(), c))
        })
        .unwrap();
    assert_eq!(copy_clip.title.as_ref().unwrap().style.min_width_px, 32.0);
    assert_eq!(s.clip_effects(&copy_track, &copy).unwrap().len(), 1);
    s.edit(EditOp::Lift {
        track: copy_track,
        start: 3.0,
        end: 8.0,
    })
    .unwrap();
    assert!(s.remove_title_template("title").is_err());
    s.remove_title_template(&saved).unwrap();
    assert!(s.title_templates().iter().all(|t| !t.saved));
    s.workspace_mut().unwrap().undo().unwrap();
    assert!(s.title_templates().iter().any(|t| t.saved));
    s.remove_title_template(&saved).unwrap();
    s.edit(EditOp::Lift {
        track: lt_track.clone(),
        start: 1.0,
        end: 6.0,
    })
    .unwrap();
}

/// Caption burn-in, settings, SRT/VTT round trip (GFX-05, GFX-06).
#[test]
fn captions_burn_in_settings_and_subtitle_files() {
    let Fx { mut s, dir, .. } = fixture("captions_burn_in_settings_and_subtitle_files");
    // Captions (GFX-05/06): burned in near the bottom while active, SRT round trip.
    s.transport(TransportAction::Seek { t: 1.4 }).unwrap();
    let (_, _, bare) = s.frame_pixels().unwrap();
    let cap = s.add_caption(1.0, 2.0, "Hello".into()).unwrap();
    let (_, h, captioned) = s.frame_pixels().unwrap();
    // On a 36-line frame the scaled caption box spans roughly rows 13..34.
    let split = (h as usize / 3) * 64 * 4;
    assert!(
        changed(&bare[split..], &captioned[split..]) > 20,
        "the lower part carries the caption"
    );
    assert_eq!(
        changed(&bare[..split], &captioned[..split]),
        0,
        "the top third is untouched"
    );
    s.transport(TransportAction::Seek { t: 0.5 }).unwrap();
    let (_, _, before) = s.frame_pixels().unwrap();
    s.remove_caption(&cap).unwrap();
    let (_, _, before_none) = s.frame_pixels().unwrap();
    assert_eq!(changed(&before, &before_none), 0, "nothing before the cue");
    let cap = s.add_caption(1.0, 2.0, "Hello".into()).unwrap();
    s.update_caption(
        &cap,
        CaptionEdit {
            start: Some(0.2),
            end: None,
            text: Some("Hi there".into()),
        },
    )
    .unwrap();
    assert_eq!(s.captions().unwrap()[0].text, "Hi there");
    assert!(s
        .update_caption(
            &cap,
            CaptionEdit {
                start: Some(3.0),
                end: None,
                text: None
            }
        )
        .is_err());
    let srt = dir.join("c.srt").to_string_lossy().into_owned();
    assert_eq!(s.export_srt(&srt).unwrap(), 1);
    assert_eq!(s.import_srt(&srt).unwrap(), 1);
    let caps = s.captions().unwrap();
    assert_eq!(caps.len(), 2);
    assert_eq!(
        (caps[1].start, caps[1].end, caps[1].text.as_str()),
        (0.2, 2.0, "Hi there")
    );
    s.remove_caption(&caps[1].id).unwrap();
    // Settings: top placement moves the change to the upper part; burn-in off
    // leaves the frame untouched; VTT export carries the header.
    s.transport(TransportAction::Seek { t: 1.4 }).unwrap();
    let mut cs = s.caption_settings().unwrap();
    assert!(cs.burn_in);
    cs.position = debut_project::CaptionPosition::Top;
    s.set_caption_settings(cs.clone()).unwrap();
    let (_, _, top) = s.frame_pixels().unwrap();
    assert!(
        changed(&bare[..split], &top[..split]) > 20,
        "caption at the top"
    );
    cs.burn_in = false;
    s.set_caption_settings(cs).unwrap();
    let (_, _, off) = s.frame_pixels().unwrap();
    assert_eq!(changed(&bare, &off), 0, "burn-in off");
    s.workspace_mut().unwrap().undo().unwrap();
    s.workspace_mut().unwrap().undo().unwrap();
    s.sync_player().unwrap();
    assert_eq!(s.caption_settings().unwrap(), CaptionSettings::default());
    let vtt = dir.join("c.vtt").to_string_lossy().into_owned();
    assert_eq!(s.export_srt(&vtt).unwrap(), 1);
    assert!(std::fs::read_to_string(&vtt)
        .unwrap()
        .starts_with("WEBVTT\n"));
    s.remove_caption(&cap).unwrap();
    assert!(s.captions().unwrap().is_empty());
}

/// Multicam clips, audio sync and the live switch (MED-11, TL-08).
#[test]
fn multicam_audio_sync_and_live_switch() {
    let Fx { mut s, v, m, .. } = fixture("multicam_audio_sync_and_live_switch");
    // Multicam (MED-11, TL-08): two angles of the same file on a fresh track
    // layout; a live switch at 1.0 s blades and switches only the tail.
    let m2 = s.import_media(FIXTURE.to_string()).unwrap();
    let mc_track = {
        let t = Track::new(s.ids.fresh(), TrackKind::Video);
        let id = t.id;
        let seq_id = s.first_sequence().unwrap().id;
        s.exec(Command::AddTrack {
            sequence: seq_id,
            track: t,
            index: Some(0),
        })
        .unwrap();
        id_str(id.0)
    };
    // add_multicam targets the first video track, which is now the empty one.
    // Synced first: identical takes line up at 0 with a confident match.
    let synced = s
        .add_multicam_synced(0.0, vec![m.id.clone(), m2.id.clone()], true)
        .unwrap();
    assert_eq!(
        synced.offsets,
        vec![0.0, 0.0],
        "identical takes line up at 0"
    );
    assert!(synced.confidences[0] > 0.9, "{:?}", synced.confidences);
    s.workspace_mut().unwrap().undo().unwrap();
    s.sync_player().unwrap();
    s.add_multicam(0.0, vec![m.id.clone(), m2.id.clone()])
        .unwrap();
    let dto = sequence_dto(s.first_sequence().unwrap());
    assert_eq!(dto.tracks[0].id, mc_track);
    let mc = &dto.tracks[0].clips[0];
    assert_eq!((mc.angles, mc.angle, mc.duration), (Some(2), Some(0), 2.0));
    assert!(
        dto.tracks
            .iter()
            .filter(|t| t.kind == "audio")
            .all(|t| t.clips.iter().any(|c| c.angles == Some(2))),
        "audio got a multicam clip too"
    );
    assert!(s.switch_angle(&mc_track, &mc.id, 5, false).is_err());
    s.transport(TransportAction::Seek { t: 1.0 }).unwrap();
    let tail = s.switch_angle(&mc_track, &mc.id, 1, true).unwrap();
    let dto = sequence_dto(s.first_sequence().unwrap());
    let clips = &dto.tracks[0].clips;
    assert_eq!(clips.len(), 2);
    assert_eq!(
        (clips[0].angle, clips[1].angle, clips[1].id.as_str()),
        (Some(0), Some(1), tail.as_str())
    );
    assert_eq!((clips[1].timeline_in, clips[1].duration), (1.0, 1.0));
    s.transport(TransportAction::Seek { t: 1.4 }).unwrap();
    let (_, _, px) = s.frame_pixels().unwrap();
    assert!(sum(&px) > 0, "angle 1 renders");
    s.workspace_mut().unwrap().undo().unwrap();
    s.sync_player().unwrap();
    assert_eq!(
        sequence_dto(s.first_sequence().unwrap()).tracks[0]
            .clips
            .len(),
        1
    );
    // Clear the multicam material again (and its track) for the checks below.
    s.workspace_mut().unwrap().undo().unwrap();
    s.workspace_mut().unwrap().undo().unwrap();
    s.sync_player().unwrap();
    assert_eq!(sequence_dto(s.first_sequence().unwrap()).tracks[0].id, v);
}

/// Bins (MED-07), keywords and ratings (MED-08), offline media and relink (MED-05).
#[test]
fn bins_tags_and_offline_media() {
    let Fx { mut s, m, dir, .. } = fixture("bins_tags_and_offline_media");
    // A second take of the same file, so the name-based smart bin matches two.
    s.import_media(FIXTURE.to_string()).unwrap();
    // Bins (MED-07): a manual bin takes an assignment, a smart bin matches by
    // name, the media list reports both, and removing a bin is undoable.
    let selects = s.add_bin("Selects".into(), None).unwrap();
    let tests = s.add_bin("Tests".into(), Some("test_".into())).unwrap();
    s.assign_media(&m.id, Some(selects.clone())).unwrap();
    assert!(
        s.assign_media(&m.id, Some(tests.clone())).is_err(),
        "smart bins refuse assignment"
    );
    let list = s.media_list().unwrap();
    assert_eq!(list.len(), 2);
    let first = list.iter().find(|x| x.id == m.id).unwrap();
    assert!(first.bins.contains(&selects) && first.bins.contains(&tests));
    assert_eq!((first.width, first.has_audio), (64, true));
    let bins = s.bins().unwrap();
    let by = |id: &str| bins.iter().find(|b| b.id == id).unwrap();
    assert_eq!((by(&selects).count, by(&selects).smart), (1, false));
    assert_eq!(
        (by(&tests).count, by(&tests).filter.as_deref()),
        (2, Some("name contains test_"))
    );
    s.rename_bin(&selects, "Keepers".into()).unwrap();
    s.assign_media(&m.id, None).unwrap();
    assert_eq!(
        s.bins()
            .unwrap()
            .iter()
            .find(|b| b.id == selects)
            .unwrap()
            .name,
        "Keepers"
    );
    s.remove_bin(&selects).unwrap();
    assert_eq!(s.bins().unwrap().len(), 1);
    s.workspace_mut().unwrap().undo().unwrap();
    assert_eq!(s.bins().unwrap().len(), 2);
    // Keywords and ratings feed smart bins (MED-08).
    s.set_media_tags(&m.id, vec!["hero".into(), " wide ".into(), "".into()], 5)
        .unwrap();
    assert!(s.set_media_tags(&m.id, vec![], 7).is_err());
    let tagged = s
        .media_list()
        .unwrap()
        .into_iter()
        .find(|x| x.id == m.id)
        .unwrap();
    assert_eq!(
        (tagged.keywords.as_slice(), tagged.rating),
        (["hero".to_string(), "wide".to_string()].as_slice(), 5)
    );
    let stars = s
        .add_smart_bin(
            "Five stars".into(),
            RuleDto {
                field: "rating".into(),
                op: "gte".into(),
                value: "5".into(),
            },
        )
        .unwrap();
    let heroes = s
        .add_smart_bin(
            "Hero".into(),
            RuleDto {
                field: "keyword".into(),
                op: "eq".into(),
                value: "Hero".into(),
            },
        )
        .unwrap();
    assert!(s
        .add_smart_bin(
            "bad".into(),
            RuleDto {
                field: "nope".into(),
                op: "eq".into(),
                value: "".into()
            }
        )
        .is_err());
    let bins = s.bins().unwrap();
    let by = |id: &str| bins.iter().find(|b| b.id == id).unwrap();
    assert_eq!((by(&stars).count, by(&heroes).count), (1, 1));
    assert_eq!(by(&stars).filter.as_deref(), Some("rating gte 5"));
    s.workspace_mut().unwrap().undo().unwrap();
    s.workspace_mut().unwrap().undo().unwrap();
    s.workspace_mut().unwrap().undo().unwrap();
    assert_eq!(
        s.media_list()
            .unwrap()
            .into_iter()
            .find(|x| x.id == m.id)
            .unwrap()
            .rating,
        0
    );
    s.remove_bin(&selects).unwrap();
    s.remove_bin(&tests).unwrap();

    // Offline media (MED-05): a file that vanishes shows a slate instead of
    // breaking playback; relinking brings the picture back.
    let moved = dir.join("moved.mp4");
    std::fs::copy(FIXTURE, &moved).unwrap();
    let m3 = s
        .import_media(moved.to_string_lossy().into_owned())
        .unwrap();
    let off_track = Track::new(s.ids.fresh(), TrackKind::Video);
    let off_track_id = id_str(off_track.id.0);
    let seq_id = s.first_sequence().unwrap().id;
    s.exec(Command::AddTrack {
        sequence: seq_id,
        track: off_track,
        index: None,
    })
    .unwrap();
    s.add_clip(&off_track_id, &m3.id, 0.0).unwrap();
    s.transport(TransportAction::Seek { t: 1.0 }).unwrap();
    let (_, _, linked) = s.frame_pixels().unwrap();
    std::fs::remove_file(&moved).unwrap();
    s.player = None;
    s.sync_player().unwrap();
    let m3_now = s
        .media_list()
        .unwrap()
        .into_iter()
        .find(|x| x.id == m3.id)
        .unwrap();
    assert!(!m3_now.online, "missing file is reported offline");
    let (_, _, slate) = s.frame_pixels().unwrap();
    assert_ne!(slate, linked);
    assert!(
        slate.chunks(4).all(|p| p[0] == p[1] && p[1] == p[2]),
        "grey slate on top"
    );
    assert!(s.relink_media(&m3.id, "/nowhere.mp4".into()).is_err());
    s.relink_media(&m3.id, FIXTURE.to_string()).unwrap();
    assert!(
        s.media_list()
            .unwrap()
            .into_iter()
            .find(|x| x.id == m3.id)
            .unwrap()
            .online
    );
    let (_, _, relinked) = s.frame_pixels().unwrap();
    // Colour is back: the slate was grey everywhere, the pattern is not.
    assert_ne!(relinked, slate);
    assert!(
        relinked
            .chunks(4)
            .any(|p| (p[0] as i32 - p[2] as i32).abs() > 50),
        "relinked footage is in colour again"
    );
    s.workspace_mut().unwrap().undo().unwrap(); // relink
    s.workspace_mut().unwrap().undo().unwrap(); // insert
    s.workspace_mut().unwrap().undo().unwrap(); // track
    s.sync_player().unwrap();
}

/// Nesting a range and opening the nested sequence (TL-07).
#[test]
fn nest_and_open_the_nested_sequence() {
    let Fx { mut s, .. } = fixture("nest_and_open_the_nested_sequence");
    // Nest 1.0..2.0 s (TL-07): a new sequence appears, V1/A1 get compound
    // clips there, the picture at 1.4 s survives, and undo restores the cut.
    let seqs_before = s.project().unwrap().sequences.len();
    s.transport(TransportAction::Seek { t: 1.4 }).unwrap();
    let (_, _, flat) = s.frame_pixels().unwrap();
    s.edit(EditOp::Nest {
        start: 1.0,
        end: 2.0,
    })
    .unwrap();
    let project = s.project().unwrap();
    assert_eq!(project.sequences.len(), seqs_before + 1);
    let dto = sequence_dto(s.first_sequence().unwrap());
    let v1 = &dto.tracks[0].clips;
    assert_eq!(v1.len(), 3, "head, compound, tail");
    assert_eq!(
        (v1[1].timeline_in, v1[1].duration, v1[1].nested.is_some()),
        (1.0, 1.0, true)
    );
    assert!(dto
        .tracks
        .iter()
        .filter(|t| t.kind == "audio")
        .any(|t| t.clips.iter().any(|c| c.nested.is_some())));
    let (_, _, nested_px) = s.frame_pixels().unwrap();
    let worst = flat
        .iter()
        .zip(&nested_px)
        .map(|(a, b)| (*a as i32 - *b as i32).abs())
        .max()
        .unwrap_or(0);
    assert!(
        worst <= 2,
        "nesting must not change the picture ({worst}/255)"
    );
    assert_eq!(dto.duration, 2.4);
    // Open the nested sequence in the timeline: its material starts at 0 and
    // renders; the list marks it active; opening the main one goes back.
    let list = s.sequences().unwrap();
    assert_eq!(list.len(), 2);
    let main_id = list.iter().find(|x| x.active).unwrap().id.clone();
    let nested_id = v1[1].nested.clone().unwrap();
    let inner = s.open_sequence(&nested_id).unwrap();
    assert_eq!(
        (inner.id.as_str(), inner.duration),
        (nested_id.as_str(), 1.0)
    );
    assert_eq!(inner.tracks[0].clips[0].timeline_in, 0.0);
    assert!(s
        .sequences()
        .unwrap()
        .iter()
        .any(|x| x.active && x.id == nested_id));
    s.transport(TransportAction::Seek { t: 0.4 }).unwrap();
    let (_, _, inner_px) = s.frame_pixels().unwrap();
    assert!(sum(&inner_px) > 0);
    assert!(s.open_sequence("nope").is_err());
    s.open_sequence(&main_id).unwrap();
    assert_eq!(sequence_dto(s.first_sequence().unwrap()).id, main_id);
    // Undoing the nest while the nested sequence is open falls back to main.
    s.open_sequence(&nested_id).unwrap();
    s.workspace_mut().unwrap().undo().unwrap();
    s.sync_player().unwrap();
    assert_eq!(sequence_dto(s.first_sequence().unwrap()).id, main_id);
    assert_eq!(
        sequence_dto(s.first_sequence().unwrap()).tracks[0]
            .clips
            .len(),
        1
    );
    s.active = None;
    assert_eq!(s.project().unwrap().sequences.len(), seqs_before);
}

/// Dissolve transitions with handle checks (FX-03).
#[test]
fn dissolve_needs_handles() {
    let Fx { mut s, v, .. } = fixture("dissolve_needs_handles");
    // Transition: blade V1 at 1.0 s, dissolve 0.4 s into the second piece (its head
    // handle is 0.6 s of source), then check the DTO and that scopes come back.
    s.edit(EditOp::Blade {
        track: v.clone(),
        at: 1.0,
    })
    .unwrap();
    assert_eq!(
        sequence_dto(s.first_sequence().unwrap()).tracks[0]
            .clips
            .len(),
        2
    );
    let second = sequence_dto(s.first_sequence().unwrap()).tracks[0].clips[1]
        .id
        .clone();
    s.edit(EditOp::Transition {
        track: v.clone(),
        clip: second.clone(),
        duration: Some(0.4),
    })
    .unwrap();
    assert_eq!(
        sequence_dto(s.first_sequence().unwrap()).tracks[0].clips[1].transition_in,
        Some(0.4)
    );
    assert!(
        s.edit(EditOp::Transition {
            track: v.clone(),
            clip: second.clone(),
            duration: Some(5.0)
        })
        .is_err(),
        "longer than the clip"
    );
    s.transport(TransportAction::Seek { t: 1.0 }).unwrap();
    let bytes = s.scopes().unwrap();
    assert_eq!(bytes.len(), 256 * 128 + 128 * 128 + 3 * 256 * 4);
    assert!(
        bytes[..256 * 128].iter().any(|b| *b > 0),
        "waveform has content"
    );
    s.workspace_mut().unwrap().undo().unwrap(); // the dissolve
    s.workspace_mut().unwrap().undo().unwrap(); // the blade
    s.sync_player().unwrap();
    assert_eq!(
        sequence_dto(s.first_sequence().unwrap()).tracks[0]
            .clips
            .len(),
        1
    );
}

/// Waveform peaks build in the background and answer range queries (AUD-04).
#[test]
fn waveform_peaks_build_in_the_background() {
    let Fx { mut s, m, .. } = fixture("waveform_peaks_build_in_the_background");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let peaks = loop {
        if let Some(p) = s.waveform(&m.id, 0.0, 2.0, 100).unwrap() {
            break p;
        }
        assert!(std::time::Instant::now() < deadline, "peaks never arrived");
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert_eq!(peaks.len(), 100);
    let loudest = peaks.iter().map(|b| b[1].max(-b[0])).fold(0.0f32, f32::max);
    assert!(loudest > 0.05, "the fixture's tone shows: {loudest}");
    assert!(peaks.iter().all(|b| b[0] <= b[1]));
    // Past the end of the file: silence; a second request is served from cache.
    let after = s.waveform(&m.id, 3.0, 4.0, 10).unwrap().unwrap();
    assert!(after.iter().all(|b| *b == [0.0, 0.0]));
    assert!(s.waveform("not-an-id", 0.0, 1.0, 10).is_err());
}

/// Snap points and closing gaps through the session (TL-06).
#[test]
fn snap_points_and_close_gaps() {
    let Fx {
        mut s,
        v,
        m,
        clip_id,
        ..
    } = fixture("snap_points_and_close_gaps");
    // Fixture: V1 holds 0.4..2.4. Add a second take at 4.0 on V1, leaving a gap.
    s.add_clip(&v, &m.id, 4.0).unwrap();
    s.transport(TransportAction::Seek { t: 1.0 }).unwrap();
    let pts = s.snap_points(None).unwrap();
    let at = |t: f64| {
        pts.iter()
            .find(|p| (p.t - t).abs() < 1e-9)
            .map(|p| p.kind.as_str())
    };
    assert_eq!(at(0.0), Some("start"));
    assert_eq!(at(1.0), Some("playhead"));
    assert_eq!(
        (at(0.4), at(2.4), at(4.0), at(6.0)),
        (Some("edge"), Some("edge"), Some("edge"), Some("edge"))
    );
    // Excluding the dragged clip drops its edges (A1 still has 0.4 / 2.4).
    let second = sequence_dto(s.first_sequence().unwrap()).tracks[0].clips[1]
        .id
        .clone();
    let without = s.snap_points(Some(second)).unwrap();
    assert!(!without.iter().any(|p| (p.t - 4.0).abs() < 1e-9));
    assert!(s.snap_points(Some("nope".into())).is_err());
    // Close the gap on V1: the second take ripples to 2.4; undo restores 4.0.
    s.edit(EditOp::CloseGaps { track: v.clone() }).unwrap();
    let dto = sequence_dto(s.first_sequence().unwrap());
    let clips = &dto.tracks[0].clips;
    assert_eq!(
        (clips[0].id.as_str(), clips[1].timeline_in),
        (clip_id.as_str(), 2.4)
    );
    s.undo().unwrap();
    assert_eq!(
        sequence_dto(s.first_sequence().unwrap()).tracks[0].clips[1].timeline_in,
        4.0
    );
}

/// EDL and OpenTimelineIO export (MED-12).
#[test]
fn interchange_writes_edl_and_otio() {
    let Fx { s, dir, .. } = fixture("interchange_writes_edl_and_otio");
    let edl = dir.join("cut.edl").to_string_lossy().into_owned();
    let otio = dir.join("cut.otio").to_string_lossy().into_owned();
    s.export_interchange(&edl, "edl").unwrap();
    s.export_interchange(&otio, "otio").unwrap();
    let edl = std::fs::read_to_string(edl).unwrap();
    assert!(edl.starts_with("TITLE: "), "{edl}");
    assert!(edl.contains("001  ") && edl.contains("* FROM CLIP NAME: "));
    let otio: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(otio).unwrap()).unwrap();
    assert_eq!(otio["OTIO_SCHEMA"], "Timeline.1");
    assert!(!otio["tracks"]["children"].as_array().unwrap().is_empty());
    let bad = dir.join("cut.xml").to_string_lossy().into_owned();
    assert!(s.export_interchange(&bad, "fcpxml").is_err());
}

/// Timeline and clip markers (TL-10).
#[test]
fn markers_edit_export_undo() {
    let Fx { mut s, dir, .. } = fixture("markers_edit_export_undo");
    // Markers: timeline and clip markers, edit, export, undo.
    let clip_for_marker = sequence_dto(s.first_sequence().unwrap()).tracks[0].clips[0]
        .id
        .clone();
    let m1 = s.add_marker(1.0, "timeline".into(), None).unwrap();
    let m2 = s
        .add_marker(2.0, "on clip".into(), Some(clip_for_marker))
        .unwrap();
    let list = s.markers().unwrap();
    assert_eq!(list.len(), 2, "{list:?}");
    assert_eq!((list[0].at, list[0].clip.is_none()), (1.0, true));
    assert_eq!(
        (list[1].at, list[1].clip.is_some()),
        (2.0, true),
        "clip marker reported in sequence time"
    );
    s.update_marker(
        &m1,
        MarkerEdit {
            note: Some("renamed".into()),
            color: Some([255, 0, 0]),
            duration: Some(0.4),
            at: None,
        },
    )
    .unwrap();
    let list = s.markers().unwrap();
    assert_eq!(
        (list[0].note.as_str(), list[0].color, list[0].duration),
        ("renamed", [255, 0, 0], 0.4)
    );
    let marker_path = dir.join("markers.tsv").to_string_lossy().into_owned();
    assert_eq!(s.export_markers(&marker_path).unwrap(), 2);
    let text = std::fs::read_to_string(&marker_path).unwrap();
    assert!(
        text.contains("00:00:01:00\t00:00:01:10\t#ff0000\trenamed"),
        "{text}"
    );
    s.remove_marker(&m2).unwrap();
    assert_eq!(s.markers().unwrap().len(), 1);
    s.workspace_mut().unwrap().undo().unwrap();
    assert_eq!(s.markers().unwrap().len(), 2);
    s.workspace_mut().unwrap().redo().unwrap();
    s.workspace_mut().unwrap().undo().unwrap();
    s.workspace_mut().unwrap().undo().unwrap();
    s.workspace_mut().unwrap().undo().unwrap();
    s.workspace_mut().unwrap().undo().unwrap();
    assert!(s.markers().unwrap().is_empty());
    s.sync_player().unwrap();
}

/// Mixer strip and inserts (AUD-02/05), normalized export with caption sidecar (EXP, AUD-06).
#[test]
fn mixer_and_normalized_export_with_sidecar() {
    let Fx { mut s, a, dir, .. } = fixture("mixer_and_normalized_export_with_sidecar");
    // Mixer: fader and an insert go through the command log.
    s.set_track_mix(
        &a,
        TrackMix {
            gain_db: -6.0,
            pan: 0.25,
            mute: false,
            solo: false,
        },
    )
    .unwrap();
    s.add_insert(&a, "limiter").unwrap();
    let dto = sequence_dto(s.first_sequence().unwrap());
    let a_track = dto.tracks.iter().find(|t| t.id == a).unwrap();
    assert_eq!(a_track.mix.gain_db, -6.0);
    assert_eq!(a_track.inserts, vec!["Limiter".to_string()]);
    assert!(s.add_insert(&a, "nope").is_err());

    // Export through the queue worker, normalized to -14 LUFS, with a caption
    // sidecar written next to the movie.
    let out = dir.join("out.mp4").to_string_lossy().into_owned();
    s.add_caption(0.6, 1.6, "Bye".into()).unwrap();
    let job_id = s
        .export_start(out.clone(), "YouTube 1080p", Some(-14.0), true, true)
        .unwrap();
    let caps = s.codec_capabilities();
    assert!(caps
        .hardware_encoders
        .iter()
        .all(|e| e.codec == "h264" || e.codec == "hevc"));
    let sidecar = std::fs::read_to_string(dir.join("out.srt")).unwrap();
    assert!(
        sidecar.contains("00:00:00,600 --> 00:00:01,600\nBye"),
        "{sidecar}"
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    let final_state = loop {
        let st = s.export_status();
        let e = st.iter().find(|e| e.id == job_id).unwrap();
        if e.state == "done" || e.state == "failed" || std::time::Instant::now() > deadline {
            break (
                e.state.clone(),
                e.error.clone(),
                e.frames_done,
                e.frames_total,
                e.loudness_lufs,
                e.true_peak_db,
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    assert_eq!(final_state.0, "done", "{:?}", final_state.1);
    assert_eq!((final_state.2, final_state.3), (60, 60), "2.4 s at 25 fps");
    let lufs = final_state.4.expect("loudness measured");
    assert!(
        (lufs + 14.0).abs() < 1.0 || final_state.5 > -1.1,
        "normalized: {lufs} LUFS, {} dBTP",
        final_state.5
    );
    assert!(std::fs::metadata(&out)
        .map(|m| m.len() > 1000)
        .unwrap_or(false));
}

/// Save, edit, crash, reopen: the journal recovers the edit (MED-09, NFR-05).
#[test]
fn persistence_recovers_unsaved_edits() {
    let Fx {
        mut s,
        path,
        v,
        dir,
        ..
    } = fixture("persistence_recovers_unsaved_edits");
    // Persistence: save, keep editing, "crash", reopen: the unsaved edit is recovered.
    assert!(s.file_status().dirty);
    let st = s.save_project(None).unwrap();
    assert!(!st.dirty && std::path::Path::new(&path).exists());
    s.edit(EditOp::Lift {
        track: v.clone(),
        start: 0.4,
        end: 1.0,
    })
    .unwrap();
    let clips_after_lift = sequence_dto(s.first_sequence().unwrap()).tracks[0]
        .clips
        .len();
    drop(s);
    let mut s2 = native();
    let st = s2.open_project_file(path.clone()).unwrap();
    assert_eq!(st.recovered, 1, "the lift was journaled but not saved");
    assert!(st.dirty);
    assert_eq!(
        sequence_dto(s2.first_sequence().unwrap()).tracks[0]
            .clips
            .len(),
        clips_after_lift
    );
    // The reopened project plays: media was re-probed and decoders rebuilt.
    s2.transport(TransportAction::Seek { t: 1.2 }).unwrap();
    let (w, _, px) = s2.frame_pixels().unwrap();
    assert_eq!(w, 64);
    assert!(px
        .chunks(4)
        .any(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 60));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
#[ignore]
fn bench_preview_divisors_on_1080p() {
    let dir = std::env::temp_dir().join(format!("debut-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let clip = dir.join("hd.mp4");
    let ok = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1920x1080:rate=25:duration=2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-preset",
            "ultrafast",
        ])
        .arg(&clip)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok, "ffmpeg is needed to generate the 1080p clip");
    let mut s = native();
    let id = s.ids.fresh();
    s.start(
        Project::new(id, "bench"),
        dir.join("b.debut").to_string_lossy().into_owned(),
    )
    .unwrap();
    let m = s.import_media(clip.to_string_lossy().into_owned()).unwrap();
    let seq = s.ensure_sequence().unwrap();
    s.add_clip(&seq.tracks[0].id, &m.id, 0.0).unwrap();
    for (q, label) in [
        (PreviewQuality::Full, "full"),
        (PreviewQuality::Half, "half"),
        (PreviewQuality::Quarter, "quarter"),
    ] {
        s.set_preview_quality(q);
        let t0 = std::time::Instant::now();
        for i in 0..10 {
            s.transport(TransportAction::Seek { t: i as f64 * 0.04 })
                .unwrap();
            s.frame_pixels().unwrap();
        }
        eprintln!(
            "{label}: {:.1} ms/frame (gpu={})",
            t0.elapsed().as_secs_f64() * 100.0,
            s.backend.is_gpu()
        );
    }
    std::fs::remove_dir_all(dir).ok();
}
