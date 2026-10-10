//! The writer's output read back with a small compound-file reader. (Also
//! checked against pyaaf2, which reads every object and property; see
//! `docs/ARCHITECTURE.md`.)

use super::*;
use debut_core::IdGen;
use debut_project::media_ref::{MediaMetadata, MediaRef};
use debut_project::{Marker, Title, TitleStyle, Track, Transition, TransitionKind};

/// Minimal MS-CFB reader: path -> stream bytes, path -> class id.
struct Cfb {
    streams: HashMap<String, Vec<u8>>,
    classes: HashMap<String, [u8; 16]>,
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

impl Cfb {
    fn read(file: &[u8]) -> Cfb {
        assert_eq!(
            &file[..8],
            &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]
        );
        let sector = |i: u32| &file[512 + i as usize * 512..512 + (i as usize + 1) * 512];
        let n_fat = u32_at(file, 0x2C) as usize;
        let mut fat_ids: Vec<u32> = (0..n_fat.min(109))
            .map(|i| u32_at(file, 0x4C + i * 4))
            .collect();
        let mut difat = u32_at(file, 0x44);
        while difat != 0xFFFF_FFFE && fat_ids.len() < n_fat {
            let s = sector(difat);
            for i in 0..127 {
                if fat_ids.len() < n_fat {
                    fat_ids.push(u32_at(s, i * 4));
                }
            }
            difat = u32_at(s, 508);
        }
        let fat: Vec<u32> = fat_ids
            .iter()
            .flat_map(|&id| (0..128).map(move |i| (id, i)))
            .map(|(id, i)| u32_at(sector(id), i * 4))
            .collect();
        let chain = |mut s: u32| {
            let mut out = Vec::new();
            while s != 0xFFFF_FFFE {
                out.extend_from_slice(sector(s));
                s = fat[s as usize];
            }
            out
        };
        let dir = chain(u32_at(file, 0x30));
        let entry = |i: usize| &dir[i * 128..(i + 1) * 128];
        let mini_fat_bytes = chain(u32_at(file, 0x3C));
        let mini_fat: Vec<u32> = (0..mini_fat_bytes.len() / 4)
            .map(|i| u32_at(&mini_fat_bytes, i * 4))
            .collect();
        let mini = chain(u32_at(entry(0), 0x74));
        let mut cfb = Cfb {
            streams: HashMap::new(),
            classes: HashMap::new(),
        };
        // Walk the sibling trees.
        fn walk(
            cfb: &mut Cfb,
            at: u32,
            prefix: &str,
            dirs: &dyn Fn(usize) -> Vec<u8>,
            data: &dyn Fn(u32, u64) -> Vec<u8>,
        ) {
            if at == 0xFFFF_FFFF {
                return;
            }
            let e = dirs(at as usize);
            let len = u16::from_le_bytes([e[64], e[65]]) as usize / 2 - 1;
            let name: String = String::from_utf16(
                &(0..len)
                    .map(|i| u16::from_le_bytes([e[i * 2], e[i * 2 + 1]]))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            let path = format!("{prefix}/{name}");
            walk(cfb, u32_at(&e, 68), prefix, dirs, data);
            walk(cfb, u32_at(&e, 72), prefix, dirs, data);
            match e[66] {
                1 => {
                    cfb.classes
                        .insert(path.clone(), e[80..96].try_into().unwrap());
                    walk(cfb, u32_at(&e, 76), &path, dirs, data);
                }
                2 => {
                    let size = u64::from_le_bytes(e[120..128].try_into().unwrap());
                    cfb.streams.insert(path, data(u32_at(&e, 116), size));
                }
                _ => {}
            }
        }
        let dirs = |i: usize| entry(i).to_vec();
        let data = |start: u32, size: u64| -> Vec<u8> {
            let mut out = Vec::new();
            if size < 4096 {
                let mut s = start;
                while s != 0xFFFF_FFFE && (out.len() as u64) < size {
                    out.extend_from_slice(&mini[s as usize * 64..s as usize * 64 + 64]);
                    s = mini_fat[s as usize];
                }
            } else {
                out = chain(start);
            }
            out.truncate(size as usize);
            out
        };
        let root_class: [u8; 16] = entry(0)[80..96].try_into().unwrap();
        cfb.classes.insert(String::new(), root_class);
        walk(&mut cfb, u32_at(entry(0), 76), "", &dirs, &data);
        cfb
    }

