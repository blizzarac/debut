//! Final Cut Pro XML import (MED-12): FCPXML 1.x projects become sequences.
//!
//! The primary storyline goes to V1 (its clips' sound to A1 unless they are
//! video only); connected clips go to the track their lane names (lane 1 =
//! V2, lane -1 = A2), moving to the next track of that kind when they would
//! overlap something already there. Dissolves become transitions into the
//! next clip, `timeMap`s speed changes (constant, reverse, freeze or a ramp),
//! titles title clips, compound clips nested sequences, and markers clip or
//! timeline markers. Assets are matched to existing media by path; others
//! come back as new media for the caller to add.

use debut_core::{Error, FrameRate, IdGen, MediaId, Rational, Result, Timecode};
use debut_project::media_ref::{MediaMetadata, MediaRef};
use debut_project::retime::SpeedKey;
use debut_project::{
    Clip, ClipSource, Marker, Sequence, Title, TitleStyle, Track, TrackKind, Transition,
    TransitionKind,
};
use roxmltree::{Document, Node};
use std::collections::HashMap;

const MAX_NESTING: usize = 8;

/// What an import produced.
#[derive(Debug, Default)]
pub struct Imported {
    /// Media the project does not have yet.
    pub media: Vec<MediaRef>,
    /// One per FCPXML project, then any compound clips they use.
    pub sequences: Vec<Sequence>,
    /// How many of `sequences` are projects (the rest are nested).
    pub projects: usize,
    /// Elements that were skipped (multicam, audition, …), by name.
    pub skipped: Vec<String>,
}

/// "1001/30000s", "10s", "0s" → seconds.
pub fn parse_time(s: &str) -> Option<Rational> {
    let s = s.trim().strip_suffix('s')?;
    match s.split_once('/') {
        Some((n, d)) => {
            let (n, d) = (n.parse::<i64>().ok()?, d.parse::<i64>().ok()?);
            (d != 0).then(|| Rational::new(n, d))
        }
        None => {
            if let Ok(n) = s.parse::<i64>() {
                Some(Rational::from_int(n))
            } else {
                // Decimal seconds: keep to the microsecond.
                let f = s.parse::<f64>().ok()?;
                Some(Rational::new((f * 1e6).round() as i64, 1_000_000))
            }
        }
    }
}

fn time(n: Node, attr: &str) -> Rational {
    n.attribute(attr)
        .and_then(parse_time)
        .unwrap_or(Rational::ZERO)
}

