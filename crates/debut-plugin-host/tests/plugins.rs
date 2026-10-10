//! The real helper process running the test plugins (an OpenFX bundle and a
//! CLAP library built by build.rs): through the platform host, and through
//! the engine as a clip effect and a track insert.
#![cfg(unix)]

use debut_engine::api::{PreviewQuality, TransportAction};
use debut_engine::Session;
use debut_platform::plugin_host::{AudioJob, PluginHost, PluginKind, VideoJob};
use debut_platform_native::plugin_host::NativePluginHost;
use debut_platform_native::NativePlatform;
use std::sync::Arc;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../debut-platform-native/tests/fixtures/test_25fps_2s.mp4"
);

fn host() -> NativePluginHost {
    NativePluginHost::new(
        env!("CARGO_BIN_EXE_debut-plugin-host").into(),
        vec![env!("DEBUT_TEST_PLUGINS").to_string()],
    )
}

#[test]
fn scans_runs_and_survives_a_crashing_plugin() {
    let h = host();
    let scan = h.scan().unwrap();
    assert!(scan.problems.is_empty(), "{:?}", scan.problems);
    let ofx = scan
        .plugins
        .iter()
        .find(|p| p.plugin.kind == PluginKind::OpenFx)
        .expect("the OpenFX test plugin");
    let clap = scan
        .plugins
        .iter()
        .find(|p| p.plugin.kind == PluginKind::Clap)
        .expect("the CLAP test plugin");
    assert_eq!(ofx.name, "Test Gain");
    let names: Vec<_> = ofx.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        ["gain", "bottom"],
        "the secret crash parameter stays hidden"
    );
    assert_eq!(
        (ofx.params[0].min, ofx.params[0].max, ofx.params[0].default),
        (0.0, 4.0, 1.0)
    );
    assert_eq!(clap.params.len(), 1);

    // 1x2 frame, top row first: "bottom" (OpenFX's own bottom-up half) must
    // land on our second row.
    let job = |params: Vec<(String, f64)>| VideoJob {
        plugin: ofx.plugin.clone(),
        width: 1,
        height: 2,
        frame: 0.0,
        fps: 25.0,
        params,
    };
    let mut px = vec![0.25, 0.25, 0.25, 1.0, 0.25, 0.25, 0.25, 1.0];
    h.process_video(
        &job(vec![("gain".into(), 2.0), ("bottom".into(), 1.0)]),
        &mut px,
    )
    .unwrap();
    assert_eq!(px, [0.25, 0.25, 0.25, 1.0, 0.5, 0.5, 0.5, 1.0]);

    let audio = |params| AudioJob {
        plugin: clap.plugin.clone(),
        instance: 1,
        sample_rate: 48_000,
        channels: 2,
        params,
    };
    let mut buf = vec![0.1f32, -0.2, 0.1, -0.2];
    h.process_audio(&audio(vec![("Gain".into(), 3.0)]), &mut buf)
        .unwrap();
    assert!(
        (buf[0] - 0.3).abs() < 1e-6 && (buf[1] + 0.6).abs() < 1e-6,
        "{buf:?}"
    );

    // A crash takes down the helper, not us; the next request gets a new one.
    let mut px = vec![0.5; 8];
    let err = h
        .process_video(&job(vec![("crash".into(), 1.0)]), &mut px)
        .unwrap_err();
    assert!(err.to_string().contains("plugin host stopped"), "{err}");
    let mut px = vec![0.25; 8];
    h.process_video(&job(vec![("gain".into(), 2.0)]), &mut px)
        .unwrap();
    assert_eq!(px[0], 0.5);
    // Unknown parameters are refused, not ignored.
    assert!(h
        .process_video(&job(vec![("nope".into(), 1.0)]), &mut px)
        .is_err());
}

