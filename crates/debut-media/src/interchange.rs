//! Interchange with other editors (MED-12): a CMX3600 EDL of the first video
//! and audio tracks, and an OpenTimelineIO document of the whole sequence
//! (nested sequences as stacks, dissolves as transitions, titles as generator
//! references, markers).

use debut_core::{FrameRate, Rational, Timecode};
use debut_project::media_ref::MediaRef;
use debut_project::{Clip, ClipSource, Marker, Project, Sequence, TrackKind};
use serde_json::{json, Value};

fn frames(t: Rational, rate: FrameRate) -> i64 {
    rate.time_to_frame(t)
}

fn tc(frame: i64, rate: FrameRate) -> String {
    Timecode::from_frames(frame.max(0), rate, true).to_string()
}

/// The media behind a clip at `t`, with its source time (active multicam
/// angle and its offset applied). `None` for titles and nested sequences.
fn media_of<'a>(
    clip: &Clip,
    media: &'a [MediaRef],
    t: Rational,
) -> Option<(&'a MediaRef, Rational)> {
    let (id, source) = clip.media_at(t)?;
    media.iter().find(|m| m.id == id).map(|m| (m, source))
}

pub(crate) fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// EDL reel name: the media's reel, else its file stem, upper-cased and cut
/// to the 8 characters CMX3600 allows.
fn reel(m: &MediaRef) -> String {
    let base = m.metadata.reel.clone().unwrap_or_else(|| {
        let name = file_name(&m.path);
        name.rsplit_once('.')
            .map_or(name, |(stem, _)| stem)
            .to_string()
    });
    let clean: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .take(8)
        .collect();
    if clean.is_empty() {
        "AX".into()
    } else {
        clean
    }
}

/// Source timecode frame for `source` seconds into `m` (adds the media's start
/// timecode when known).
fn source_frame(m: &MediaRef, source: Rational, rate: FrameRate) -> i64 {
    let start = m.metadata.start_timecode.map_or(0, |t| t.to_frames(rate));
    start + frames(source, rate)
}

/// CMX3600 EDL of the first video track ("V" events) and the first audio
/// track ("A" events). Dissolves become a cut-plus-dissolve event pair; titles
/// and nested sequences are listed as comments, not events.
pub fn edl(seq: &Sequence, media: &[MediaRef]) -> String {
    let rate = seq.frame_rate;
    let drop = Timecode::from_frames(0, rate, true).drop_frame;
    let mut out = format!(
        "TITLE: {}\nFCM: {}\n\n",
        seq.name.replace('\n', " "),
        if drop { "DROP FRAME" } else { "NON-DROP FRAME" }
    );
    let mut event = 0;
    let line = |n: i32, reel: &str, ch: &str, tr: &str, si: i64, so: i64, ri: i64, ro: i64| {
        format!(
            "{n:03}  {reel:<8} {ch:<5} {tr:<8} {} {} {} {}\n",
            tc(si, rate),
            tc(so, rate),
            tc(ri, rate),
            tc(ro, rate)
        )
    };
    for (kind, ch) in [(TrackKind::Video, "V"), (TrackKind::Audio, "A")] {
        let Some(track) = seq.tracks.iter().find(|t| t.kind == kind) else {
            continue;
        };
        for (i, clip) in track.clips.iter().enumerate() {
            let half = clip
                .transition_in
                .map(|t| t.half())
                .unwrap_or(Rational::ZERO);
            let next_half = track
                .clips
                .get(i + 1)
                .and_then(|n| n.transition_in)
                .map(|t| t.half())
                .unwrap_or(Rational::ZERO);
            // Record span: from the middle of the incoming dissolve to the start
            // of the outgoing one.
            let rec_in = clip.timeline_in - half;
            let rec_out = clip.timeline_out() - next_half;
            let Some((m, src_in)) = media_of(clip, media, rec_in) else {
                let what = match &clip.source {
                    ClipSource::Title(t) => format!("TITLE \"{}\"", t.text.replace('\n', " ")),
                    ClipSource::Shape(s) => format!("SHAPE {}", s.label().to_uppercase()),
                    _ => "NESTED SEQUENCE".into(),
                };
                out.push_str(&format!(
                    "* SKIPPED {what} AT {}\n",
                    tc(frames(clip.timeline_in, rate), rate)
                ));
                continue;
            };
            let src_out = media_of(clip, media, rec_out)
                .map(|(_, s)| s)
                .unwrap_or(src_in + (rec_out - rec_in) * clip.speed);
            event += 1;
            // Source in/out ascend even when the clip plays backwards; the M2
            // line carries the direction.
            let (si, so) = (
                source_frame(m, src_in.min(src_out), rate),
                source_frame(m, src_in.max(src_out), rate),
            );
            let (ri, ro) = (frames(rec_in, rate), frames(rec_out, rate));
            match (
                clip.transition_in,
                i.checked_sub(1).map(|p| &track.clips[p]),
            ) {
                (Some(tr), Some(prev)) => {
                    // Outgoing side: a zero-length cut at the dissolve start.
                    let (pm, ps) = media_of(prev, media, rec_in)
                        .map(|(pm, ps)| (reel(pm), source_frame(pm, ps, rate)))
                        .unwrap_or_else(|| ("BL".into(), 0));
                    out.push_str(&line(event, &pm, ch, "C", ps, ps, ri, ri));
                    let d = format!("D    {:03}", frames(tr.duration, rate));
                    out.push_str(&line(event, &reel(m), ch, &d, si, so, ri, ro));
                }
                _ => out.push_str(&line(event, &reel(m), ch, "C", si, so, ri, ro)),
            }
            if clip.is_retimed() && rec_out > rec_in {
                // Motion effect: speed in frames per second (mean over a ramp).
                let mean = (src_out - src_in).as_f64() / (rec_out - rec_in).as_f64();
                let fps = mean * rate.0.as_f64();
                out.push_str(&format!(
                    "M2   {:<8}       {:05.1}                {}\n",
                    reel(m),
                    fps,
                    tc(si, rate)
                ));
            }
            out.push_str(&format!("* FROM CLIP NAME: {}\n", file_name(&m.path)));
        }
    }
    out
}