/// `file:///a/b%20c.mov` → `/a/b c.mov`.
fn url_path(src: &str) -> String {
    let raw = src
        .strip_prefix("file://localhost")
        .or_else(|| src.strip_prefix("file://"))
        .unwrap_or(src);
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&raw[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    let path = String::from_utf8_lossy(&out).into_owned();
    // "/C:/x" on Windows-made files.
    if path.len() > 3 && path.as_bytes()[2] == b':' {
        path[1..].to_string()
    } else {
        path
    }
}

struct Format {
    rate: FrameRate,
    width: u32,
    height: u32,
}

struct Asset {
    media: MediaId,
    start: Rational,
    has_audio: bool,
}

struct Ctx<'a, 'd> {
    doc: &'a Document<'d>,
    ids: &'a mut IdGen,
    formats: HashMap<String, Format>,
    assets: HashMap<String, Asset>,
    /// Compound clip resources: id → nested sequence id once built.
    compounds: HashMap<String, debut_core::SequenceId>,
    out: Imported,
    existing: &'a [MediaRef],
}

/// Where items of one container land: a time in the container's own domain
/// maps to sequence time as `base + (t - start)`.
#[derive(Clone, Copy)]
struct Frame {
    base: Rational,
    start: Rational,
    /// Sequence-time window clips are trimmed to (a container clip shows
    /// only its own span of what it holds).
    window: Option<(Rational, Rational)>,
}

impl Frame {
    fn at(&self, t: Rational) -> Rational {
        self.base + (t - self.start)
    }

    /// Trim `clip` to the window; false when nothing of it is left.
    fn trim(&self, clip: &mut Clip) -> bool {
        let Some((lo, hi)) = self.window else {
            return true;
        };
        if clip.timeline_out() > hi {
            clip.duration = hi - clip.timeline_in;
        }
        if clip.timeline_in < lo && clip.timeline_out() > lo {
            clip.set_head(lo);
        }
        clip.timeline_in >= lo && clip.duration > Rational::ZERO
    }
}

/// Clips being collected for one sequence, by kind and track index.
#[derive(Default)]
struct Layout {
    video: Vec<Vec<Clip>>,
    audio: Vec<Vec<Clip>>,
    markers: Vec<Marker>,
}

impl Layout {
    /// Put `clip` on track `index` of its kind, or the first one after it
    /// where it does not overlap.
    fn place(&mut self, kind: TrackKind, index: usize, clip: Clip) {
        let tracks = if kind == TrackKind::Audio {
            &mut self.audio
        } else {
            &mut self.video
        };
        let mut i = index;
        loop {
            while tracks.len() <= i {
                tracks.push(Vec::new());
            }
            let free = tracks[i].iter().all(|c| {
                c.timeline_out() <= clip.timeline_in || clip.timeline_out() <= c.timeline_in
            });
            if free {
                tracks[i].push(clip);
                return;
            }
            i += 1;
        }
    }
}

fn title_of(n: Node) -> Title {
    let text: String = n
        .descendants()
        .filter(|d| d.has_tag_name("text"))
        .flat_map(|t| t.descendants().filter(|d| d.is_text()))
        .filter_map(|d| d.text())
        .collect();
    let mut style = TitleStyle::default();
    if let Some(ts) = n
        .descendants()
        .find(|d| d.has_tag_name("text-style") && d.attribute("font").is_some())
    {
        if let Some(f) = ts.attribute("font") {
            style.font = f.to_string();
        }
        if let Some(sz) = ts.attribute("fontSize").and_then(|v| v.parse::<f32>().ok()) {
            style.size_px = sz;
        }
        if let Some(c) = ts.attribute("fontColor") {
            let v: Vec<f32> = c
                .split_whitespace()
                .filter_map(|x| x.parse().ok())
                .collect();
            if v.len() >= 3 {
                let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
                style.color = [b(v[0]), b(v[1]), b(v[2]), v.get(3).map_or(255, |a| b(*a))];
            }
        }
    }
    Title {
        text: if text.is_empty() {
            "Title".into()
        } else {
            text
        },
        style,
    }
}

/// Speed from a `timeMap` (clip time → media time), relative to the clip.
fn retime(clip: &mut Clip, n: Node, start: Rational) {
    let pts: Vec<(Rational, Rational)> = n
        .children()
        .filter(|c| c.has_tag_name("timept"))
        .filter_map(|p| {
            Some((
                parse_time(p.attribute("time")?)?,
                parse_time(p.attribute("value")?)?,
            ))
        })
        .collect();
    if pts.len() < 2 {
        return;
    }
    let slope = |a: (Rational, Rational), b: (Rational, Rational)| -> f64 {
        let dt = (b.0 - a.0).as_f64();
        if dt == 0.0 {
            0.0
        } else {
            (b.1 - a.1).as_f64() / dt
        }
    };
    // Media time at the clip's first frame.
    clip.source_in = lerp_value(&pts, start);
    let speeds: Vec<f64> = pts.windows(2).map(|w| slope(w[0], w[1])).collect();
    let constant = speeds.iter().all(|s| (s - speeds[0]).abs() < 1e-6);
    if constant {
        clip.speed = Rational::new((speeds[0] * 1000.0).round() as i64, 1000);
    } else {
        clip.ramp = pts
            .windows(2)
            .zip(&speeds)
            .map(|(w, s)| SpeedKey {
                at: (w[0].0 - start).max(Rational::ZERO),
                speed: *s,
            })
            .collect();
    }
}

/// Media time at clip time `t`, interpolating a time map linearly.
fn lerp_value(pts: &[(Rational, Rational)], t: Rational) -> Rational {
    if t <= pts[0].0 {
        return pts[0].1;
    }
    for w in pts.windows(2) {
        if t <= w[1].0 {
            let span = (w[1].0 - w[0].0).as_f64();
            let f = if span == 0.0 {
                0.0
            } else {
                (t - w[0].0).as_f64() / span
            };
            let v = w[0].1.as_f64() + f * (w[1].1 - w[0].1).as_f64();
            return Rational::new((v * 1e6).round() as i64, 1_000_000);
        }
    }
    pts[pts.len() - 1].1
}

impl Ctx<'_, '_> {
    fn resources(&mut self) -> Result<()> {
        let Some(res) = self.doc.descendants().find(|n| n.has_tag_name("resources")) else {
            return Err(Error::InvalidArgument("FCPXML has no resources".into()));
        };
        for r in res.children().filter(|n| n.is_element()) {
            let Some(id) = r.attribute("id") else {
                continue;
            };
            match r.tag_name().name() {
                "format" => {
                    let fd = r.attribute("frameDuration").and_then(parse_time);
                    let rate = fd
                        .filter(|d| !d.is_zero())
                        .map(|d| FrameRate::new(d.den, d.num))
                        .unwrap_or(FrameRate::FPS_25);
                    let num = |a: &str| r.attribute(a).and_then(|v| v.parse().ok());
                    self.formats.insert(
                        id.into(),
                        Format {
                            rate,
                            width: num("width").unwrap_or(1920),
                            height: num("height").unwrap_or(1080),
                        },
                    );
                }
                "asset" => {
                    let src = r
                        .children()
                        .find(|c| c.has_tag_name("media-rep"))
                        .and_then(|m| m.attribute("src"))
                        .or_else(|| r.attribute("src"));
                    let Some(src) = src else { continue };
                    let path = url_path(src);
                    let has_audio = r.attribute("hasAudio") == Some("1");
                    let start = time(r, "start");
                    let rate = r
                        .attribute("format")
                        .and_then(|f| self.formats.get(f))
                        .map(|f| f.rate);
                    let media = match self
                        .existing
                        .iter()
                        .chain(&self.out.media)
                        .find(|m| m.path == path)
                    {
                        Some(m) => m.id,
                        None => {
                            let media: MediaId = self.ids.fresh();
                            let channels = r
                                .attribute("audioChannels")
                                .and_then(|c| c.parse().ok())
                                .unwrap_or(if has_audio { 2 } else { 0 });
                            let start_timecode =
                                rate.filter(|_| start > Rational::ZERO).map(|rate| {
                                    Timecode::from_frames((start * rate.0).round(), rate, false)
                                });
                            self.out.media.push(MediaRef {
                                id: media,
                                path,
                                online: true,
                                metadata: MediaMetadata {
                                    frame_rate: rate,
                                    start_timecode,
                                    audio_channels: channels,
                                    ..Default::default()
                                },
                                proxies: vec![],
                                keywords: vec![],
                                rating: 0,
                            });
                            media
                        }
                    };
                    self.assets.insert(
                        id.into(),
                        Asset {
                            media,
                            start,
                            has_audio,
                        },
                    );
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// A compound clip resource as a nested sequence (built once).
    fn compound(&mut self, id: &str, depth: usize) -> Option<debut_core::SequenceId> {
        if let Some(s) = self.compounds.get(id) {
            return Some(*s);
        }
        if depth >= MAX_NESTING {
            return None;
        }
        let doc = self.doc;
        let media = doc
            .descendants()
            .find(|n| n.has_tag_name("media") && n.attribute("id") == Some(id))?;
        let seq_node = media.children().find(|c| c.has_tag_name("sequence"))?;
        let name = media.attribute("name").unwrap_or("Compound Clip");
        let seq_id: debut_core::SequenceId = self.ids.fresh();
        self.compounds.insert(id.into(), seq_id);
        let seq = self.sequence(seq_node, name, seq_id, depth + 1);
        self.out.sequences.push(seq);
        Some(seq_id)
    }

    fn sequence(
        &mut self,
        n: Node,
        name: &str,
        id: debut_core::SequenceId,
        depth: usize,
    ) -> Sequence {
        let (rate, width, height) = n
            .attribute("format")
            .and_then(|f| self.formats.get(f))
            .map(|f| (f.rate, f.width, f.height))
            .unwrap_or((FrameRate::FPS_25, 1920, 1080));
        let mut seq = Sequence::new(id, name, rate, width, height);
        let mut layout = Layout::default();
        if let Some(spine) = n.children().find(|c| c.has_tag_name("spine")) {
            let frame = Frame {
                base: Rational::ZERO,
                start: Rational::ZERO,
                window: None,
            };
            self.container(spine, frame, None, &mut layout, depth);
        }
        // At least V1 and A1, like a new sequence.
        layout
            .video
            .resize_with(layout.video.len().max(1), Vec::new);
        layout
            .audio
            .resize_with(layout.audio.len().max(1), Vec::new);
        for (kind, tracks) in [
            (TrackKind::Video, layout.video),
            (TrackKind::Audio, layout.audio),
        ] {
            for mut clips in tracks {
                clips.sort_by_key(|c| c.timeline_in);
                let mut t = Track::new(self.ids.fresh(), kind);
                t.clips = clips;
                seq.tracks.push(t);
            }
        }
        layout.markers.sort_by_key(|m| m.at);
        seq.markers = layout.markers;
        seq
    }

    /// Items of a spine (lane None: the primary storyline).
    fn container(
        &mut self,
        parent: Node,
        frame: Frame,
        lane: Option<i32>,
        layout: &mut Layout,
        depth: usize,
    ) {
        let items: Vec<Node> = parent.children().filter(|c| c.is_element()).collect();
        self.items(&items, frame, lane, layout, depth);
    }

    /// Place `items` (siblings in one time domain, `frame`) on tracks: the
    /// storyline when `lane` is None, else as connected clips in that lane.
    fn items(
        &mut self,
        items: &[Node],
        frame: Frame,
        lane: Option<i32>,
        layout: &mut Layout,
        depth: usize,
    ) {
        let mut pending: Option<Rational> = None;
        for &item in items {
            let tag = item.tag_name().name();
            let offset = time(item, "offset");
            let at = frame.at(offset);
            match tag {
                "transition" => {
                    pending = Some(time(item, "duration"));
                    continue;
                }
                "marker" | "chapter-marker" => {
                    let start = time(item, "start");
                    let mut m = Marker::new(
                        self.ids.fresh(),
                        frame.at(start),
                        item.attribute("value").unwrap_or(""),
                    );
                    m.duration = time(item, "duration");
                    layout.markers.push(m);
                    continue;
                }
                "spine" => {
                    // A secondary storyline: its items run from its offset.
                    let l = item.attribute("lane").and_then(|v| v.parse().ok()).or(lane);
                    let f = Frame {
                        base: at,
                        start: Rational::ZERO,
                        window: frame.window,
                    };
                    self.container(item, f, l, layout, depth);
                    continue;
                }
                _ => {}
            }
            let item_lane = item
                .attribute("lane")
                .and_then(|v| v.parse::<i32>().ok())
                .or(lane);
            let duration = time(item, "duration");
            let start = time(item, "start");
            if duration <= Rational::ZERO {
                continue;
            }
            let transition = pending.take();
            let mut clips: Vec<(TrackKind, Clip)> = Vec::new();
            match tag {
                "clip"
                    if !item.children().any(|c| {
                        c.attribute("lane").is_none()
                            && matches!(c.tag_name().name(), "video" | "asset-clip" | "audio")
                    }) =>
                {
                    // A clip holding a storyline of its own (detached audio in
                    // a gap, …): walk its children in its time.
                    let kids: Vec<Node> = item
                        .children()
                        .filter(|c| {
                            matches!(
                                c.tag_name().name(),
                                "gap"
                                    | "clip"
                                    | "asset-clip"
                                    | "video"
                                    | "audio"
                                    | "title"
                                    | "ref-clip"
                                    | "spine"
                            )
                        })
                        .collect();
                    let window = Some((at, at + duration));
                    self.items(
                        &kids,
                        Frame {
                            base: at,
                            start,
                            window,
                        },
                        item_lane,
                        layout,
                        depth,
                    );
                    continue;
                }
                "asset-clip" | "clip" | "video" | "audio" => {
                    // `clip` wraps its media in `video` / `audio` children; its
                    // sound is there only when an `audio` child is.
                    let direct = |name: &str| {
                        item.children()
                            .find(|c| c.attribute("lane").is_none() && c.has_tag_name(name))
                    };
                    let media_node = if tag == "clip" {
                        direct("video")
                            .or_else(|| direct("asset-clip"))
                            .or_else(|| direct("audio"))
                    } else {
                        Some(item)
                    };
                    let wrapped_enable = (tag == "clip").then(|| {
                        match (
                            direct("video").or_else(|| direct("asset-clip")).is_some(),
                            direct("audio").is_some(),
                        ) {
                            (true, true) => "all",
                            (true, false) => "video",
                            _ => "audio",
                        }
                    });
                    let Some(asset) = media_node
                        .and_then(|m| m.attribute("ref"))
                        .and_then(|r| self.assets.get(r))
                    else {
                        self.out.skipped.push(tag.into());
                        continue;
                    };
                    let (media, asset_start, has_audio) =
                        (asset.media, asset.start, asset.has_audio);
                    let enable = item
                        .attribute("srcEnable")
                        .or(wrapped_enable)
                        .unwrap_or(if tag == "audio" { "audio" } else { "all" });
                    let mut clip = Clip::new(
                        self.ids.fresh(),
                        ClipSource::Media(media),
                        at,
                        duration,
                        start - asset_start,
                    );
                    if let Some(tm) = item.children().find(|c| c.has_tag_name("timeMap")) {
                        retime(&mut clip, tm, start);
                        clip.source_in -= asset_start;
                    }
                    if enable != "audio" {
                        clips.push((TrackKind::Video, clip.clone()));
                    }
                    if enable != "video" && has_audio {
                        let mut a = clip.clone();
                        a.id = self.ids.fresh();
                        clips.push((TrackKind::Audio, a));
                    }
                }
                "title" => {
                    let clip = Clip::new(
                        self.ids.fresh(),
                        ClipSource::Title(title_of(item)),
                        at,
                        duration,
                        Rational::ZERO,
                    );
                    clips.push((TrackKind::Video, clip));
                }
                "ref-clip" => {
                    let Some(nested) = item.attribute("ref").and_then(|r| self.compound(r, depth))
                    else {
                        self.out.skipped.push(tag.into());
                        continue;
                    };
                    let mut clip = Clip::new(
                        self.ids.fresh(),
                        ClipSource::Sequence(nested),
                        at,
                        duration,
                        start,
                    );
                    if let Some(tm) = item.children().find(|c| c.has_tag_name("timeMap")) {
                        retime(&mut clip, tm, start);
                    }
                    clips.push((TrackKind::Video, clip));
                }
                "gap" => {}
                other => {
                    self.out.skipped.push(other.into());
                    continue;
                }
            }
            for (kind, mut clip) in clips {
                if let Some(d) = transition {
                    clip.transition_in = Some(Transition {
                        kind: TransitionKind::Dissolve,
                        duration: d,
                    });
                }
                // Markers inside the clip, in its own time.
                for m in item.children().filter(|c| c.has_tag_name("marker")) {
                    if kind == TrackKind::Video || !clips_has_video(tag, item) {
                        let mut mk = Marker::new(
                            self.ids.fresh(),
                            time(m, "start") - start,
                            m.attribute("value").unwrap_or(""),
                        );
                        mk.duration = time(m, "duration");
                        clip.markers.push(mk);
                    }
                }
                let index = match (kind, item_lane) {
                    (_, None) => 0,
                    (TrackKind::Audio, Some(l)) if l < 0 => (-l) as usize,
                    (TrackKind::Audio, Some(_)) => 1,
                    (_, Some(l)) => l.max(0) as usize,
                };
                if frame.trim(&mut clip) {
                    layout.place(kind, index, clip);
                }
            }
            // Connected children run in this item's time.
            let inner = Frame {
                base: at,
                start,
                window: frame.window,
            };
            let children: Vec<Node> = item
                .children()
                .filter(|c| c.is_element() && c.attribute("lane").is_some())
                .collect();
            for child in children {
                let l = child.attribute("lane").and_then(|v| v.parse().ok());
                self.items(&[child], inner, l, layout, depth);
            }
            if tag == "gap" {
                for m in item.children().filter(|c| c.has_tag_name("marker")) {
                    let mut mk = Marker::new(
                        self.ids.fresh(),
                        inner.at(time(m, "start")),
                        m.attribute("value").unwrap_or(""),
                    );
                    mk.duration = time(m, "duration");
                    layout.markers.push(mk);
                }
            }
        }
    }
}

fn clips_has_video(tag: &str, item: Node) -> bool {
    tag != "audio" && item.attribute("srcEnable") != Some("audio")
}

/// Parse FCPXML `text`. `existing` is the project's media, reused by path.
pub fn import(text: &str, ids: &mut IdGen, existing: &[MediaRef]) -> Result<Imported> {
    // FCPXML files start with `<!DOCTYPE fcpxml>`; it declares no entities.
    let opts = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = Document::parse_with_options(text, opts)
        .map_err(|e| Error::InvalidArgument(format!("not XML: {e}")))?;
    if !doc.root_element().has_tag_name("fcpxml") {
        return Err(Error::InvalidArgument("not an FCPXML document".into()));
    }
    let mut ctx = Ctx {
        doc: &doc,
        ids,
        formats: HashMap::new(),
        assets: HashMap::new(),
        compounds: HashMap::new(),
        out: Imported::default(),
        existing,
    };
    ctx.resources()?;
    let projects: Vec<Node> = doc
        .descendants()
        .filter(|n| n.has_tag_name("project"))
        .collect();
    let mut main = Vec::new();
    for p in projects {
        let Some(seq) = p.children().find(|c| c.has_tag_name("sequence")) else {
            continue;
        };
        let id = ctx.ids.fresh();
        main.push(ctx.sequence(seq, p.attribute("name").unwrap_or("Imported"), id, 0));
    }
    if main.is_empty() {
        return Err(Error::InvalidArgument("FCPXML has no project".into()));
    }
    let mut out = ctx.out;
    out.projects = main.len();
    main.append(&mut out.sequences);
    out.sequences = main;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_project::Project;

    fn secs(n: i64) -> Rational {
        Rational::from_int(n)
    }

    /// The export test's project: V1 A (1..5, linked sound) and B (5..9,
    /// half speed, 1 s dissolve), a title on V2 (6..8), a music bed on A1
    /// (9..11), a marker at 3 s.
    fn project(ids: &mut IdGen) -> Project {
        let mut p = Project::new(ids.fresh(), "Proj");
        let media = |ids: &mut IdGen, path: &str, tc: Option<&str>| MediaRef {
            id: ids.fresh(),
            path: path.into(),
            online: true,
            metadata: MediaMetadata {
                frame_rate: Some(FrameRate::FPS_25),
                start_timecode: tc.and_then(Timecode::parse),
                audio_channels: 2,
                ..Default::default()
            },
            proxies: vec![],
            keywords: vec![],
            rating: 0,
        };
        let a = media(ids, "/shoot/A cam.mov", Some("01:00:00:00"));
        let b = media(ids, "/shoot/b.mp4", None);
        let mut seq = Sequence::new(ids.fresh(), "Cut 1", FrameRate::FPS_25, 1920, 1080);
        let mut v1 = Track::new(ids.fresh(), TrackKind::Video);
        let mut v2 = Track::new(ids.fresh(), TrackKind::Video);
        let mut a1 = Track::new(ids.fresh(), TrackKind::Audio);
        let ca = Clip::new(
            ids.fresh(),
            ClipSource::Media(a.id),
            secs(1),
            secs(4),
            secs(10),
        );
        let mut cb = Clip::new(
            ids.fresh(),
            ClipSource::Media(b.id),
            secs(5),
            secs(4),
            secs(2),
        );
        cb.speed = Rational::new(1, 2);
        cb.transition_in = Some(Transition {
            kind: TransitionKind::Dissolve,
            duration: secs(1),
        });
        let mut linked = ca.clone();
        linked.id = ids.fresh();
        a1.clips.push(linked);
        a1.clips.push(Clip::new(
            ids.fresh(),
            ClipSource::Media(b.id),
            secs(9),
            secs(2),
            secs(0),
        ));
        v1.clips.extend([ca, cb]);
        v2.clips.push(Clip::new(
            ids.fresh(),
            ClipSource::Title(Title {
                text: "Hello & welcome".into(),
                style: TitleStyle::default(),
            }),
            secs(6),
            secs(2),
            Rational::ZERO,
        ));
        seq.tracks = vec![v1, v2, a1];
        seq.markers.push(Marker::new(ids.fresh(), secs(3), "check"));
        p.media = vec![a, b];
        p.sequences.push(seq);
        p
    }

    #[test]
    fn round_trips_our_own_export() {
        let mut ids = IdGen::new(21);
        let p = project(&mut ids);
        let durations = HashMap::from([(p.media[0].id, secs(60)), (p.media[1].id, secs(30))]);
        let xml = crate::fcpxml::fcpxml(&p.sequences[0], &p, &durations);

        // Into a project that has the media: nothing new, same ids.
        let got = import(&xml, &mut ids, &p.media).unwrap();
        assert!(
            got.media.is_empty() && got.skipped.is_empty(),
            "{:?}",
            got.skipped
        );
        assert_eq!((got.projects, got.sequences.len()), (1, 1));
        let seq = &got.sequences[0];
        assert_eq!(
            (seq.name.as_str(), seq.width, seq.frame_rate),
            ("Cut 1", 1920, FrameRate::FPS_25)
        );
        let tracks = |k: TrackKind| {
            seq.tracks
                .iter()
                .filter(move |t| t.kind == k)
                .collect::<Vec<_>>()
        };
        let (video, audio) = (tracks(TrackKind::Video), tracks(TrackKind::Audio));
        let v1 = &video[0].clips;
        assert_eq!(v1.len(), 2);
        assert_eq!(
            (
                v1[0].timeline_in,
                v1[0].duration,
                v1[0].source_in,
                v1[0].source.clone()
            ),
            (secs(1), secs(4), secs(10), ClipSource::Media(p.media[0].id))
        );
        assert_eq!(
            (v1[1].timeline_in, v1[1].speed, v1[1].source_in),
            (secs(5), Rational::new(1, 2), secs(2))
        );
        assert_eq!(v1[1].transition_in.map(|t| t.duration), Some(secs(1)));
        // The marker came back on A (clip time 2 s = sequence 3 s).
        assert_eq!(
            v1[0]
                .markers
                .iter()
                .map(|m| (m.at, m.note.as_str()))
                .collect::<Vec<_>>(),
            vec![(secs(2), "check")]
        );
        // A's sound is linked again on A1; B was video only.
        assert_eq!(audio[0].clips[0].timeline_in, secs(1));
        assert_eq!(audio[0].clips[0].source_in, secs(10));
        // The title on V2 and the music bed on an audio track at 9 s.
        let title = &video[1].clips[0];
        assert!(matches!(&title.source, ClipSource::Title(t) if t.text == "Hello & welcome"));
        assert_eq!((title.timeline_in, title.duration), (secs(6), secs(2)));
        assert!(audio.iter().any(|t| t
            .clips
            .iter()
            .any(|c| c.timeline_in == secs(9) && c.duration == secs(2))));

        // Into an empty project: the assets become media, paths decoded.
        let fresh = import(&xml, &mut ids, &[]).unwrap();
        let paths: Vec<&str> = fresh.media.iter().map(|m| m.path.as_str()).collect();
        assert_eq!(paths, vec!["/shoot/A cam.mov", "/shoot/b.mp4"]);
        assert_eq!(
            fresh.media[0]
                .metadata
                .start_timecode
                .map(|t| t.to_string())
                .as_deref(),
            Some("01:00:00:00")
        );
    }

    #[test]
    fn reads_clip_wrappers_ramps_and_refuses_non_fcpxml() {
        let xml = r#"<?xml version="1.0"?><fcpxml version="1.8"><resources>
            <format id="f" frameDuration="1001/30000s" width="1280" height="720"/>
            <asset id="a" name="x" start="0s" duration="100s" hasVideo="1" hasAudio="0" src="file:///m/x%20y.mov"/>
          </resources><library><event><project name="P"><sequence format="f"><spine>
            <clip offset="0s" start="0s" duration="10s"><video ref="a" offset="0s" duration="10s"/>
              <timeMap><timept time="0s" value="0s"/><timept time="5s" value="5s"/><timept time="10s" value="20s"/></timeMap>
            </clip>
            <multicam-clip offset="10s" duration="2s"/>
          </spine></sequence></project></event></library></fcpxml>"#;
        let mut ids = IdGen::new(3);
        let got = import(xml, &mut ids, &[]).unwrap();
        assert_eq!(got.media[0].path, "/m/x y.mov");
        let seq = &got.sequences[0];
        assert_eq!(seq.frame_rate, FrameRate::FPS_29_97);
        let c = &seq.tracks[0].clips[0];
        // 1x for 5 s, then 3x: a ramp with keys at 0 and 5.
        assert_eq!(
            c.ramp.iter().map(|k| (k.at, k.speed)).collect::<Vec<_>>(),
            vec![(secs(0), 1.0), (secs(5), 3.0)]
        );
        assert_eq!(got.skipped, vec!["multicam-clip".to_string()]);
        assert!(import("<xmeml/>", &mut ids, &[]).is_err());
        assert!(import("not xml", &mut ids, &[]).is_err());
    }
}