    /// pid -> (stored form, bytes) of an object's properties stream.
    fn props(&self, path: &str) -> HashMap<u16, (u16, Vec<u8>)> {
        let s = &self.streams[&format!("{path}/properties")];
        assert_eq!((s[0], s[1]), (0x4c, 32));
        let n = u16::from_le_bytes([s[2], s[3]]) as usize;
        let mut at = 4 + n * 6;
        let mut out = HashMap::new();
        for i in 0..n {
            let h = |k: usize| u16::from_le_bytes([s[4 + i * 6 + k], s[5 + i * 6 + k]]);
            let (pid, form, len) = (h(0), h(2), h(4) as usize);
            out.insert(pid, (form, s[at..at + len].to_vec()));
            at += len;
        }
        out
    }
}

fn i64_of(b: &[u8]) -> i64 {
    i64::from_le_bytes(b.try_into().unwrap())
}

/// V1: A 0..4 s (source 10..14), B 4..8 with a 1 s dissolve, a title 9..10;
/// A1: A 0..4; a marker at 2 s; B also nested in a second sequence.
fn project() -> (Project, HashMap<MediaId, AafMedia>) {
    let mut ids = IdGen::new(5);
    let mut p = Project::new(ids.fresh(), "p");
    let mref = |ids: &mut IdGen, path: &str| MediaRef {
        id: ids.fresh(),
        path: path.into(),
        online: true,
        metadata: MediaMetadata {
            audio_channels: 2,
            ..Default::default()
        },
        proxies: vec![],
        keywords: vec![],
        rating: 0,
    };
    let a = mref(&mut ids, "/shoot/Interview A.mov");
    let b = mref(&mut ids, "/shoot/broll.mp4");
    let s = Rational::from_int;
    let mut seq = Sequence::new(ids.fresh(), "Cut 1", FrameRate::FPS_25, 1920, 1080);
    let mut v = Track::new(ids.fresh(), TrackKind::Video);
    v.clips.push(Clip::new(
        ids.fresh(),
        ClipSource::Media(a.id),
        s(0),
        s(4),
        s(10),
    ));
    let mut cb = Clip::new(ids.fresh(), ClipSource::Media(b.id), s(4), s(4), s(2));
    cb.transition_in = Some(Transition {
        kind: TransitionKind::Dissolve,
        duration: s(1),
    });
    v.clips.push(cb);
    v.clips.push(Clip::new(
        ids.fresh(),
        ClipSource::Title(Title {
            text: "Hello".into(),
            style: TitleStyle::default(),
        }),
        s(9),
        s(1),
        Rational::ZERO,
    ));
    let mut au = Track::new(ids.fresh(), TrackKind::Audio);
    au.clips.push(Clip::new(
        ids.fresh(),
        ClipSource::Media(a.id),
        s(0),
        s(4),
        s(10),
    ));
    seq.tracks.push(v);
    seq.tracks.push(au);
    seq.markers.push(Marker::new(ids.fresh(), s(2), "check"));
    let mut inner = Sequence::new(ids.fresh(), "Inner", FrameRate::FPS_25, 1920, 1080);
    let mut iv = Track::new(ids.fresh(), TrackKind::Video);
    iv.clips.push(Clip::new(
        ids.fresh(),
        ClipSource::Media(b.id),
        s(0),
        s(2),
        s(0),
    ));
    inner.tracks.push(iv);
    let mut v2 = Track::new(ids.fresh(), TrackKind::Video);
    v2.clips.push(Clip::new(
        ids.fresh(),
        ClipSource::Sequence(inner.id),
        s(1),
        s(2),
        s(0),
    ));
    seq.tracks.push(v2);
    let info = |audio| AafMedia {
        width: 1920,
        height: 1080,
        duration: s(30),
        has_video: true,
        has_audio: audio,
    };
    let media = HashMap::from([(a.id, info(true)), (b.id, info(false))]);
    p.media = vec![a, b];
    p.sequences.push(seq);
    p.sequences.push(inner);
    (p, media)
}

#[test]
fn writes_mobs_tracks_dissolves_and_markers() {
    let (p, media) = project();
    let bytes = aaf(&p.sequences[0], &p, &media, Stamp::default()).unwrap();
    if let Ok(dir) = std::env::var("DEBUT_AAF_OUT") {
        #[allow(clippy::disallowed_methods)]
        std::fs::write(format!("{dir}/test.aaf"), &bytes).unwrap();
    }
    let f = Cfb::read(&bytes);
    assert_eq!(f.classes[""], auid(ROOT));
    assert!(f.streams.contains_key("/referenced properties"));
    assert_eq!(f.classes["/Header-2"], auid(HEADER));
    // 2 media x (file + master) + 2 compositions.
    let index = &f.streams["/Header-2/Content-3b03/Mobs-1901 index"];
    assert_eq!(u32_at(index, 0), 6);
    let mob_paths: Vec<String> = (0..6)
        .map(|i| format!("/Header-2/Content-3b03/Mobs-1901{{{i:x}}}"))
        .collect();
    let comp = mob_paths
        .iter()
        .find(|m| {
            f.classes[*m] == auid(COMPOSITION_MOB) && f.props(m)[&0x4408].1 == auid(USAGE_TOP_LEVEL)
        })
        .expect("a top-level composition");
    assert_eq!(f.props(comp)[&0x4402].1, utf16z("Cut 1"));
    // V1: clip A, transition, clip B, filler, filler (title).
    let v1 = format!("{comp}/Slots-4403{{0}}/Segment-4803");
    let classes: Vec<[u8; 16]> = (0..5)
        .map(|i| f.classes[&format!("{v1}/Components-1001{{{i:x}}}")])
        .collect();
    assert_eq!(
        classes,
        [SOURCE_CLIP, TRANSITION, SOURCE_CLIP, FILLER, FILLER].map(auid)
    );
    let len = |i: usize| i64_of(&f.props(&format!("{v1}/Components-1001{{{i:x}}}"))[&0x0202].1);
    // B starts 12 frames (half the dissolve) before the cut and A runs the
    // other 13 past it; the track still lasts 10 s = 250 frames.
    assert_eq!(
        (len(0), len(1), len(2), len(3), len(4)),
        (113, 25, 112, 25, 25)
    );
    assert_eq!(i64_of(&f.props(&v1)[&0x0202].1), 250);
    let b = f.props(&format!("{v1}/Components-1001{{2}}"));
    assert_eq!(
        i64_of(&b[&0x1201].1),
        2 * 25 - 12,
        "B's source starts earlier by the overlap"
    );
    let cut = f.props(&format!("{v1}/Components-1001{{1}}"));
    assert_eq!(i64_of(&cut[&0x1802].1), 12);
    // A1 references the master's sound slot (2: picture is 1).
    let a1 = f.props(&format!(
        "{comp}/Slots-4403{{1}}/Segment-4803/Components-1001{{0}}"
    ));
    assert_eq!(
        u32::from_le_bytes(a1[&0x1102].1.clone().try_into().unwrap()),
        2
    );
    // Timecode and marker slots follow the three tracks.
    assert_eq!(
        f.classes[&format!("{comp}/Slots-4403{{3}}/Segment-4803")],
        auid(TIMECODE)
    );
    let marker = f.props(&format!(
        "{comp}/Slots-4403{{4}}/Segment-4803/Components-1001{{0}}"
    ));
    assert_eq!(i64_of(&marker[&0x0601].1), 50);
    assert_eq!(marker[&0x0602].1, utf16z("check"));
    // The nested sequence clip points at the lower-level composition.
    let nested = f.props(&format!(
        "{comp}/Slots-4403{{2}}/Segment-4803/Components-1001{{1}}"
    ));
    let inner_id = mob_id(p.sequences[1].id.0, 2);
    assert_eq!(nested[&0x1101].1, inner_id);
    // File mobs carry the path.
    let file = mob_paths
        .iter()
        .find(|m| f.classes[*m] == auid(SOURCE_MOB))
        .unwrap();
    let loc = f.props(&format!("{file}/EssenceDescription-4701/Locator-2f01{{0}}"));
    let url = String::from_utf16(
        &loc[&0x4001]
            .1
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&c| c != 0)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(url.starts_with("file:///shoot/"), "{url}");
}

#[test]
fn compound_file_trees_stay_valid_for_many_entries() {
    // Enough streams for several directory sectors, a big stream outside the
    // mini stream, and a perfect and an imperfect sibling tree.
    for n in [1usize, 3, 7, 40] {
        let children: Vec<cfb::Entry> = (0..n)
            .map(|i| cfb::Entry::Stream {
                name: format!("s{i}"),
                data: vec![i as u8; if i == 0 { 5000 } else { i * 10 }],
            })
            .collect();
        let bytes = cfb::write([7; 16], children).unwrap();
        let f = Cfb::read(&bytes);
        assert_eq!(f.streams.len(), n);
        assert_eq!(f.streams["/s0"].len(), 5000);
        if n > 3 {
            assert_eq!(f.streams["/s3"], vec![3u8; 30]);
        }
    }
}
