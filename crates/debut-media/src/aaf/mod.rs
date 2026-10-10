//! AAF export (MED-12): the sequence as an Advanced Authoring Format file,
//! the interchange Avid Media Composer, Pro Tools and Resolve read.
//!
//! The file carries the full AAF object model (the MetaDictionary) and the
//! standard Dictionary, both taken from `baseline.json`, which
//! `tools/aaf_baseline.py` generates with pyaaf2 (MIT, see `LICENSE.pyaaf2`).
//! The timeline itself is built here:
//!
//! - each media file becomes a file SourceMob (descriptors for its picture
//!   and sound, a network locator with its path) and a MasterMob that clips
//!   refer to;
//! - each sequence becomes a CompositionMob (top level for the exported one,
//!   lower level for nested ones) with one timeline slot per track, a
//!   timecode slot and an event slot of markers;
//! - clips are SourceClips, gaps Fillers, dissolves Transitions (the clips
//!   on both sides lengthened by the overlap, as AAF wants);
//! - titles go out as Fillers: AAF has no portable title.
//!
//! Speed changes are not written (the clip keeps its timeline length at
//! normal speed), nor are clip effects, clip markers or audio levels.

mod cfb;
mod object;

use crate::interchange::{file_name, file_url};
use debut_core::{FrameRate, MediaId, Rational, SequenceId};
use debut_project::{Clip, ClipSource, Project, Sequence, TrackKind};
use object::{auid, mangle, utf16z, Key, Obj, Prop, WeakTable};
use std::collections::HashMap;

/// What the writer needs to know about a media file beyond its `MediaRef`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AafMedia {
    pub width: u32,
    pub height: u32,
    pub duration: Rational,
    pub has_video: bool,
    pub has_audio: bool,
}

/// A date and time for the file's timestamps (AAF stores local time with no zone).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stamp {
    pub year: i16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl Default for Stamp {
    fn default() -> Self {
        Self {
            year: 2000,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        }
    }
}

// Classes.
const ROOT: &str = "b3b398a5-1c90-11d4-8053-080036210804";
const HEADER: &str = "0d010101-0101-2f00-060e-2b3402060101";
const CONTENT_STORAGE: &str = "0d010101-0101-1800-060e-2b3402060101";
const IDENTIFICATION: &str = "0d010101-0101-3000-060e-2b3402060101";
const COMPOSITION_MOB: &str = "0d010101-0101-3500-060e-2b3402060101";
const MASTER_MOB: &str = "0d010101-0101-3600-060e-2b3402060101";
const SOURCE_MOB: &str = "0d010101-0101-3700-060e-2b3402060101";
const TIMELINE_SLOT: &str = "0d010101-0101-3b00-060e-2b3402060101";
const EVENT_SLOT: &str = "0d010101-0101-3900-060e-2b3402060101";
const SEQUENCE: &str = "0d010101-0101-0f00-060e-2b3402060101";
const SOURCE_CLIP: &str = "0d010101-0101-1100-060e-2b3402060101";
const FILLER: &str = "0d010101-0101-0900-060e-2b3402060101";
const TRANSITION: &str = "0d010101-0101-1700-060e-2b3402060101";
const OPERATION_GROUP: &str = "0d010101-0101-0a00-060e-2b3402060101";
const TIMECODE: &str = "0d010101-0101-1400-060e-2b3402060101";
const DESCRIPTIVE_MARKER: &str = "0d010101-0101-4100-060e-2b3402060101";
const MULTIPLE_DESCRIPTOR: &str = "0d010101-0101-4400-060e-2b3402060101";
const CDCI_DESCRIPTOR: &str = "0d010101-0101-2800-060e-2b3402060101";
const PCM_DESCRIPTOR: &str = "0d010101-0101-4800-060e-2b3402060101";
const NETWORK_LOCATOR: &str = "0d010101-0101-3200-060e-2b3402060101";