/// OTIO speed effect: `FreezeFrame.1` for a hold, `LinearTimeWarp.1` for
/// constant speed, and for a ramp its mean speed with the keys in metadata.
fn time_warp(clip: &Clip) -> Value {
    if !clip.is_retimed() {
        return json!([]);
    }
    let len = clip.duration.as_f64();
    let scalar = if clip.ramp.is_empty() {
        clip.speed.as_f64()
    } else if len > 0.0 {
        clip.source_offset_f64(len) / len
    } else {
        1.0
    };
    let schema = if scalar == 0.0 && clip.ramp.is_empty() {
        "FreezeFrame.1"
    } else {
        "LinearTimeWarp.1"
    };
    let ramp: Vec<Value> = clip
        .ramp
        .iter()
        .map(|k| json!({ "at": k.at.as_f64(), "speed": k.speed }))
        .collect();
    let metadata = if ramp.is_empty() {
        json!({})
    } else {
        json!({ "debut": { "speed_ramp": ramp } })
    };
    json!([{
        "OTIO_SCHEMA": schema,
        "name": "speed",
        "effect_name": if schema == "FreezeFrame.1" { "FreezeFrame" } else { "LinearTimeWarp" },
        "time_scalar": scalar,
        "metadata": metadata,
    }])
}

fn rational_time(frames: i64, rate: FrameRate) -> Value {
    json!({ "OTIO_SCHEMA": "RationalTime.1", "rate": rate.0.as_f64(), "value": frames as f64 })
}

fn time_range(start: i64, duration: i64, rate: FrameRate) -> Value {
    json!({
        "OTIO_SCHEMA": "TimeRange.1",
        "start_time": rational_time(start, rate),
        "duration": rational_time(duration, rate),
    })
}

/// `file://` URL for a path, escaping what URLs do not allow.
pub(crate) fn file_url(path: &str) -> String {
    let mut out = String::from("file://");
    if !path.starts_with('/') {
        out.push('/');
    }
    for b in path.replace('\\', "/").bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// OTIO's named marker colours, nearest to an RGB colour.
fn marker_color(rgb: [u8; 3]) -> &'static str {
    const NAMED: &[(&str, [u8; 3])] = &[
        ("RED", [239, 68, 68]),
        ("ORANGE", [249, 115, 22]),
        ("YELLOW", [234, 179, 8]),
        ("GREEN", [34, 197, 94]),
        ("CYAN", [6, 182, 212]),
        ("BLUE", [59, 130, 246]),
        ("PURPLE", [168, 85, 247]),
        ("PINK", [236, 72, 153]),
        ("WHITE", [255, 255, 255]),
        ("BLACK", [0, 0, 0]),
    ];
    let d = |c: [u8; 3]| -> i32 { (0..3).map(|i| (c[i] as i32 - rgb[i] as i32).pow(2)).sum() };
    NAMED.iter().min_by_key(|(_, c)| d(*c)).unwrap().0
}

