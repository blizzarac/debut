//! Scripting API (NFR-11): Rhai scripts that drive the editor through the
//! same `Session` command surface the app uses, so a script can do what a
//! user can and nothing else (no files or processes of its own).
//!
//! A run is one undo step: every edit it makes is folded into a single
//! history entry, and a script that fails is rolled back. Scripts get an
//! operation budget, so a runaway loop ends with an error instead of
//! hanging the app.
//!
//! ```rhai
//! let m = import_media("/footage/a.mp4");
//! let v1 = sequence().tracks[0].id;
//! for i in 0..4 { add_clip(v1, m.id, i * 2.5); }
//! add_marker(5.0, "review here");
//! ```

use debut_engine::api::{EditOp, Session, TransportAction};
use debut_project::{Param, Shape, TrackMix};
use rhai::serde::to_dynamic;
use rhai::{Dynamic, Engine, EvalAltResult, ImmutableString, Map, INT};
use serde::Serialize;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// What a run produced.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ScriptOutput {
    /// Lines from `print` and `debug`, in order.
    pub log: Vec<String>,
    /// The value of the script's last expression, unless it is `()`.
    pub value: Option<String>,
    /// Why the script stopped, with its position; its edits were undone.
    pub error: Option<String>,
    /// Edits the script made (one undo step), 0 after an error.
    pub edits: usize,
}

/// Operations a script may run before it is stopped.
pub const MAX_OPERATIONS: u64 = 5_000_000;

/// The functions scripts can call, with a one-line description each (the
/// app shows these as help).
pub const API: &[(&str, &str)] = &[
    ("media()", "Imported media: id, path, duration, width, height, ..."),
    ("import_media(path)", "Import a file; returns its media entry"),
    ("sequence()", "The open sequence (made from the first media if there is none): tracks (id, kind, clips), duration, size"),
    ("markers()", "Timeline and clip markers"),
    ("captions()", "Captions"),
    ("presets()", "Export presets"),
    ("add_clip(track, media, at)", "Put media on a track at `at` seconds"),
    ("insert(media, at) / overwrite(media, at)", "Edit media into the targeted tracks"),
    ("blade(at)", "Cut the targeted tracks at `at` seconds"),
    ("edit(#{kind: ..})", "Any timeline edit: ripple_head/tail, roll, slip, slide, move, ..."),
    ("add_title(at, text [, template])", "Add a 5 s title; returns the clip id"),
    ("add_shape(at, #{kind: \"ellipse\", ..})", "Add a 5 s shape; returns the clip id"),
    ("add_marker(at, note)", "Add a timeline marker; returns its id"),
    ("add_caption(start, end, text)", "Add a caption; returns its id"),
    ("add_effect(track, clip, kind)", "transform, grade, mask, key, lut ..."),
    ("set_param(track, clip, effect, param, value [, keyframe])", "Set an effect parameter"),
    ("set_mix(track, #{gain_db, pan, mute, solo})", "Change a track's mixer strip"),
    ("seek(t) / play() / pause()", "Transport"),
    ("undo() / redo()", "Step the history"),
    ("snapshot(name)", "Save a timeline snapshot"),
    ("save([path])", "Save the project"),
    ("start_export(path, preset [, #{normalize, sidecar, hardware, smart}])", "Queue an export; returns the job id"),
    ("exports()", "Export jobs and their progress"),
    ("print(x)", "Write a line to the output (built in)"),
];

type Res = Result<Dynamic, Box<EvalAltResult>>;

/// The session a run works on. Set for the length of `run` only.
#[derive(Clone)]
struct Handle(Rc<Cell<*mut Session>>);

impl Handle {
    fn with<R>(
        &self,
        f: impl FnOnce(&mut Session) -> Result<R, String>,
    ) -> Result<R, Box<EvalAltResult>> {
        let ptr = self.0.get();
        if ptr.is_null() {
            return Err("the script has finished".into());
        }
        // SAFETY: `run` sets the pointer from its `&mut Session`, does not
        // touch the session itself while the engine runs, and clears the
        // pointer before returning. Native functions do not call back into
        // scripts, so only one of these borrows is alive at a time.
        let session = unsafe { &mut *ptr };
        f(session).map_err(|e| e.into())
    }
}

fn num(d: &Dynamic) -> Result<f64, Box<EvalAltResult>> {
    if let Some(f) = d.clone().try_cast::<f64>() {
        Ok(f)
    } else if let Some(i) = d.clone().try_cast::<INT>() {
        Ok(i as f64)
    } else {
        Err(format!("expected a number, got {}", d.type_name()).into())
    }
}

fn dynamic<T: Serialize>(v: T) -> Res {
    to_dynamic(v)
}

/// Read a script value as `T`, through JSON so numbers convert the way
/// people expect (a float into an `f32` field, an integer into a float).
fn read<T: serde::de::DeserializeOwned>(v: &Dynamic) -> Result<T, Box<EvalAltResult>> {
    let json = serde_json::to_value(v).map_err(|e| e.to_string())?;
    serde_json::from_value(json).map_err(|e| e.to_string().into())
}