// Data definitions and other ids.
const PICTURE: &str = "01030202-0100-0000-060e-2b3404010101";
const SOUND: &str = "01030202-0200-0000-060e-2b3404010101";
const TIMECODE_DEF: &str = "01030201-0100-0000-060e-2b3404010101";
const DESCRIPTIVE: &str = "01030201-1000-0000-060e-2b3404010101";
const VIDEO_DISSOLVE: &str = "0c3bea40-fc05-11d2-8a29-0050040ef7d2";
const AUDIO_DISSOLVE: &str = "0c3bea44-fc05-11d2-8a29-0050040ef7d2";
const USAGE_TOP_LEVEL: &str = "0d010102-0101-0700-060e-2b3404010101";
const USAGE_LOWER_LEVEL: &str = "0d010102-0101-0800-060e-2b3404010101";
const OP_PATTERN_EDIT: &str = "0d011201-0100-0000-060e-2b3404010105";
const PRODUCT_ID: &str = "6a6e8b4c-1f5d-4b2e-9c43-64656275740a";

/// Weak references to definitions: Header / Dictionary / DataDefinitions or
/// OperationDefinitions, keyed by their identification.
const DATA_DEFS: [u16; 3] = [0x0002, 0x3b04, 0x2605];
const OPERATION_DEFS: [u16; 3] = [0x0002, 0x3b04, 0x2603];
const DEF_ID_PID: u16 = 0x1b01;

const BASELINE: &str = include_str!("baseline.json");

fn obj(class: &str, props: Vec<(u16, Prop)>) -> Obj {
    Obj {
        class: auid(class),
        props,
    }
}

fn data(v: impl Into<Vec<u8>>) -> Prop {
    Prop::Data(v.into())
}

fn strong(name: &str, pid: u16, o: Obj) -> (u16, Prop) {
    (pid, Prop::Strong(mangle(name, pid, 32), Box::new(o)))
}

fn vector(name: &str, pid: u16, items: Vec<Obj>) -> (u16, Prop) {
    (pid, Prop::Vector(mangle(name, pid, 22), items))
}

fn rational(r: Rational) -> Vec<u8> {
    let mut v = (r.num as i32).to_le_bytes().to_vec();
    v.extend_from_slice(&(r.den as i32).to_le_bytes());
    v
}

fn stamp(s: Stamp) -> Vec<u8> {
    let mut v = s.year.to_le_bytes().to_vec();
    v.extend_from_slice(&[s.month, s.day, s.hour, s.minute, s.second, 0]);
    v
}

fn data_def(kind: &str) -> (u16, Prop) {
    (
        0x0201,
        Prop::Weak {
            path: DATA_DEFS.to_vec(),
            key_pid: DEF_ID_PID,
            key: auid(kind).to_vec(),
        },
    )
}

/// A 32-byte UMID: SMPTE label, length, `instance`, and a 16-byte material number.
fn mob_id(material: u128, instance: u8) -> Key {
    let mut v = vec![
        0x06, 0x0a, 0x2b, 0x34, 0x01, 0x01, 0x01, 0x05, 0x01, 0x01, 0x0f, 0x20, 0x13, 0x00, 0x00,
        instance,
    ];
    v.extend_from_slice(&material.to_le_bytes());
    v
}

fn frames(t: Rational, rate: FrameRate) -> i64 {
    rate.time_to_frame(t)
}

/// A mob's common properties, slots last.
fn mob(class: &str, id: &Key, name: &str, slots: Vec<Obj>, when: Stamp) -> Obj {
    obj(
        class,
        vec![
            (0x4401, data(id.clone())),
            (0x4402, data(utf16z(name))),
            vector("Slots", 0x4403, slots),
            (0x4404, data(stamp(when))),
            (0x4405, data(stamp(when))),
        ],
    )
}

fn timeline_slot(id: u32, name: &str, rate: FrameRate, segment: Obj) -> Obj {
    obj(
        TIMELINE_SLOT,
        vec![
            (0x4801, data(id.to_le_bytes())),
            (0x4802, data(utf16z(name))),
            strong("Segment", 0x4803, segment),
            (0x4b01, data(rational(rate.0))),
            (0x4b02, data(0i64.to_le_bytes())),
        ],
    )
}

