#![allow(clippy::disallowed_methods, clippy::disallowed_types)] // tests use the OS directly

use super::*;
use debut_core::IdGen;
use debut_platform_native::NativePlatform;
use debut_project::Project;
use std::sync::Arc;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../debut-platform-native/tests/fixtures/test_25fps_2s.mp4"
);

struct Fx {
    s: Session,
    dir: std::path::PathBuf,
}

impl Drop for Fx {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn session(name: &str) -> Fx {
    let dir = std::env::temp_dir().join(format!("debut-script-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new(Arc::new(NativePlatform::new()));
    let id = IdGen::new(3).fresh();
    s.start(
        Project::new(id, "t"),
        dir.join("t.debut").to_string_lossy().into_owned(),
    )
    .unwrap();
    Fx { s, dir }
}

fn ok(s: &mut Session, src: &str) -> ScriptOutput {
    let out = run(s, src);
    assert_eq!(out.error, None, "{src}");
    out
}

#[test]
fn builds_a_timeline_in_one_undo_step() {
    let Fx { ref mut s, .. } = session("build");
    let out = ok(
        s,
        &format!(
            r#"
            let m = import_media("{FIXTURE}");
            let seq = sequence();
            let v1 = seq.tracks[0].id;
            for i in 0..3 {{ add_clip(v1, m.id, i * 2); }}
            add_marker(1.5, "look");
            add_title(0, "Hello");
            print(`clips: ${{sequence().tracks[0].clips.len()}}`);
            sequence().duration
            "#
        ),
    );
    assert_eq!(out.log, vec!["clips: 3"]);
    assert_eq!(out.value.as_deref(), Some("6.0"));
    // import creates no history entry; sequence, clips, marker, title do.
    assert!(out.edits >= 5, "{}", out.edits);
    let seq = s.sequence().unwrap();
    assert_eq!(seq.tracks[0].clips.len(), 3);
    assert_eq!(s.markers().unwrap().len(), 1);
    // One undo takes the whole script back.
    s.undo().unwrap();
    assert!(
        s.sequence().is_err()
            || s.sequence()
                .unwrap()
                .tracks
                .iter()
                .all(|t| t.clips.is_empty())
    );
    assert!(s.markers().map(|m| m.is_empty()).unwrap_or(true));
}

#[test]
fn failed_scripts_are_rolled_back() {
    let Fx { ref mut s, .. } = session("rollback");
    ok(s, &format!(r#"import_media("{FIXTURE}"); sequence();"#));
    let before = s.sequence().unwrap();
    let out = run(
        s,
        r#"
        let v1 = sequence().tracks[0].id;
        add_clip(v1, media()[0].id, 0);
        add_marker(1, "a");
        add_clip(v1, "no-such-media", 1);
        "#,
    );
    let err = out.error.unwrap();
    assert!(err.contains("line 5"), "{err}");
    assert_eq!(out.edits, 0);
    let ids = |q: &debut_engine::api::SequenceDto| {
        q.tracks[0]
            .clips
            .iter()
            .map(|c| c.id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&s.sequence().unwrap()), ids(&before));
    assert!(s.markers().unwrap().is_empty());
}

#[test]
fn runaway_loops_stop() {
    let Fx { ref mut s, .. } = session("loop");
    let out = run(s, "let x = 0; loop { x += 1; }");
    assert!(out.error.unwrap().contains("operations"));
}

#[test]
fn edits_shapes_effects_and_mix_from_maps() {
    let Fx { ref mut s, .. } = session("maps");
    ok(
        s,
        &format!(
            r#"
            let m = import_media("{FIXTURE}");
            let seq = sequence();
            let v1 = seq.tracks[0].id;
            let a1 = seq.tracks[1].id;
            add_clip(v1, m.id, 0);
            let clip = sequence().tracks[0].clips[0].id;
            add_effect(v1, clip, "transform");
            set_param(v1, clip, 0, "opacity", 0.5);
            edit(#{{kind: "move", track: v1, clip: clip, delta: 1.0}});
            add_shape(0, #{{kind: "ellipse", width: 20.0, height: 10.0}});
            set_mix(a1, #{{gain_db: -6.0}});
            "#
        ),
    );
    let seq = s.sequence().unwrap();
    assert_eq!(seq.tracks[0].clips[0].timeline_in, 1.0);
    let shape = seq
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .find_map(|c| c.shape.clone())
        .unwrap();
    assert_eq!(shape.kind, debut_project::ShapeKind::Ellipse);
    assert_eq!((shape.width, shape.height), (20.0, 10.0));
    let a1 = seq.tracks.iter().find(|t| t.kind == "audio").unwrap();
    assert_eq!(a1.mix.gain_db, -6.0);
    assert_eq!(a1.mix.pan, 0.0, "fields not given keep their value");
    let fx = s
        .clip_effects(&seq.tracks[0].id, &seq.tracks[0].clips[0].id)
        .unwrap();
    assert_eq!(fx.len(), 1);
    // Bad input is an error, not a panic.
    let out = run(s, r#"edit(#{kind: "nonsense"})"#);
    assert!(out.error.is_some());
    let out = run(s, r#"add_clip("x", "y", "soon")"#);
    assert!(out.error.unwrap().contains("expected a number"));
}

#[test]
fn every_documented_function_exists() {
    let Fx { ref mut s, .. } = session("api");
    let engine = engine(
        &Handle(Rc::new(Cell::new(std::ptr::null_mut()))),
        &Rc::new(RefCell::new(Vec::new())),
    );
    let names: Vec<String> = engine
        .gen_fn_signatures(false)
        .into_iter()
        .map(|sig| sig.split('(').next().unwrap().to_string())
        .collect();
    for (sig, _) in API.iter().filter(|(_, doc)| !doc.contains("built in")) {
        for part in sig.split(" / ") {
            let name = part.split('(').next().unwrap().trim();
            assert!(names.iter().any(|n| n == name), "{name} not registered");
        }
    }
    // A finished run's functions refuse to touch the session.
    let out = run(s, "presets().len()");
    assert!(out.value.unwrap().parse::<i64>().unwrap() > 3);
}