fn markers(list: &[Marker], rate: FrameRate) -> Value {
    Value::Array(
        list.iter()
            .map(|m| {
                json!({
                    "OTIO_SCHEMA": "Marker.2",
                    "name": m.note,
                    "color": marker_color(m.color),
                    "marked_range": time_range(frames(m.at, rate), frames(m.duration, rate), rate),
                    "metadata": {},
                })
            })
            .collect(),
    )
}

/// One OTIO item for a clip (a clip, a nested stack, or a generator).
fn item(clip: &Clip, project: &Project, rate: FrameRate, depth: usize) -> Value {
    let range = time_range(
        frames(clip.source_in, rate),
        frames(clip.duration, rate),
        rate,
    );
    let clip_markers = markers(&clip.markers, rate);
    if let ClipSource::Sequence(id) = &clip.source {
        if let Some(nested) = project.sequence(*id).filter(|_| depth < 8) {
            let mut stack = stack(nested, project, depth + 1);
            stack["source_range"] = range;
            stack["markers"] = clip_markers;
            return stack;
        }
    }
    let (name, reference) = match &clip.source {
        ClipSource::Title(t) => (
            t.text.lines().next().unwrap_or("Title").to_string(),
            json!({
                "OTIO_SCHEMA": "GeneratorReference.1",
                "generator_kind": "debut.title",
                "parameters": { "text": t.text, "font": t.style.font, "size_px": t.style.size_px },
                "available_range": null,
                "metadata": {},
            }),
        ),
        ClipSource::Shape(s) => (
            s.label().to_string(),
            json!({
                "OTIO_SCHEMA": "GeneratorReference.1",
                "generator_kind": "debut.shape",
                "parameters": serde_json::to_value(s).unwrap_or_default(),
                "available_range": null,
                "metadata": {},
            }),
        ),
        _ => match clip
            .media_at(clip.timeline_in)
            .and_then(|(id, _)| project.media.iter().find(|m| m.id == id))
        {
            Some(m) => (
                file_name(&m.path).to_string(),
                json!({
                    "OTIO_SCHEMA": "ExternalReference.1",
                    "target_url": file_url(&m.path),
                    "available_range": null,
                    "metadata": {},
                }),
            ),
            None => (
                "missing".to_string(),
                json!({ "OTIO_SCHEMA": "MissingReference.1", "metadata": {} }),
            ),
        },
    };
    // Multicam: the active angle plays; its offset shifts the source range.
    let range = match clip.media_at(clip.timeline_in) {
        Some((_, src)) => time_range(frames(src, rate), frames(clip.duration, rate), rate),
        None => range,
    };
    json!({
        "OTIO_SCHEMA": "Clip.1",
        "name": name,
        "source_range": range,
        "media_reference": reference,
        "effects": time_warp(clip),
        "markers": clip_markers,
        "metadata": {},
    })
}

fn stack(seq: &Sequence, project: &Project, depth: usize) -> Value {
    let rate = seq.frame_rate;
    let (mut v, mut a) = (0, 0);
    let tracks: Vec<Value> = seq
        .tracks
        .iter()
        .map(|track| {
            let (kind, name) = match track.kind {
                TrackKind::Audio => {
                    a += 1;
                    ("Audio", format!("A{a}"))
                }
                _ => {
                    v += 1;
                    ("Video", format!("V{v}"))
                }
            };
            let mut children = Vec::new();
            let mut cursor = Rational::ZERO;
            for clip in &track.clips {
                if clip.timeline_in > cursor {
                    let gap = frames(clip.timeline_in, rate) - frames(cursor, rate);
                    children.push(json!({
                        "OTIO_SCHEMA": "Gap.1",
                        "source_range": time_range(0, gap, rate),
                        "effects": [], "markers": [], "metadata": {},
                    }));
                }
                if let Some(tr) = clip.transition_in {
                    let half = frames(tr.half(), rate);
                    children.push(json!({
                        "OTIO_SCHEMA": "Transition.1",
                        "name": "Dissolve",
                        "transition_type": "SMPTE_Dissolve",
                        "in_offset": rational_time(half, rate),
                        "out_offset": rational_time(frames(tr.duration, rate) - half, rate),
                        "metadata": {},
                    }));
                }
                children.push(item(clip, project, rate, depth));
                cursor = clip.timeline_out();
            }
            json!({ "OTIO_SCHEMA": "Track.1", "name": name, "kind": kind, "children": children, "markers": [], "effects": [], "metadata": {} })
        })
        .collect();
    json!({
        "OTIO_SCHEMA": "Stack.1",
        "name": seq.name,
        "children": tracks,
        "markers": markers(&seq.markers, rate),
        "effects": [],
        "metadata": {},
    })
}

