//! Final Cut Pro XML (FCPXML 1.10) export (MED-12).
//!
//! The first video track becomes the primary storyline (with gaps where it is
//! empty); other video tracks become connected clips in lanes above it and
//! audio clips connected clips in lanes below, each anchored to the storyline
//! item it starts over. Audio linked to a storyline clip (same source and
//! span) rides inside that clip instead of being repeated. Dissolves become
//! Cross Dissolve transitions, titles Basic Titles, nested sequences compound
//! clips, speed changes `timeMap`s, and markers sit in the items they fall on.

use crate::interchange::{file_name, file_url};
use debut_core::{MediaId, Rational, SequenceId};
use debut_project::media_ref::MediaRef;
use debut_project::{Clip, ClipSource, Marker, Project, Sequence, TrackKind};
use std::collections::HashMap;

const MAX_NESTING: usize = 8;
const CROSS_DISSOLVE_UID: &str = "FxPlug:4731E73A-8DAC-4113-9A30-AE85B1761265";
const BASIC_TITLE_UID: &str =
    ".../Titles.localized/Bumper:Opener.localized/Basic Title.localized/Basic Title.moti";

/// FCPXML time: rational seconds.
fn t(r: Rational) -> String {
    if r.den == 1 {
        format!("{}s", r.num)
    } else {
        format!("{}/{}s", r.num, r.den)
    }
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\n' => out.push_str("&#10;"),
            c => out.push(c),
        }
    }
    out
}

struct Ctx<'a> {
    project: &'a Project,
    durations: &'a HashMap<MediaId, Rational>,
    resources: Vec<String>,
    next_id: usize,
    formats: HashMap<(u32, u32, Rational), String>,
    assets: HashMap<MediaId, String>,
    compounds: HashMap<SequenceId, String>,
    dissolve: Option<String>,
    title: Option<String>,
    text_styles: usize,
}

impl Ctx<'_> {
    fn id(&mut self) -> String {
        self.next_id += 1;
        format!("r{}", self.next_id)
    }

    fn format(&mut self, seq: &Sequence) -> String {
        let key = (seq.width, seq.height, seq.frame_rate.0);
        if let Some(id) = self.formats.get(&key) {
            return id.clone();
        }
        let id = self.id();
        self.resources.push(format!(
            r#"<format id="{id}" frameDuration="{}" width="{}" height="{}" colorSpace="1-1-1 (Rec. 709)"/>"#,
            t(seq.frame_rate.frame_duration()),
            seq.width,
            seq.height
        ));
        self.formats.insert(key, id.clone());
        id
    }

    fn media(&self, id: MediaId) -> Option<&MediaRef> {
        self.project.media.iter().find(|m| m.id == id)
    }

    /// Where the asset's own time starts: its start timecode, else zero.
    fn asset_start(&self, id: MediaId) -> Rational {
        self.media(id)
            .and_then(|m| {
                let rate = m.metadata.frame_rate?;
                let tc = m.metadata.start_timecode?;
                Some(rate.frame_to_time(tc.to_frames(rate)))
            })
            .unwrap_or(Rational::ZERO)
    }

    fn asset(&mut self, id: MediaId, format: &str, fallback_len: Rational) -> Option<String> {
        if let Some(r) = self.assets.get(&id) {
            return Some(r.clone());
        }
        let m = self.media(id)?.clone();
        let rid = self.id();
        let duration = self.durations.get(&id).copied().unwrap_or(fallback_len);
        let has_audio = m.metadata.audio_channels > 0;
        let audio = if has_audio {
            format!(
                r#" audioSources="1" audioChannels="{}" audioRate="48000""#,
                m.metadata.audio_channels
            )
        } else {
            String::new()
        };
        self.resources.push(format!(
            r#"<asset id="{rid}" name="{}" start="{}" duration="{}" hasVideo="1" hasAudio="{}" format="{format}"{audio}><media-rep kind="original-media" src="{}"/></asset>"#,
            esc(stem(&m.path)),
            t(self.asset_start(id)),
            t(duration),
            u8::from(has_audio),
            esc(&file_url(&m.path)),
        ));
        self.assets.insert(id, rid.clone());
        Some(rid)
    }

    fn dissolve(&mut self) -> String {
        if let Some(id) = &self.dissolve {
            return id.clone();
        }
        let id = self.id();
        self.resources.push(format!(
            r#"<effect id="{id}" name="Cross Dissolve" uid="{CROSS_DISSOLVE_UID}"/>"#
        ));
        self.dissolve = Some(id.clone());
        id
    }

    fn title_effect(&mut self) -> String {
        if let Some(id) = &self.title {
            return id.clone();
        }
        let id = self.id();
        self.resources.push(format!(
            r#"<effect id="{id}" name="Basic Title" uid="{BASIC_TITLE_UID}"/>"#
        ));
        self.title = Some(id.clone());
        id
    }
}