/// Overlay a script map onto a value's serialized form, then read it back.
fn merged<T: Serialize + serde::de::DeserializeOwned>(
    base: &T,
    over: Map,
) -> Result<T, Box<EvalAltResult>> {
    let mut json = serde_json::to_value(base).map_err(|e| e.to_string())?;
    let over = serde_json::to_value(Dynamic::from_map(over)).map_err(|e| e.to_string())?;
    if let (Some(a), Some(b)) = (json.as_object_mut(), over.as_object()) {
        a.extend(b.clone());
    }
    serde_json::from_value(json).map_err(|e| e.to_string().into())
}

/// Run `source` against `session`.
pub fn run(session: &mut Session, source: &str) -> ScriptOutput {
    let mark = session.undo_mark();
    let log = Rc::new(RefCell::new(Vec::new()));
    let handle = Handle(Rc::new(Cell::new(session as *mut Session)));
    let result = {
        let engine = engine(&handle, &log);
        engine.eval::<Dynamic>(source)
    };
    handle.0.set(std::ptr::null_mut());
    let edits = session.group_undo_since(mark);
    let mut out = ScriptOutput {
        log: log.take(),
        ..Default::default()
    };
    match result {
        Ok(v) if v.is_unit() => out.edits = edits,
        Ok(v) => {
            out.edits = edits;
            out.value = Some(v.to_string());
        }
        Err(e) => {
            if edits > 0 {
                let _ = session.undo();
            }
            out.error = Some(e.to_string());
        }
    }
    out
}