/// OpenTimelineIO JSON of `seq` (resolving media and nested sequences in
/// `project`).
pub fn otio(seq: &Sequence, project: &Project) -> String {
    let doc = json!({
        "OTIO_SCHEMA": "Timeline.1",
        "name": seq.name,
        "global_start_time": rational_time(0, seq.frame_rate),
        "tracks": stack(seq, project, 0),
        "metadata": { "debut": { "width": seq.width, "height": seq.height } },
    });
    serde_json::to_string_pretty(&doc).expect("OTIO document serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::IdGen;
    use debut_project::media_ref::MediaMetadata;
    use debut_project::{Title, TitleStyle, Track, Transition, TransitionKind};

    fn secs(n: i64) -> Rational {
        Rational::from_int(n)
    }

    /// V1: A 0..4 (source 10..14), B 4..8 with a 1 s dissolve, a title 9..10.
    /// A1: A 0..4. One timeline marker. A nested sequence holding B.
    fn project() -> Project {
        let mut ids = IdGen::new(5);
        let mut p = Project::new(ids.fresh(), "p");
        let mref = |ids: &mut IdGen, path: &str, reel: Option<&str>| MediaRef {
            id: ids.fresh(),
            path: path.into(),
            online: true,
            metadata: MediaMetadata {
                reel: reel.map(str::to_string),
                ..Default::default()
            },
            proxies: vec![],
            keywords: vec![],
            rating: 0,
        };
        let a = mref(&mut ids, "/shoot/Interview A.mov", None);
        let b = mref(&mut ids, "/shoot/broll.mp4", Some("B001"));
        let mut seq = Sequence::new(ids.fresh(), "Cut 1", FrameRate::FPS_25, 1920, 1080);
        let mut v = Track::new(ids.fresh(), TrackKind::Video);
        v.clips.push(Clip::new(
            ids.fresh(),
            ClipSource::Media(a.id),
            secs(0),
            secs(4),
            secs(10),
        ));
        let mut cb = Clip::new(
            ids.fresh(),
            ClipSource::Media(b.id),
            secs(4),
            secs(4),
            secs(2),
        );
        cb.transition_in = Some(Transition {
            kind: TransitionKind::Dissolve,
            duration: secs(1),
        });
        v.clips.push(cb);
        v.clips.push(Clip::new(
            ids.fresh(),
            ClipSource::Title(Title {
                text: "Hello".into(),
                style: TitleStyle::default(),
            }),
            secs(9),
            secs(1),
            Rational::ZERO,
        ));
        let mut au = Track::new(ids.fresh(), TrackKind::Audio);
        au.clips.push(Clip::new(
            ids.fresh(),
            ClipSource::Media(a.id),
            secs(0),
            secs(4),
            secs(10),
        ));
        seq.tracks.push(v);
        seq.tracks.push(au);
        seq.markers.push(Marker::new(ids.fresh(), secs(2), "check"));
        p.media = vec![a, b];
        p.sequences.push(seq);
        p
    }

    #[test]
    fn retimed_clips_carry_motion_effects() {
        let mut p = project();
        // A at double speed: record 0..3.5 (up to the dissolve) shows source 10..17.
        p.sequences[0].tracks[0].clips[0].speed = secs(2);
        let text = edl(&p.sequences[0], &p.media);
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            lines.contains(
                &"001  INTERVIE V     C        00:00:10:00 00:00:17:00 00:00:00:00 00:00:03:12"
            ),
            "{text}"
        );
        assert!(
            lines.contains(&"M2   INTERVIE       050.0                00:00:10:00"),
            "{text}"
        );
        let tl: Value = serde_json::from_str(&otio(&p.sequences[0], &p)).unwrap();
        let fx = &tl["tracks"]["children"][0]["children"][0]["effects"][0];
        assert_eq!(
            (fx["OTIO_SCHEMA"].as_str(), fx["time_scalar"].as_f64()),
            (Some("LinearTimeWarp.1"), Some(2.0))
        );
        p.sequences[0].tracks[0].clips[0].speed = Rational::ZERO;
        let tl: Value = serde_json::from_str(&otio(&p.sequences[0], &p)).unwrap();
        assert_eq!(
            tl["tracks"]["children"][0]["children"][0]["effects"][0]["OTIO_SCHEMA"],
            "FreezeFrame.1"
        );
    }

    #[test]
    fn edl_lists_cuts_dissolves_and_skipped_titles() {
        let p = project();
        let text = edl(&p.sequences[0], &p.media);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(&lines[..2], &["TITLE: Cut 1", "FCM: NON-DROP FRAME"]);
        // A runs to the middle of the dissolve start: record 0..3.5 s.
        assert_eq!(
            lines[3],
            "001  INTERVIE V     C        00:00:10:00 00:00:13:12 00:00:00:00 00:00:03:12"
        );
        assert_eq!(lines[4], "* FROM CLIP NAME: Interview A.mov");
        // Dissolve pair: zero-length cut on A, then 25-frame dissolve into B.
        assert_eq!(
            lines[5],
            "002  INTERVIE V     C        00:00:13:12 00:00:13:12 00:00:03:12 00:00:03:12"
        );
        assert_eq!(
            lines[6],
            "002  B001     V     D    025 00:00:01:12 00:00:06:00 00:00:03:12 00:00:08:00"
        );
        assert!(text.contains("* SKIPPED TITLE \"Hello\" AT 00:00:09:00"));
        assert!(text.contains(
            "003  INTERVIE A     C        00:00:10:00 00:00:14:00 00:00:00:00 00:00:04:00"
        ));
    }

    #[test]
    fn otio_has_tracks_gaps_transitions_generators_and_markers() {
        let mut p = project();
        // Nest a sequence: a second sequence with one clip, used on a new V2.
        let inner = p.sequences[0].clone();
        let mut inner = Sequence {
            id: debut_core::IdGen::new(77).fresh(),
            name: "Inner".into(),
            ..inner
        };
        inner.tracks.truncate(1);
        let inner_id = inner.id;
        p.sequences.push(inner);
        let mut v2 = Track::new(debut_core::IdGen::new(78).fresh(), TrackKind::Video);
        v2.clips.push(Clip::new(
            debut_core::IdGen::new(79).fresh(),
            ClipSource::Sequence(inner_id),
            secs(1),
            secs(2),
            secs(4),
        ));
        p.sequences[0].tracks.push(v2);

        let doc: Value = serde_json::from_str(&otio(&p.sequences[0], &p)).unwrap();
        assert_eq!(doc["OTIO_SCHEMA"], "Timeline.1");
        let tracks = doc["tracks"]["children"].as_array().unwrap();
        let names: Vec<&str> = tracks.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["V1", "A1", "V2"]);
        let v1: Vec<&str> = tracks[0]["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["OTIO_SCHEMA"].as_str().unwrap())
            .collect();
        assert_eq!(v1, ["Clip.1", "Transition.1", "Clip.1", "Gap.1", "Clip.1"]);
        let ch = &tracks[0]["children"];
        assert_eq!(ch[0]["source_range"]["start_time"]["value"], 250.0);
        assert_eq!(
            ch[0]["media_reference"]["target_url"],
            "file:///shoot/Interview%20A.mov"
        );
        assert_eq!(ch[1]["in_offset"]["value"], 12.0);
        assert_eq!(ch[1]["out_offset"]["value"], 13.0);
        assert_eq!(
            ch[3]["source_range"]["duration"]["value"], 25.0,
            "gap 8..9 s"
        );
        assert_eq!(ch[4]["media_reference"]["generator_kind"], "debut.title");
        // Nested sequence: a gap, then a stack with its own tracks and range.
        let v2 = tracks[2]["children"].as_array().unwrap();
        assert_eq!(v2[1]["OTIO_SCHEMA"], "Stack.1");
        assert_eq!(v2[1]["name"], "Inner");
        assert_eq!(v2[1]["source_range"]["start_time"]["value"], 100.0);
        assert_eq!(v2[1]["children"][0]["name"], "V1");
        // Timeline markers on the top stack, with a named colour.
        let m = &doc["tracks"]["markers"][0];
        assert_eq!(
            (m["name"].as_str(), m["color"].as_str()),
            (Some("check"), Some("BLUE"))
        );
        assert_eq!(m["marked_range"]["start_time"]["value"], 50.0);
    }
}