fn stem(path: &str) -> &str {
    let f = file_name(path);
    f.rsplit_once('.').map_or(f, |(s, _)| s)
}

/// One storyline item: where it sits, its own time at that point, and the
/// XML around the children (connected clips and markers) it collects.
struct Item {
    offset: Rational,
    start: Rational,
    open: String,
    inner: Vec<String>,
    close: String,
    /// Transitions take no storyline time and anchor nothing.
    anchors: bool,
}

impl Item {
    fn render(self) -> String {
        if self.inner.is_empty() {
            // Self-close childless items for readability.
            let tag_end = self.open.len() - 1;
            return format!("{}/>", &self.open[..tag_end]);
        }
        format!("{}{}{}", self.open, self.inner.concat(), self.close)
    }
}

fn gap(offset: Rational, duration: Rational) -> Item {
    Item {
        offset,
        start: Rational::ZERO,
        open: format!(
            r#"<gap name="Gap" offset="{}" start="0s" duration="{}">"#,
            t(offset),
            t(duration)
        ),
        inner: Vec::new(),
        close: "</gap>".into(),
        anchors: true,
    }
}

fn marker_xml(m: &Marker, start: Rational) -> String {
    format!(
        r#"<marker start="{}" duration="{}" value="{}"/>"#,
        t(start),
        t(m.duration.max(Rational::new(1, 1000))),
        esc(&m.note)
    )
}

/// The `timeMap` of a retimed clip: clip time (from `start`) to asset time,
/// sampled at the ends, every ramp key and between keys.
fn time_map(clip: &Clip, start: Rational, asset_start: Rational) -> String {
    let mut xs = vec![Rational::ZERO, clip.duration];
    for w in clip.ramp.windows(2) {
        xs.push((w[0].at + w[1].at) * Rational::new(1, 2));
    }
    xs.extend(clip.ramp.iter().map(|k| k.at));
    xs.retain(|x| *x >= Rational::ZERO && *x <= clip.duration);
    xs.sort();
    xs.dedup();
    let points: String = xs
        .iter()
        .filter_map(|x| {
            let (_, src) = clip.media_at(clip.timeline_in + *x)?;
            Some(format!(
                r#"<timept time="{}" value="{}" interp="linear"/>"#,
                t(start + *x),
                t(asset_start + src)
            ))
        })
        .collect();
    format!("<timeMap>{points}</timeMap>")
}