fn source_clip(kind: &str, length: i64, source: &Key, slot: u32, start: i64) -> Obj {
    obj(
        SOURCE_CLIP,
        vec![
            data_def(kind),
            (0x0202, data(length.to_le_bytes())),
            (0x1101, data(source.clone())),
            (0x1102, data(slot.to_le_bytes())),
            (0x1201, data(start.to_le_bytes())),
        ],
    )
}

fn filler(kind: &str, length: i64) -> Obj {
    obj(
        FILLER,
        vec![data_def(kind), (0x0202, data(length.to_le_bytes()))],
    )
}

fn sequence(kind: &str, components: Vec<Obj>, length: i64) -> Obj {
    obj(
        SEQUENCE,
        vec![
            data_def(kind),
            (0x0202, data(length.to_le_bytes())),
            vector("Components", 0x1001, components),
        ],
    )
}

/// Master and file mobs per media; slot ids within the master mob.
struct MediaMobs {
    master: Key,
    picture_slot: Option<u32>,
    sound_slot: Option<u32>,
}

/// The `.aaf` bytes for `seq` (resolving media and nested sequences in
/// `project`; `media` adds what the files' metadata lacks).
pub fn aaf(
    seq: &Sequence,
    project: &Project,
    media: &HashMap<MediaId, AafMedia>,
    when: Stamp,
) -> Result<Vec<u8>, String> {
    let rate = seq.frame_rate;
    let baseline: serde_json::Value =
        serde_json::from_str(BASELINE).map_err(|e| format!("AAF baseline: {e}"))?;
    let metadict = object::from_json(&baseline["metadict"])?;
    let dictionary = object::from_json(&baseline["dictionary"])?;

    // Sequences reachable from `seq` (itself first), and the media they use.
    let mut sequences: Vec<&Sequence> = vec![seq];
    let mut i = 0;
    while i < sequences.len() {
        for t in &sequences[i].tracks {
            for c in &t.clips {
                if let ClipSource::Sequence(id) = c.source {
                    if let Some(s) = project.sequence(id) {
                        if !sequences.iter().any(|x| x.id == s.id) {
                            sequences.push(s);
                        }
                    }
                }
            }
        }
        i += 1;
    }
    let mut used: Vec<MediaId> = Vec::new();
    for s in &sequences {
        for t in &s.tracks {
            for c in &t.clips {
                if let Some((m, _)) = c.media_at(c.timeline_in) {
                    if !used.contains(&m) {
                        used.push(m);
                    }
                }
            }
        }
    }

    let mut mobs: Vec<(Key, Obj)> = Vec::new();
    let mut media_mobs: HashMap<MediaId, MediaMobs> = HashMap::new();
    for id in used {
        let Some(mref) = project.media.iter().find(|m| m.id == id) else {
            continue;
        };
        let info = media.get(&id).copied().unwrap_or(AafMedia {
            width: 0,
            height: 0,
            duration: Rational::ZERO,
            has_video: false,
            has_audio: mref.metadata.audio_channels > 0,
        });
        // Long enough for every use, when the file's length is unknown.
        let used_len = sequences
            .iter()
            .flat_map(|s| s.tracks.iter().flat_map(|t| t.clips.iter()))
            .filter(|c| c.media_at(c.timeline_in).map(|(m, _)| m) == Some(id))
            .map(|c| frames(c.source_in + c.duration, rate) + frames(c.duration, rate))
            .max()
            .unwrap_or(0);
        let length = frames(info.duration, rate).max(used_len).max(1);
        let has_video = info.has_video || (!info.has_audio && info.width > 0);
        let has_audio = info.has_audio;
        let name = file_name(&mref.path);
        let file_id = mob_id(id.0, 1);
        let master_id = mob_id(id.0, 0);
        let mut kinds = Vec::new();
        if has_video {
            kinds.push(PICTURE);
        }
        if has_audio {
            kinds.push(SOUND);
        }
        if kinds.is_empty() {
            kinds.push(PICTURE);
        }
        let null = vec![0u8; 32];
        let file_slots: Vec<Obj> = kinds
            .iter()
            .enumerate()
            .map(|(i, k)| {
                timeline_slot(i as u32 + 1, "", rate, source_clip(k, length, &null, 0, 0))
            })
            .collect();
        let master_slots: Vec<Obj> = kinds
            .iter()
            .enumerate()
            .map(|(i, k)| {
                timeline_slot(
                    i as u32 + 1,
                    "",
                    rate,
                    source_clip(k, length, &file_id, i as u32 + 1, 0),
                )
            })
            .collect();
        let mut descriptors = Vec::new();
        if has_video {
            let (w, h) = (info.width.max(1), info.height.max(1));
            descriptors.push(obj(
                CDCI_DESCRIPTOR,
                vec![
                    (0x3001, data(rational(rate.0))),
                    (0x3002, data(length.to_le_bytes())),
                    (0x3202, data(h.to_le_bytes())),
                    (0x3203, data(w.to_le_bytes())),
                    (0x320c, data([0u8])), // FullFrame
                    (0x320d, data([0u8; 8])),
                    (0x320e, data(rational(Rational::new(w as i64, h as i64)))),
                    (0x3301, data(8u32.to_le_bytes())),
                    (0x3302, data(2u32.to_le_bytes())),
                ],
            ));
        }
        if has_audio {
            let channels = (mref.metadata.audio_channels.max(1)) as u32;
            // Sound descriptors count samples.
            let samples =
                (Rational::from_int(length) * rate.frame_duration() * Rational::from_int(48_000))
                    .round();
            descriptors.push(obj(
                PCM_DESCRIPTOR,
                vec![
                    (0x3001, data(rational(Rational::from_int(48_000)))),
                    (0x3002, data(samples.to_le_bytes())),
                    (0x3d01, data(16u32.to_le_bytes())),
                    (0x3d03, data(rational(Rational::from_int(48_000)))),
                    (0x3d07, data(channels.to_le_bytes())),
                    (0x3d09, data((48_000 * 2 * channels).to_le_bytes())),
                    (0x3d0a, data(((2 * channels) as u16).to_le_bytes())),
                ],
            ));
        }
        let locator = obj(
            NETWORK_LOCATOR,
            vec![(0x4001, data(utf16z(&file_url(&mref.path))))],
        );
        let descriptor = obj(
            MULTIPLE_DESCRIPTOR,
            vec![
                vector("Locator", 0x2f01, vec![locator]),
                (0x3001, data(rational(rate.0))),
                (0x3002, data(length.to_le_bytes())),
                vector("FileDescriptors", 0x3f01, descriptors),
            ],
        );
        let mut file_mob = mob(SOURCE_MOB, &file_id, name, file_slots, when);
        file_mob
            .props
            .push(strong("EssenceDescription", 0x4701, descriptor));
        mobs.push((file_id.clone(), file_mob));
        let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
        mobs.push((
            master_id.clone(),
            mob(MASTER_MOB, &master_id, stem, master_slots, when),
        ));
        media_mobs.insert(
            id,
            MediaMobs {
                master: master_id,
                picture_slot: has_video.then_some(1),
                sound_slot: has_audio.then_some(if has_video { 2 } else { 1 }),
            },
        );
    }

    // Composition mobs: slot ids of each sequence's first video and audio track.
    let comp_ids: HashMap<SequenceId, Key> = sequences
        .iter()
        .map(|s| (s.id, mob_id(s.id.0, 2)))
        .collect();
    let first_slots = |s: &Sequence| -> (Option<u32>, Option<u32>) {
        let v = s.tracks.iter().position(|t| t.kind != TrackKind::Audio);
        let a = s.tracks.iter().position(|t| t.kind == TrackKind::Audio);
        (v.map(|i| i as u32 + 1), a.map(|i| i as u32 + 1))
    };
    for (n, s) in sequences.iter().enumerate() {
        let r = s.frame_rate;
        let mut slots = Vec::new();
        let (mut vn, mut an) = (0, 0);
        for (ti, track) in s.tracks.iter().enumerate() {
            let audio = track.kind == TrackKind::Audio;
            let kind = if audio { SOUND } else { PICTURE };
            let name = if audio {
                an += 1;
                format!("A{an}")
            } else {
                vn += 1;
                format!("V{vn}")
            };
            let total = frames(s.duration(), r);
            let (components, length) = track_components(&track.clips, audio, r, total, |clip| {
                // What a clip points at: (mob, slot, start frame).
                if let ClipSource::Sequence(id) = clip.source {
                    let nested = project.sequence(id)?;
                    let (v, a) = first_slots(nested);
                    let slot = if audio { a } else { v }?;
                    return Some((comp_ids.get(&id)?.clone(), slot, frames(clip.source_in, r)));
                }
                let (m, src) = clip.media_at(clip.timeline_in)?;
                let mm = media_mobs.get(&m)?;
                let slot = if audio {
                    mm.sound_slot
                } else {
                    mm.picture_slot
                }?;
                Some((mm.master.clone(), slot, frames(src, r)))
            });
            let mut slot =
                timeline_slot(ti as u32 + 1, &name, r, sequence(kind, components, length));
            // Track number within its kind (V1 = 1, A1 = 1), as Avid numbers them.
            let number = if audio { an } else { vn } as u32;
            slot.props.push((0x4804, data(number.to_le_bytes())));
            slots.push(slot);
        }
        let total = frames(s.duration(), r).max(1);
        let fps = (r.0.as_f64().round() as u16).max(1);
        let timecode = obj(
            TIMECODE,
            vec![
                data_def(TIMECODE_DEF),
                (0x0202, data(total.to_le_bytes())),
                (0x1501, data(0i64.to_le_bytes())),
                (0x1502, data(fps.to_le_bytes())),
                (0x1503, data([0u8])),
            ],
        );
        slots.push(timeline_slot(s.tracks.len() as u32 + 1, "TC1", r, timecode));
        if !s.markers.is_empty() {
            let described = first_slots(s).0.or(first_slots(s).1).unwrap_or(1);
            let markers: Vec<Obj> = s
                .markers
                .iter()
                .map(|m| {
                    obj(
                        DESCRIPTIVE_MARKER,
                        vec![
                            data_def(DESCRIPTIVE),
                            (0x0202, data(frames(m.duration, r).max(1).to_le_bytes())),
                            (0x0601, data(frames(m.at, r).to_le_bytes())),
                            (0x0602, data(utf16z(&m.note))),
                            // The track it annotates (the first video track, as Avid does).
                            (0x6102, data(described.to_le_bytes())),
                        ],
                    )
                })
                .collect();
            slots.push(obj(
                EVENT_SLOT,
                vec![
                    (0x4801, data((s.tracks.len() as u32 + 2).to_le_bytes())),
                    (0x4802, data(utf16z("Markers"))),
                    strong("Segment", 0x4803, sequence(DESCRIPTIVE, markers, 0)),
                    (0x4804, data(1u32.to_le_bytes())),
                    (0x4901, data(rational(r.0))),
                ],
            ));
        }
        let id = &comp_ids[&s.id];
        let mut comp = mob(COMPOSITION_MOB, id, &s.name, slots, when);
        let usage = if n == 0 {
            USAGE_TOP_LEVEL
        } else {
            USAGE_LOWER_LEVEL
        };
        comp.props.push((0x4408, data(auid(usage))));
        mobs.push((id.clone(), comp));
    }

    let identification = obj(
        IDENTIFICATION,
        vec![
            (0x3c01, data(utf16z("debut"))),
            (0x3c02, data(utf16z("debut"))),
            (0x3c04, data(utf16z(env!("CARGO_PKG_VERSION")))),
            (0x3c05, data(auid(PRODUCT_ID))),
            (0x3c06, data(stamp(when))),
            (0x3c08, data(utf16z("debut"))),
            (0x3c09, data(seq.id.0.to_le_bytes())),
        ],
    );
    let content = obj(
        CONTENT_STORAGE,
        vec![(
            0x1901,
            Prop::Set {
                name: mangle("Mobs", 0x1901, 22),
                key_pid: 0x4401,
                key_size: 32,
                items: mobs,
            },
        )],
    );
    let header = obj(
        HEADER,
        vec![
            (0x3b01, data(0x4949u16.to_le_bytes())),
            (0x3b02, data(stamp(when))),
            strong("Content", 0x3b03, content),
            strong("Dictionary", 0x3b04, dictionary),
            (0x3b05, data([1u8, 2])),
            vector("IdentificationList", 0x3b06, vec![identification]),
            (0x3b07, data(1u32.to_le_bytes())),
            (0x3b09, data(auid(OP_PATTERN_EDIT))),
        ],
    );
    let root = obj(
        ROOT,
        vec![
            strong("MetaDictionary", 0x0001, metadict),
            strong("Header", 0x0002, header),
        ],
    );
    let mut weak = WeakTable::default();
    let mut entries = object::entries(&root, &mut weak);
    entries.push(cfb::Entry::Stream {
        name: "referenced properties".into(),
        data: weak.stream(),
    });
    cfb::write(root.class, entries)
}