fn engine(h: &Handle, log: &Rc<RefCell<Vec<String>>>) -> Engine {
    let mut e = Engine::new();
    e.set_max_operations(MAX_OPERATIONS);
    e.set_max_expr_depths(64, 32);
    e.set_max_string_size(1 << 20);
    e.set_max_array_size(100_000);
    e.set_max_map_size(100_000);
    {
        let log = Rc::clone(log);
        e.on_print(move |s| log.borrow_mut().push(s.to_string()));
    }
    {
        let log = Rc::clone(log);
        e.on_debug(move |s, _, pos| log.borrow_mut().push(format!("{pos:?}: {s}")));
    }

    // Reading.
    let s = h.clone();
    e.register_fn("media", move || -> Res {
        s.with(|s| s.media_list()).and_then(dynamic)
    });
    let s = h.clone();
    e.register_fn("import_media", move |path: ImmutableString| -> Res {
        s.with(|s| s.import_media(path.to_string()))
            .and_then(dynamic)
    });
    let s = h.clone();
    e.register_fn("sequence", move || -> Res {
        s.with(|s| s.ensure_sequence()).and_then(dynamic)
    });
    let s = h.clone();
    e.register_fn("markers", move || -> Res {
        s.with(|s| s.markers()).and_then(dynamic)
    });
    let s = h.clone();
    e.register_fn("captions", move || -> Res {
        s.with(|s| s.captions()).and_then(dynamic)
    });
    let s = h.clone();
    e.register_fn("presets", move || -> Res {
        s.with(|s| Ok(s.export_presets())).and_then(dynamic)
    });
    let s = h.clone();
    e.register_fn("exports", move || -> Res {
        s.with(|s| Ok(s.export_status())).and_then(dynamic)
    });

    // Editing.
    let s = h.clone();
    e.register_fn(
        "add_clip",
        move |track: ImmutableString, media: ImmutableString, at: Dynamic| -> Res {
            let at = num(&at)?;
            s.with(|s| s.add_clip(&track, &media, at))
                .map(|_| Dynamic::UNIT)
        },
    );
    for (name, overwrite) in [("insert", false), ("overwrite", true)] {
        let s = h.clone();
        e.register_fn(name, move |media: ImmutableString, at: Dynamic| -> Res {
            let at = num(&at)?;
            s.with(|s| s.insert_media(&media, at, overwrite))
                .map(|_| Dynamic::UNIT)
        });
    }
    let s = h.clone();
    e.register_fn("blade", move |at: Dynamic| -> Res {
        let at = num(&at)?;
        s.with(|s| s.blade_targeted(at)).map(|_| Dynamic::UNIT)
    });
    let s = h.clone();
    e.register_fn("edit", move |op: Map| -> Res {
        let op: EditOp = read(&Dynamic::from_map(op))?;
        s.with(|s| s.edit(op)).map(|_| Dynamic::UNIT)
    });
    let s = h.clone();
    e.register_fn(
        "add_title",
        move |at: Dynamic, text: ImmutableString| -> Res {
            let at = num(&at)?;
            s.with(|s| s.add_title(at, text.to_string()))
                .map(Dynamic::from)
        },
    );
    let s = h.clone();
    e.register_fn(
        "add_title",
        move |at: Dynamic, text: ImmutableString, template: ImmutableString| -> Res {
            let at = num(&at)?;
            s.with(|s| s.add_title_from(at, text.to_string(), Some(template.to_string())))
                .map(Dynamic::from)
        },
    );
    let s = h.clone();
    e.register_fn("add_shape", move |at: Dynamic, shape: Map| -> Res {
        let at = num(&at)?;
        let shape: Shape = merged(&Shape::default(), shape)?;
        s.with(|s| s.add_shape(at, shape)).map(Dynamic::from)
    });
    let s = h.clone();
    e.register_fn(
        "add_marker",
        move |at: Dynamic, note: ImmutableString| -> Res {
            let at = num(&at)?;
            s.with(|s| s.add_marker(at, note.to_string(), None))
                .map(Dynamic::from)
        },
    );
    let s = h.clone();
    e.register_fn(
        "add_caption",
        move |start: Dynamic, end: Dynamic, text: ImmutableString| -> Res {
            let (start, end) = (num(&start)?, num(&end)?);
            s.with(|s| s.add_caption(start, end, text.to_string()))
                .map(Dynamic::from)
        },
    );
    let s = h.clone();
    e.register_fn(
        "add_effect",
        move |track: ImmutableString, clip: ImmutableString, kind: ImmutableString| -> Res {
            s.with(|s| s.add_effect(&track, &clip, &kind))
                .map(|_| Dynamic::UNIT)
        },
    );
    for keyed in [false, true] {
        let s = h.clone();
        let set = move |track: ImmutableString,
                        clip: ImmutableString,
                        effect: INT,
                        param: ImmutableString,
                        value: Dynamic,
                        keyframe: bool|
              -> Res {
            let value = num(&value)?;
            let param: Param = read(&Dynamic::from(param))?;
            let effect = usize::try_from(effect).map_err(|_| "effect index must be 0 or more")?;
            s.with(|s| s.set_param(&track, &clip, effect, param, value, keyframe))
                .map(|_| Dynamic::UNIT)
        };
        if keyed {
            e.register_fn("set_param", set);
        } else {
            e.register_fn(
                "set_param",
                move |track: ImmutableString,
                      clip: ImmutableString,
                      effect: INT,
                      param: ImmutableString,
                      value: Dynamic|
                      -> Res { set(track, clip, effect, param, value, false) },
            );
        }
    }
    let s = h.clone();
    e.register_fn("set_mix", move |track: ImmutableString, over: Map| -> Res {
        s.with(|s| {
            let current = s
                .sequence()?
                .tracks
                .into_iter()
                .find(|t| t.id == track.as_str())
                .map(|t| t.mix)
                .ok_or_else(|| format!("no track {track}"))?;
            Ok(current)
        })
        .and_then(|current| merged::<TrackMix>(&current, over))
        .and_then(|mix| s.with(|s| s.set_track_mix(&track, mix)))
        .map(|_| Dynamic::UNIT)
    });

    // Transport and history.
    let s = h.clone();
    e.register_fn("seek", move |t: Dynamic| -> Res {
        let t = num(&t)?;
        s.with(|s| s.transport(TransportAction::Seek { t }))
            .map(|_| Dynamic::UNIT)
    });
    let s = h.clone();
    e.register_fn("play", move || -> Res {
        s.with(|s| s.transport(TransportAction::Play))
            .map(|_| Dynamic::UNIT)
    });
    let s = h.clone();
    e.register_fn("pause", move || -> Res {
        s.with(|s| s.transport(TransportAction::Pause))
            .map(|_| Dynamic::UNIT)
    });
    let s = h.clone();
    e.register_fn("undo", move || -> Res {
        s.with(|s| s.undo()).map(Dynamic::from)
    });
    let s = h.clone();
    e.register_fn("redo", move || -> Res {
        s.with(|s| s.redo()).map(Dynamic::from)
    });
    let s = h.clone();
    e.register_fn("snapshot", move |name: ImmutableString| -> Res {
        s.with(|s| s.take_snapshot(name.to_string()))
            .map(Dynamic::from)
    });

    // Files and export.
    let s = h.clone();
    e.register_fn("save", move || -> Res {
        s.with(|s| s.save_project(None)).and_then(dynamic)
    });
    let s = h.clone();
    e.register_fn("save", move |path: ImmutableString| -> Res {
        s.with(|s| s.save_project(Some(path.to_string())))
            .and_then(dynamic)
    });
    let s = h.clone();
    let export = move |path: ImmutableString, preset: ImmutableString, opts: Map| -> Res {
        let flag = |k: &str| opts.get(k).and_then(|v| v.as_bool().ok()).unwrap_or(false);
        let normalize = match opts.get("normalize") {
            Some(v) if !v.is_unit() => Some(num(v)? as f32),
            _ => None,
        };
        let (sidecar, hardware, smart) = (flag("sidecar"), flag("hardware"), flag("smart"));
        s.with(|s| {
            s.export_start_with(
                path.to_string(),
                &preset,
                normalize,
                sidecar,
                hardware,
                smart,
            )
        })
        .map(|id| Dynamic::from(id as INT))
    };
    let plain = export.clone();
    e.register_fn(
        "start_export",
        move |path: ImmutableString, preset: ImmutableString| -> Res {
            plain(path, preset, Map::new())
        },
    );
    e.register_fn("start_export", export);
    e
}

#[cfg(test)]
mod tests;