/// A clip as a storyline item (`lane` None) or a connected clip in `lane`,
/// placed at `offset` in its parent's time. `audio` picks which of the
/// asset's streams it uses: Some(true) audio only, Some(false) video only.
fn clip_item(
    clip: &Clip,
    offset: Rational,
    lane: Option<i32>,
    audio: Option<bool>,
    ctx: &mut Ctx,
    seq: &Sequence,
    depth: usize,
) -> Option<Item> {
    let lane_attr = lane.map(|l| format!(r#" lane="{l}""#)).unwrap_or_default();
    let common = |name: &str, start: Rational| {
        format!(
            r#"name="{}" offset="{}" start="{}" duration="{}"{lane_attr}"#,
            esc(name),
            t(offset),
            t(start),
            t(clip.duration)
        )
    };
    let markers = |start: Rational| -> Vec<String> {
        clip.markers
            .iter()
            .map(|m| marker_xml(m, start + m.at))
            .collect()
    };
    match &clip.source {
        ClipSource::Media(_) | ClipSource::Multicam { .. } => {
            let (media, src) = clip.media_at(clip.timeline_in)?;
            let format = ctx.format(seq);
            let asset = ctx.asset(media, &format, clip.source_out().max(src))?;
            let asset_start = ctx.asset_start(media);
            let start = asset_start + src.min(clip.source_out());
            let name = ctx.media(media).map(|m| stem(&m.path).to_string())?;
            let enable = match audio {
                Some(true) => r#" srcEnable="audio""#,
                Some(false) => r#" srcEnable="video""#,
                None => "",
            };
            let mut inner = Vec::new();
            if clip.is_retimed() {
                inner.push(time_map(clip, start, asset_start));
            }
            inner.extend(markers(start));
            Some(Item {
                offset,
                start,
                open: format!(
                    r#"<asset-clip ref="{asset}" {}{enable} tcFormat="NDF">"#,
                    common(&name, start)
                ),
                inner,
                close: "</asset-clip>".into(),
                anchors: true,
            })
        }
        ClipSource::Title(title) => {
            let effect = ctx.title_effect();
            ctx.text_styles += 1;
            let ts = format!("ts{}", ctx.text_styles);
            let s = &title.style;
            let color = s
                .color
                .iter()
                .map(|c| format!("{:.3}", *c as f32 / 255.0))
                .collect::<Vec<_>>()
                .join(" ");
            let mut inner = vec![format!(
                r#"<text><text-style ref="{ts}">{}</text-style></text><text-style-def id="{ts}"><text-style font="{}" fontSize="{}" fontColor="{color}" alignment="center"/></text-style-def>"#,
                esc(&title.text),
                esc(&s.font),
                s.size_px.round()
            )];
            inner.extend(markers(Rational::ZERO));
            Some(Item {
                offset,
                start: Rational::ZERO,
                open: format!(
                    r#"<title ref="{effect}" {}>"#,
                    common(title.text.lines().next().unwrap_or("Title"), Rational::ZERO)
                ),
                inner,
                close: "</title>".into(),
                anchors: true,
            })
        }
        ClipSource::Sequence(id) => {
            if depth >= MAX_NESTING {
                return None;
            }
            let nested = ctx.project.sequence(*id)?;
            let media = match ctx.compounds.get(id) {
                Some(r) => r.clone(),
                None => {
                    let rid = ctx.id();
                    ctx.compounds.insert(*id, rid.clone());
                    let body = sequence_xml(nested, ctx, depth + 1);
                    ctx.resources.push(format!(
                        r#"<media id="{rid}" name="{}">{body}</media>"#,
                        esc(&nested.name)
                    ));
                    rid
                }
            };
            let start = clip.source_in;
            let mut inner = Vec::new();
            if clip.is_retimed() {
                inner.push(time_map(clip, start, Rational::ZERO));
            }
            inner.extend(markers(start));
            Some(Item {
                offset,
                start,
                open: format!(
                    r#"<ref-clip ref="{media}" {}>"#,
                    common(&nested.name, start)
                ),
                inner,
                close: "</ref-clip>".into(),
                anchors: true,
            })
        }
    }
}

/// Clips on other tracks that play the same material over the same span.
fn linked_to(clip: &Clip, other: &[&Clip]) -> bool {
    other.iter().any(|o| {
        o.source == clip.source
            && o.timeline_in == clip.timeline_in
            && o.duration == clip.duration
            && o.source_in == clip.source_in
    })
}

fn sequence_xml(seq: &Sequence, ctx: &mut Ctx, depth: usize) -> String {
    let format = ctx.format(seq);
    let duration = seq.duration().max(seq.frame_rate.frame_duration());
    let videos: Vec<_> = seq
        .tracks
        .iter()
        .filter(|tr| tr.kind == TrackKind::Video)
        .collect();
    let audios: Vec<_> = seq
        .tracks
        .iter()
        .filter(|tr| tr.kind == TrackKind::Audio)
        .collect();
    let primary: Vec<&Clip> = videos
        .first()
        .map(|v| v.clips.iter().collect())
        .unwrap_or_default();
    let audio_clips: Vec<&Clip> = audios.iter().flat_map(|a| a.clips.iter()).collect();

    // Storyline: the primary track with gaps, dissolves between clips.
    let mut items: Vec<Item> = Vec::new();
    let mut at = Rational::ZERO;
    for clip in &primary {
        if clip.timeline_in > at {
            items.push(gap(at, clip.timeline_in - at));
        }
        if let Some(tr) = clip
            .transition_in
            .filter(|_| clip.timeline_in == at && at > Rational::ZERO)
        {
            let effect = ctx.dissolve();
            items.push(Item {
                offset: clip.timeline_in - tr.half(),
                start: Rational::ZERO,
                open: format!(
                    r#"<transition name="Cross Dissolve" offset="{}" duration="{}">"#,
                    t(clip.timeline_in - tr.half()),
                    t(tr.duration)
                ),
                inner: vec![format!(
                    r#"<filter-video ref="{effect}" name="Cross Dissolve"/>"#
                )],
                close: "</transition>".into(),
                anchors: false,
            });
        }
        // Its sound rides along when an audio track carries the linked part.
        let audio = linked_to(clip, &audio_clips);
        let has_audio = clip
            .media_at(clip.timeline_in)
            .and_then(|(m, _)| ctx.media(m))
            .is_some_and(|m| m.metadata.audio_channels > 0);
        let enable = (has_audio && !audio).then_some(false);
        match clip_item(clip, clip.timeline_in, None, enable, ctx, seq, depth) {
            Some(item) => items.push(item),
            None => items.push(gap(clip.timeline_in, clip.duration)),
        }
        at = clip.timeline_out();
    }
    if duration > at {
        items.push(gap(at, duration - at));
    }

    // Connected clips: video above, audio below, anchored where they start.
    let mut connected: Vec<(&Clip, i32, bool)> = Vec::new();
    for (i, v) in videos.iter().enumerate().skip(1) {
        connected.extend(v.clips.iter().map(|c| (c, i as i32, false)));
    }
    for (i, a) in audios.iter().enumerate() {
        connected.extend(
            a.clips
                .iter()
                .filter(|c| !linked_to(c, &primary))
                .map(|c| (c, -(i as i32) - 1, true)),
        );
    }
    let mut anchored = Vec::new();
    for (clip, lane, audio) in connected {
        let Some(host) = items
            .iter()
            .rposition(|it| it.anchors && it.offset <= clip.timeline_in)
        else {
            continue;
        };
        let offset = items[host].start + (clip.timeline_in - items[host].offset);
        let is_media = matches!(
            clip.source,
            ClipSource::Media(_) | ClipSource::Multicam { .. }
        );
        let enable = is_media.then_some(audio);
        if let Some(item) = clip_item(clip, offset, Some(lane), enable, ctx, seq, depth) {
            anchored.push((host, item.render()));
        }
    }
    for (host, xml) in anchored {
        items[host].inner.push(xml);
    }
    for m in &seq.markers {
        if let Some(host) = items.iter().rposition(|it| it.anchors && it.offset <= m.at) {
            let start = items[host].start + (m.at - items[host].offset);
            items[host].inner.push(marker_xml(m, start));
        }
    }
    let spine: String = items.into_iter().map(Item::render).collect();
    format!(
        r#"<sequence format="{format}" duration="{}" tcStart="0s" tcFormat="NDF" audioLayout="stereo" audioRate="48k"><spine>{spine}</spine></sequence>"#,
        t(duration)
    )
}

/// FCPXML for `seq`. `durations` gives each media's length (the asset
/// duration); media missing from it get the furthest point a clip uses.
pub fn fcpxml(seq: &Sequence, project: &Project, durations: &HashMap<MediaId, Rational>) -> String {
    let mut ctx = Ctx {
        project,
        durations,
        resources: Vec::new(),
        next_id: 0,
        formats: HashMap::new(),
        assets: HashMap::new(),
        compounds: HashMap::new(),
        dissolve: None,
        title: None,
        text_styles: 0,
    };
    let body = sequence_xml(seq, &mut ctx, 0);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE fcpxml>\n<fcpxml version=\"1.10\">\n<resources>\n{}\n</resources>\n<library>\n<event name=\"{}\">\n<project name=\"{}\">\n{body}\n</project>\n</event>\n</library>\n</fcpxml>\n",
        ctx.resources.join("\n"),
        esc(&project.name),
        esc(&seq.name)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::{FrameRate, IdGen, Timecode};
    use debut_project::media_ref::MediaMetadata;
    use debut_project::{Title, TitleStyle, Track, Transition, TransitionKind};

    fn secs(n: i64) -> Rational {
        Rational::from_int(n)
    }

    #[test]
    fn storyline_lanes_transitions_titles_and_speed() {
        let mut ids = IdGen::new(8);
        let mut p = Project::new(ids.fresh(), "Proj & Co");
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
        let a = media(&mut ids, "/shoot/A cam.mov", Some("01:00:00:00"));
        let b = media(&mut ids, "/shoot/b.mp4", None);
        let mut seq = Sequence::new(ids.fresh(), "Cut <1>", FrameRate::FPS_25, 1920, 1080);
        let mut v1 = Track::new(ids.fresh(), TrackKind::Video);
        let mut v2 = Track::new(ids.fresh(), TrackKind::Video);
        let mut a1 = Track::new(ids.fresh(), TrackKind::Audio);
        // V1: A 1..5 (linked audio on A1), B 5..9 at half speed with a dissolve.
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
        // A music bed on A1 from 9 s, not linked.
        a1.clips.push(Clip::new(
            ids.fresh(),
            ClipSource::Media(b.id),
            secs(9),
            secs(2),
            secs(0),
        ));
        v1.clips.extend([ca, cb]);
        // A title over B on V2.
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
        seq.markers
            .push(Marker::new(ids.fresh(), secs(3), "check <this>"));
        p.media = vec![a.clone(), b.clone()];
        p.sequences.push(seq);
        let durations = HashMap::from([(a.id, secs(60)), (b.id, secs(30))]);
        let xml = fcpxml(&p.sequences[0], &p, &durations);

        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE fcpxml>"));
        assert!(xml.contains(r#"frameDuration="1/25s" width="1920" height="1080""#));
        // Asset start is the 01:00:00:00 timecode; the clip enters 10 s in.
        assert!(
            xml.contains(r#"name="A cam" start="3600s" duration="60s""#),
            "{xml}"
        );
        assert!(
            xml.contains(r#"<gap name="Gap" offset="0s" start="0s" duration="1s"/>"#),
            "{xml}"
        );
        assert!(
            xml.contains(r#"name="A cam" offset="1s" start="3610s" duration="4s" tcFormat"#)
                || xml.contains(r#"name="A cam" offset="1s" start="3610s" duration="4s""#),
            "{xml}"
        );
        // A's sound is on A1, so it stays enabled; B's isn't, so B is video only.
        assert!(
            !xml.contains(r#"start="3610s" duration="4s" srcEnable"#),
            "{xml}"
        );
        assert!(xml.contains(r#"<transition name="Cross Dissolve" offset="9/2s" duration="1s">"#));
        assert!(xml.contains(r#"<timeMap><timept time="2s" value="2s" interp="linear"/><timept time="6s" value="4s" interp="linear"/></timeMap>"#), "{xml}");
        // The title is a connected clip in lane 1 inside B (B's time starts at 2 s).
        assert!(
            xml.contains(r#"<title ref="#)
                && xml.contains(r#"offset="3s" start="0s" duration="2s" lane="1""#),
            "{xml}"
        );
        assert!(xml.contains("Hello &amp; welcome"));
        // The unlinked music bed hangs below the trailing gap, audio only.
        assert!(xml.contains(r#"lane="-1" srcEnable="audio""#), "{xml}");
        // The 3 s marker sits in A (its time 3610 + 2).
        assert!(xml.contains(r#"<marker start="3612s""#) && xml.contains("check &lt;this&gt;"));
        assert!(
            xml.contains(r#"<event name="Proj &amp; Co">"#)
                && xml.contains(r#"<project name="Cut &lt;1&gt;">"#)
        );
    }
}