/// A track as AAF components and its length. Dissolves overlap the
/// segments on both sides: the outgoing one runs on past the cut and the
/// incoming one starts before it, by the dissolve's two halves.
fn track_components(
    clips: &[Clip],
    audio: bool,
    rate: FrameRate,
    total: i64,
    target: impl Fn(&Clip) -> Option<(Key, u32, i64)>,
) -> (Vec<Obj>, i64) {
    let kind = if audio { SOUND } else { PICTURE };
    // (length, start frame, target) per segment; transitions between them.
    struct Seg {
        len: i64,
        start: i64,
        target: Option<(Key, u32)>,
    }
    let mut segs: Vec<Seg> = Vec::new();
    let mut transitions: Vec<(usize, i64, i64)> = Vec::new(); // (before segment, length, cut point)
    let mut cursor = 0i64;
    for clip in clips {
        let at = frames(clip.timeline_in, rate);
        if at > cursor {
            segs.push(Seg {
                len: at - cursor,
                start: 0,
                target: None,
            });
        }
        let len = frames(clip.timeline_out(), rate) - at;
        let t = target(clip);
        if let Some(tr) = clip.transition_in {
            let total = frames(tr.duration, rate);
            let before = frames(tr.half(), rate); // incoming material shown before the cut
            let start = t.as_ref().map_or(0, |x| x.2);
            if total > 0 && !segs.is_empty() && start >= before {
                transitions.push((segs.len(), total, before));
            }
        }
        segs.push(Seg {
            len,
            start: t.as_ref().map_or(0, |x| x.2),
            target: t.map(|(m, s, _)| (m, s)),
        });
        cursor = at + len;
    }
    // Every track runs the sequence's length.
    if total > cursor {
        segs.push(Seg {
            len: total - cursor,
            start: 0,
            target: None,
        });
    }
    for &(i, total, before) in &transitions {
        segs[i - 1].len += total - before;
        segs[i].len += before;
        segs[i].start -= before;
    }
    let length =
        segs.iter().map(|s| s.len).sum::<i64>() - transitions.iter().map(|t| t.1).sum::<i64>();
    let op = if audio {
        AUDIO_DISSOLVE
    } else {
        VIDEO_DISSOLVE
    };
    let mut out = Vec::new();
    for (i, s) in segs.iter().enumerate() {
        if let Some(&(_, total, before)) = transitions.iter().find(|t| t.0 == i) {
            let group = obj(
                OPERATION_GROUP,
                vec![
                    data_def(kind),
                    (0x0202, data(total.to_le_bytes())),
                    (
                        0x0b01,
                        Prop::Weak {
                            path: OPERATION_DEFS.to_vec(),
                            key_pid: DEF_ID_PID,
                            key: auid(op).to_vec(),
                        },
                    ),
                ],
            );
            out.push(obj(
                TRANSITION,
                vec![
                    data_def(kind),
                    (0x0202, data(total.to_le_bytes())),
                    strong("OperationGroup", 0x1801, group),
                    (0x1802, data(before.to_le_bytes())),
                ],
            ));
        }
        out.push(match &s.target {
            Some((m, slot)) => source_clip(kind, s.len, m, *slot, s.start),
            None => filler(kind, s.len),
        });
    }
    (out, length.max(0))
}

#[cfg(test)]
mod tests;