#[test]
fn a_bad_binary_is_a_problem_not_a_failure() {
    let dir = std::env::temp_dir().join(format!("debut-badplug-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("broken.clap"), b"not a library").unwrap();
    let h = NativePluginHost::new(
        env!("CARGO_BIN_EXE_debut-plugin-host").into(),
        vec![
            dir.to_string_lossy().into_owned(),
            env!("DEBUT_TEST_PLUGINS").to_string(),
        ],
    );
    let scan = h.scan().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(scan.plugins.len(), 2);
    assert_eq!(scan.problems.len(), 1);
    assert!(scan.problems[0].path.ends_with("broken.clap"));
}

#[test]
fn engine_runs_an_openfx_effect_and_a_clap_insert() {
    let mut s = Session::new(Arc::new(NativePlatform::with_plugins(host())));
    s.new_project("p".into()).unwrap();
    let m = s.import_media(FIXTURE.to_string()).unwrap();
    let seq = s.ensure_sequence().unwrap();
    let (v, a) = (seq.tracks[0].id.clone(), seq.tracks[1].id.clone());
    s.add_clip(&v, &m.id, 0.0).unwrap();
    s.set_preview_quality(PreviewQuality::Full);
    let clip = s.sequence().unwrap().tracks[0].clips[0].id.clone();
    s.transport(TransportAction::Seek { t: 0.4 }).unwrap();
    let (_, _, before) = s.frame_pixels().unwrap();

    let found = s.scan_plugins().unwrap();
    assert!(found.available);
    let ofx = found
        .plugins
        .iter()
        .find(|p| p.kind == "openfx")
        .unwrap()
        .clone();
    let clap = found
        .plugins
        .iter()
        .find(|p| p.kind == "clap")
        .unwrap()
        .clone();

    s.add_plugin_effect(&v, &clip, &ofx.path, ofx.index)
        .unwrap();
    s.set_plugin_param(&v, &clip, 0, "gain", 0.5, false)
        .unwrap();
    let fx = s.clip_effects(&v, &clip).unwrap();
    let p = fx[0].options.plugin.as_ref().unwrap();
    assert_eq!((p.name.as_str(), p.params[0].value), ("Test Gain", 0.5));
    let (_, _, after) = s.frame_pixels().unwrap();
    assert!(s.plugin_error().is_none(), "{:?}", s.plugin_error());
    // RGB only (alpha stays 255); half linear gain is ~0.73 once sRGB-encoded.
    let mean = |px: &[u8]| {
        let rgb: Vec<f64> = px
            .chunks_exact(4)
            .flat_map(|p| p[..3].iter().map(|&b| b as f64))
            .collect();
        rgb.iter().sum::<f64>() / rgb.len() as f64
    };
    let ratio = mean(&after) / mean(&before);
    assert!(
        (0.65..0.8).contains(&ratio),
        "half gain darkens the frame: ratio {ratio}"
    );
    // Undo removes the parameter change, then the effect.
    s.undo().unwrap();
    assert_eq!(
        s.clip_effects(&v, &clip).unwrap()[0]
            .options
            .plugin
            .as_ref()
            .unwrap()
            .params[0]
            .value,
        1.0
    );

    s.add_plugin_insert(&a, &clap.path, clap.index).unwrap();
    s.set_insert_param(&a, 0, "Gain", 2.0).unwrap();
    let seq = s.sequence().unwrap();
    let track = &seq.tracks[1];
    assert_eq!(track.inserts, vec!["Test Gain".to_string()]);
    assert_eq!(track.insert_params[0][0].value, 2.0);
    assert!(s.set_insert_param(&a, 0, "Nope", 1.0).is_err());
}

#[test]
fn audio_inserts_run_in_the_helper() {
    use debut_audio::{render_span, Inserts, SampleSource, CHANNELS};
    use debut_core::{IdGen, MediaId, Rational, Result};
    use debut_project::{
        AudioEffect, AudioPluginParam, Clip, ClipSource, Sequence, Track, TrackKind,
    };

    struct Ones(Arc<dyn PluginHost>);
    impl SampleSource for Ones {
        fn read(
            &mut self,
            _: MediaId,
            _: Rational,
            frames: usize,
            out: &mut Vec<f32>,
        ) -> Result<u16> {
            out.clear();
            out.resize(frames * 2, 0.25);
            Ok(2)
        }
        fn plugins(&self) -> Option<Arc<dyn PluginHost>> {
            Some(self.0.clone())
        }
    }

    let h: Arc<dyn PluginHost> = Arc::new(host());
    let clap = h
        .scan()
        .unwrap()
        .plugins
        .into_iter()
        .find(|p| p.plugin.kind == PluginKind::Clap)
        .unwrap();
    let mut ids = IdGen::new(7);
    let media: MediaId = ids.fresh();
    let mut seq = Sequence::new(ids.fresh(), "s", debut_core::FrameRate::FPS_25, 64, 36);
    let mut t = Track::new(ids.fresh(), TrackKind::Audio);
    t.clips.push(Clip::new(
        ids.fresh(),
        ClipSource::Media(media),
        Rational::ZERO,
        Rational::from_int(1),
        Rational::ZERO,
    ));
    t.audio_effects.push(AudioEffect::Plugin {
        path: clap.plugin.path.clone(),
        index: clap.plugin.index,
        id: clap.id.clone(),
        name: clap.name.clone(),
        params: vec![AudioPluginParam {
            name: "Gain".into(),
            min: 0.0,
            max: 4.0,
            value: 2.0,
        }],
    });
    seq.tracks.push(t);
    let mut src = Ones(h);
    let mut inserts = Inserts::default();
    inserts.set_plugins(src.plugins());
    inserts.sync(&seq, 48_000);
    let frames = 480;
    let mut bus = vec![0.0f32; frames * CHANNELS];
    render_span(&seq, &mut inserts, &mut src, 0, frames, 48_000, &mut bus).unwrap();
    // The fader's default pan law may scale both sides alike; the insert doubles.
    let mut plain = vec![0.0f32; frames * CHANNELS];
    let mut seq2 = seq.clone();
    seq2.tracks[0].audio_effects.clear();
    let mut inserts2 = Inserts::default();
    inserts2.sync(&seq2, 48_000);
    render_span(
        &seq2,
        &mut inserts2,
        &mut src,
        0,
        frames,
        48_000,
        &mut plain,
    )
    .unwrap();
    assert!(plain[10] > 0.0);
    assert!(
        (bus[10] - 2.0 * plain[10]).abs() < 1e-5,
        "{} vs {}",
        bus[10],
        plain[10]
    );
}
