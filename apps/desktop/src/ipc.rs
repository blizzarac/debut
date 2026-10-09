//! IPC commands. All project mutation goes through the command history; the
//! player is rebuilt from the project's sequence after every edit.

use debut_audio::normalize_gain;
use debut_command::{Command, MarkerTarget, Target};
use debut_core::id::{BinId, CaptionId, MarkerId};
use debut_core::{ClipId, FrameRate, IdGen, MediaId, Rational, SequenceId, TrackId};
use debut_engine::{Player, Stats, Workspace};
use debut_export::{export, measure_loudness, ExportJob, ExportQueue, JobId, JobState, Preset};
use debut_platform::audio_out::AudioOut;

use debut_platform::Encoder;
use debut_platform::{Decoder, FileStore};
use debut_platform_native::audio_out::{CpalAudioOut, SilentAudioOut};
use debut_platform_native::codec::FfmpegDecoder;
use debut_platform_native::codec::{AudioEncodeSettings, EncodeSettings, FfmpegEncoder};
use debut_platform_native::file_store::NativeFileStore;
use debut_project::media_ref::{MediaMetadata, MediaRef};
use debut_project::{
    schema, AudioEffect, Bin, Caption, CaptionSettings, Clip, ClipSource, Effect, EqBand, EqKind,
    GradeFx, KeyFx, Marker, MaskFx, MaskShape, Param, Project, Sequence, Title, TitleStyle, Track,
    TrackKind, TrackMix, TransformFx, Transition, TransitionKind,
};
use debut_render::AnyBackend;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::State;

pub struct Session {
    workspace: Option<Workspace>,
    store: Arc<dyn FileStore>,
    /// Commands replayed from the journal when the current file was opened.
    recovered: usize,
    ids: IdGen,
    /// The sequence shown in the timeline (TL-07): a nested one while it is
    /// opened for editing, else the project's first.
    active: Option<SequenceId>,
    player: Option<Player>,
    audio_out: Option<Box<dyn AudioOut>>,
    backend: AnyBackend,
    /// Media probed so far: id -> (width, height, duration, has_audio).
    probed: std::collections::HashMap<MediaId, (u32, u32, Rational, bool)>,
    preview: PreviewQuality,
    /// Rolling cost of producing a frame (render + readback), for Auto.
    frame_cost: Option<std::time::Duration>,
    frames_since_change: u32,
    exports: Arc<Mutex<ExportQueue>>,
    export_worker: Arc<std::sync::atomic::AtomicBool>,
    /// Per-job preset, normalization target and media paths (shared with the worker).
    export_specs: std::collections::HashMap<JobId, ExportSpec>,
    export_specs_shared: Option<Arc<Mutex<std::collections::HashMap<JobId, ExportSpec>>>>,
}

type ExportSpec = (Preset, Option<f32>, Vec<(MediaId, String)>, Vec<Sequence>);

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            workspace: None,
            store: Arc::new(NativeFileStore::new("/")),
            recovered: 0,
            ids: IdGen::random(),
            active: None,
            player: None,
            audio_out: None,
            backend: AnyBackend::detect(),
            probed: Default::default(),
            preview: PreviewQuality::Auto,
            frame_cost: None,
            frames_since_change: 0,
            exports: Arc::new(Mutex::new(ExportQueue::default())),
            export_worker: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            export_specs: Default::default(),
            export_specs_shared: None,
        }
    }

    fn project(&self) -> Option<&Project> {
        self.workspace.as_ref().map(|w| &w.project)
    }

    fn project_mut(&mut self) -> Result<&mut Project, String> {
        self.workspace
            .as_mut()
            .map(|w| &mut w.project)
            .ok_or_else(|| "no project open".to_string())
    }

    fn workspace_mut(&mut self) -> Result<&mut Workspace, String> {
        self.workspace
            .as_mut()
            .ok_or_else(|| "no project open".to_string())
    }

    /// Where a new project lives until Save As: `~/debut-projects/<name>.debut`.
    fn default_path(name: &str) -> String {
        let base = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("debut-projects");
        let safe: String = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        base.join(format!("{safe}.debut"))
            .to_string_lossy()
            .into_owned()
    }

    /// Start a fresh workspace around `project` at `path`.
    pub fn start(&mut self, project: Project, path: String) -> Result<(), String> {
        self.workspace = Some(
            Workspace::create(Arc::clone(&self.store), path, project).map_err(|e| e.to_string())?,
        );
        self.recovered = 0;
        self.player = None;
        self.active = None;
        self.sync_player()
    }

    pub fn save_project(&mut self, path: Option<String>) -> Result<FileStatus, String> {
        let ws = self.workspace_mut()?;
        match path {
            Some(p) if p != ws.path => ws.save_as(p).map_err(|e| e.to_string())?,
            _ => ws.save().map_err(|e| e.to_string())?,
        }
        Ok(self.file_status())
    }

    pub fn open_project_file(&mut self, path: String) -> Result<FileStatus, String> {
        let (ws, opened) =
            Workspace::open(Arc::clone(&self.store), path).map_err(|e| e.to_string())?;
        self.workspace = Some(ws);
        self.recovered = opened.recovered;
        self.player = None;
        self.active = None;
        self.probed.clear();
        // Re-probe media so sequences and clip insertion keep working.
        let media: Vec<(MediaId, String)> = self
            .project()
            .map(|p| p.media.iter().map(|m| (m.id, m.path.clone())).collect())
            .unwrap_or_default();
        for (id, path) in media {
            if let Ok(dec) = FfmpegDecoder::open(&path) {
                if let Some(v) = dec.video_info() {
                    self.probed.insert(
                        id,
                        (v.width, v.height, v.duration, dec.audio_info().is_some()),
                    );
                }
            }
        }
        self.sync_player()?;
        Ok(self.file_status())
    }

    pub fn file_status(&self) -> FileStatus {
        FileStatus {
            path: self.workspace.as_ref().map(|w| w.path.clone()),
            dirty: self.workspace.as_ref().is_some_and(|w| w.is_dirty()),
            recovered: self.recovered,
        }
    }

    fn exec(&mut self, cmd: Command) -> Result<(), String> {
        self.workspace_mut()?
            .execute(cmd)
            .map_err(|e| e.to_string())?;
        self.sync_player()
    }

    /// The sequence being edited: the opened nested sequence when one is
    /// active (and still exists), else the project's first.
    fn first_sequence(&self) -> Result<&Sequence, String> {
        let p = self.project().ok_or("no project open")?;
        self.active
            .and_then(|id| p.sequence(id))
            .or_else(|| p.sequences.first())
            .ok_or_else(|| "no sequence".to_string())
    }

    /// Every sequence in the project, for the breadcrumb / switcher.
    pub fn sequences(&self) -> Result<Vec<SequenceListDto>, String> {
        let active = self.first_sequence()?.id;
        Ok(self
            .project()
            .map(|p| {
                p.sequences
                    .iter()
                    .map(|s| SequenceListDto {
                        id: id_str(s.id.0),
                        name: s.name.clone(),
                        duration: secs(s.duration()),
                        active: s.id == active,
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Show `id` in the timeline (a nested sequence, or back to the main one).
    /// The player is rebuilt for it; the transport starts at 0.
    pub fn open_sequence(&mut self, id: &str) -> Result<SequenceDto, String> {
        let id = SequenceId(parse_id(id)?);
        if self.project().and_then(|p| p.sequence(id)).is_none() {
            return Err("no such sequence".into());
        }
        self.active = Some(id);
        self.player = None;
        self.sync_player()?;
        Ok(sequence_dto(self.first_sequence()?))
    }

    /// Rebuild the player's view of the sequence after an edit, keeping transport
    /// state; create it on first use.
    fn sync_player(&mut self) -> Result<(), String> {
        let Some(seq) = self.first_sequence().ok().cloned() else {
            self.player = None;
            return Ok(());
        };
        let media: Vec<(MediaId, String)> = self
            .project()
            .map(|p| p.media.iter().map(|m| (m.id, m.path.clone())).collect())
            .unwrap_or_default();
        if self.player.is_none() {
            let (player, sink) = Player::new(seq.clone());
            let mut out: Box<dyn AudioOut> = match CpalAudioOut::default_device() {
                Ok(dev) => Box::new(dev),
                Err(_) => Box::new(SilentAudioOut::new()),
            };
            out.start(Box::new(sink)).map_err(|e| e.to_string())?;
            self.audio_out = Some(out);
            self.player = Some(player);
        }
        let player = self.player.as_mut().unwrap();
        for (id, path) in media {
            if player.frames.dimensions(id).is_some() {
                continue;
            }
            let video = FfmpegDecoder::open(&path).map_err(|e| e.to_string())?;
            let has_audio = video.audio_info().is_some();
            let audio = if has_audio {
                Some(
                    Box::new(FfmpegDecoder::open(&path).map_err(|e| e.to_string())?)
                        as Box<dyn Decoder>,
                )
            } else {
                None
            };
            player
                .add_media(id, Some(Box::new(video)), audio)
                .map_err(|e| e.to_string())?;
        }
        let all = self
            .project()
            .map(|p| p.sequences.clone())
            .unwrap_or_default();
        let player = self.player.as_mut().unwrap();
        player.set_sequences(&all);
        player.sequence = seq.clone();
        player.transport.set_end(seq.duration());
        Ok(())
    }
}

type Shared = Mutex<Session>;

fn lock<'a>(state: &'a State<'a, Shared>) -> std::sync::MutexGuard<'a, Session> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

fn id_str(v: u128) -> String {
    v.to_string()
}

fn parse_id(s: &str) -> Result<u128, String> {
    s.parse::<u128>().map_err(|_| format!("bad id {s}"))
}

fn secs(r: Rational) -> f64 {
    r.as_f64()
}

/// Seconds -> frame-aligned Rational.
fn frames_of(t: f64, fr: FrameRate) -> Rational {
    fr.frame_to_time((t * fr.0.as_f64()).round() as i64)
}

fn fr_of(seq: &Sequence) -> FrameRate {
    seq.frame_rate
}

/// Build the command that nests `[start, end)` of `seq`: a new sequence with the
/// same track layout holding copies of the material in range (re-based to 0),
/// and, on every track that had something there, a compound clip overwriting
/// the range. Clips straddling the range are split by the overwrite.
fn nest_command(
    seq: &Sequence,
    start: Rational,
    end: Rational,
    ids: &mut IdGen,
) -> debut_core::Result<Command> {
    if end <= start {
        return Err(debut_core::Error::InvalidArgument("nothing to nest".into()));
    }
    let mut nested = Sequence::new(
        ids.fresh(),
        format!("{} (nested)", seq.name),
        seq.frame_rate,
        seq.width,
        seq.height,
    );
    let mut cmds = Vec::new();
    let mut overwrites = Vec::new();
    for track in &seq.tracks {
        let mut inner = Track::new(ids.fresh(), track.kind);
        inner.mix = track.mix;
        inner.audio_effects = track.audio_effects.clone();
        for clip in &track.clips {
            if clip.timeline_out() <= start || clip.timeline_in >= end {
                continue;
            }
            let mut c = clip.clone();
            c.id = ids.fresh();
            c.transition_in = None;
            if c.timeline_in < start {
                c.trim_head_to(start);
            }
            if c.timeline_out() > end {
                c.trim_tail_to(end);
            }
            c.timeline_in -= start;
            inner.clips.push(c);
        }
        if !inner.clips.is_empty() {
            overwrites.push((
                track.id,
                Clip::new(
                    ids.fresh(),
                    ClipSource::Sequence(nested.id),
                    start,
                    end - start,
                    Rational::ZERO,
                ),
            ));
        }
        nested.tracks.push(inner);
    }
    if overwrites.is_empty() {
        return Err(debut_core::Error::InvalidArgument(
            "nothing to nest in that range".into(),
        ));
    }
    cmds.push(Command::AddSequence(nested));
    for (track, clip) in overwrites {
        let target = Target {
            sequence: seq.id,
            track,
        };
        cmds.push(Command::overwrite(target, clip, ids));
    }
    Ok(Command::Group(cmds))
}

/// Default length of a freshly added title.
const TITLE_SECONDS: i64 = 5;

// ---- project ------------------------------------------------------------------

#[tauri::command]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[derive(Serialize)]
pub struct FileStatus {
    pub path: Option<String>,
    pub dirty: bool,
    pub recovered: usize,
}

#[tauri::command]
pub fn new_project(state: State<'_, Shared>, name: String) -> Result<(), String> {
    let mut s = lock(&state);
    let id = s.ids.fresh();
    let path = Session::default_path(&name);
    s.start(Project::new(id, name), path)
}

#[tauri::command]
pub fn open_project(state: State<'_, Shared>, json: String) -> Result<(), String> {
    let project = schema::from_json(&json).map_err(|e| e.to_string())?;
    let mut s = lock(&state);
    let path = Session::default_path(&project.name);
    s.start(project, path)
}

#[tauri::command]
pub fn project_json(state: State<'_, Shared>) -> Result<String, String> {
    let s = lock(&state);
    let p = s.project().ok_or("no project open")?;
    schema::to_json(p).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_project(state: State<'_, Shared>, path: Option<String>) -> Result<FileStatus, String> {
    lock(&state).save_project(path)
}

#[tauri::command]
pub fn open_project_file(state: State<'_, Shared>, path: String) -> Result<FileStatus, String> {
    lock(&state).open_project_file(path)
}

#[tauri::command]
pub fn file_status(state: State<'_, Shared>) -> FileStatus {
    lock(&state).file_status()
}

#[tauri::command]
pub fn execute(state: State<'_, Shared>, command_json: String) -> Result<(), String> {
    let cmd: Command = serde_json::from_str(&command_json).map_err(|e| e.to_string())?;
    lock(&state).exec(cmd)
}

#[tauri::command]
pub fn undo(state: State<'_, Shared>) -> Result<bool, String> {
    let mut s = lock(&state);
    let r = s.workspace_mut()?.undo().map_err(|e| e.to_string())?;
    s.sync_player()?;
    Ok(r)
}

#[tauri::command]
pub fn redo(state: State<'_, Shared>) -> Result<bool, String> {
    let mut s = lock(&state);
    let r = s.workspace_mut()?.redo().map_err(|e| e.to_string())?;
    s.sync_player()?;
    Ok(r)
}

#[tauri::command]
pub fn can_undo(state: State<'_, Shared>) -> bool {
    lock(&state)
        .workspace
        .as_ref()
        .is_some_and(|w| w.history.can_undo())
}

// ---- media and sequence -------------------------------------------------------

#[derive(Serialize)]
pub struct MediaDto {
    pub id: String,
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub frame_rate: [i64; 2],
    pub has_audio: bool,
    /// Bins this media is in (manual membership and smart matches).
    #[serde(default)]
    pub bins: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub rating: u8,
}

#[derive(Serialize)]
pub struct BinDto {
    pub id: String,
    pub name: String,
    pub smart: bool,
    /// The smart bin's rule as "field op value", if it is a smart bin.
    pub filter: Option<String>,
    pub count: usize,
}

/// A smart-bin rule as the app sends it.
#[derive(Clone, Debug, Deserialize)]
pub struct RuleDto {
    pub field: String,
    pub op: String,
    pub value: String,
}

#[derive(Serialize)]
pub struct ClipDto {
    pub id: String,
    pub media: Option<String>,
    /// Text and style when this is a title clip (GFX-01).
    pub title: Option<Title>,
    /// Name of the nested sequence when this is a compound clip (TL-07).
    pub nested: Option<String>,
    /// Multicam clips: number of angles and the active one (MED-11, TL-08).
    pub angles: Option<usize>,
    pub angle: Option<usize>,
    pub timeline_in: f64,
    pub duration: f64,
    pub source_in: f64,
    pub transition_in: Option<f64>,
}

#[derive(Serialize)]
pub struct SequenceListDto {
    pub id: String,
    pub name: String,
    pub duration: f64,
    pub active: bool,
}

#[derive(Serialize)]
pub struct TrackDto {
    pub id: String,
    pub kind: String,
    pub clips: Vec<ClipDto>,
    pub mix: TrackMix,
    pub inserts: Vec<String>,
}

#[derive(Serialize)]
pub struct SequenceDto {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: [i64; 2],
    pub duration: f64,
    pub tracks: Vec<TrackDto>,
}

fn sequence_dto(seq: &Sequence) -> SequenceDto {
    SequenceDto {
        id: id_str(seq.id.0),
        name: seq.name.clone(),
        width: seq.width,
        height: seq.height,
        frame_rate: [seq.frame_rate.0.num, seq.frame_rate.0.den],
        duration: secs(seq.duration()),
        tracks: seq
            .tracks
            .iter()
            .map(|t| TrackDto {
                id: id_str(t.id.0),
                kind: format!("{:?}", t.kind).to_lowercase(),
                mix: t.mix,
                inserts: t.audio_effects.iter().map(insert_name).collect(),
                clips: t
                    .clips
                    .iter()
                    .map(|c| ClipDto {
                        id: id_str(c.id.0),
                        media: match &c.source {
                            ClipSource::Media(m) => Some(id_str(m.0)),
                            _ => None,
                        },
                        title: match &c.source {
                            ClipSource::Title(t) => Some(t.clone()),
                            _ => None,
                        },
                        nested: match &c.source {
                            ClipSource::Sequence(id) => Some(id_str(id.0)),
                            _ => None,
                        },
                        angles: match &c.source {
                            ClipSource::Multicam { angles, .. } => Some(angles.len()),
                            _ => None,
                        },
                        angle: match &c.source {
                            ClipSource::Multicam { active, .. } => Some(*active),
                            _ => None,
                        },
                        timeline_in: secs(c.timeline_in),
                        duration: secs(c.duration),
                        source_in: secs(c.source_in),
                        transition_in: c.transition_in.map(|t| secs(t.duration)),
                    })
                    .collect(),
            })
            .collect(),
    }
}

impl Session {
    pub fn import_media(&mut self, path: String) -> Result<MediaDto, String> {
        let dec = FfmpegDecoder::open(&path).map_err(|e| e.to_string())?;
        let v = dec.video_info().ok_or("file has no video stream")?.clone();
        let has_audio = dec.audio_info().is_some();
        self.project_mut()?;
        let id: MediaId = self.ids.fresh();
        let media = MediaRef {
            id,
            path: path.clone(),
            online: true,
            metadata: MediaMetadata {
                frame_rate: Some(v.frame_rate),
                audio_channels: dec.audio_info().map(|a| a.channels).unwrap_or(0),
                ..Default::default()
            },
            proxies: vec![],
            keywords: vec![],
            rating: 0,
        };
        self.probed
            .insert(id, (v.width, v.height, v.duration, has_audio));
        self.exec(Command::AddMedia(media))?;
        Ok(MediaDto {
            id: id_str(id.0),
            path,
            width: v.width,
            height: v.height,
            duration: secs(v.duration),
            frame_rate: [v.frame_rate.0.num, v.frame_rate.0.den],
            has_audio,
            bins: Vec::new(),
            keywords: Vec::new(),
            rating: 0,
        })
    }

    /// Every media in the project with its bins; probes files not seen yet in
    /// this session (after opening a project) and caches the result.
    pub fn media_list(&mut self) -> Result<Vec<MediaDto>, String> {
        let media: Vec<MediaRef> = self.project().map(|p| p.media.clone()).unwrap_or_default();
        for m in &media {
            if self.probed.contains_key(&m.id) {
                continue;
            }
            if let Ok(dec) = FfmpegDecoder::open(&m.path) {
                if let Some(v) = dec.video_info() {
                    self.probed.insert(
                        m.id,
                        (v.width, v.height, v.duration, dec.audio_info().is_some()),
                    );
                }
            }
        }
        let bins = self.project().map(|p| p.bins.clone()).unwrap_or_default();
        Ok(media
            .iter()
            .map(|m| {
                let (w, h, d, a) =
                    self.probed
                        .get(&m.id)
                        .copied()
                        .unwrap_or((0, 0, Rational::ZERO, false));
                let fr = m.metadata.frame_rate.unwrap_or(FrameRate::FPS_25);
                MediaDto {
                    id: id_str(m.id.0),
                    path: m.path.clone(),
                    width: w,
                    height: h,
                    duration: secs(d),
                    frame_rate: [fr.0.num, fr.0.den],
                    has_audio: a,
                    bins: bins
                        .iter()
                        .filter(|b| b.contains(m))
                        .map(|b| id_str(b.id.0))
                        .collect(),
                    keywords: m.keywords.clone(),
                    rating: m.rating,
                }
            })
            .collect())
    }

    pub fn bins(&self) -> Result<Vec<BinDto>, String> {
        let p = self.project().ok_or("no project open")?;
        Ok(p.bins
            .iter()
            .map(|b| BinDto {
                id: id_str(b.id.0),
                name: b.name.clone(),
                smart: b.is_smart(),
                filter: match &b.kind {
                    debut_project::BinKind::Smart { rules } => Some(
                        rules
                            .iter()
                            .map(|r| format!("{} {} {}", r.field, r.op, r.value))
                            .collect::<Vec<_>>()
                            .join(" and "),
                    ),
                    _ => None,
                },
                count: p.media.iter().filter(|m| b.contains(m)).count(),
            })
            .collect())
    }

    /// New bin; with `filter` it is a smart bin matching file names containing it.
    pub fn add_bin(&mut self, name: String, filter: Option<String>) -> Result<String, String> {
        match filter.filter(|f| !f.trim().is_empty()) {
            Some(f) => self.add_smart_bin(
                name,
                RuleDto {
                    field: "name".into(),
                    op: "contains".into(),
                    value: f.trim().to_string(),
                },
            ),
            None => {
                let id: BinId = self.ids.fresh();
                self.exec(Command::AddBin(Bin::manual(id, name)))?;
                Ok(id_str(id.0))
            }
        }
    }

    /// New smart bin with one rule (name/path/keyword contains, rating gte, ...).
    pub fn add_smart_bin(&mut self, name: String, rule: RuleDto) -> Result<String, String> {
        const FIELDS: &[&str] = &[
            "name", "path", "reel", "camera", "audio", "keyword", "rating",
        ];
        if !FIELDS.contains(&rule.field.as_str()) {
            return Err(format!("unknown rule field {}", rule.field));
        }
        let id: BinId = self.ids.fresh();
        let bin = Bin {
            id,
            name,
            kind: debut_project::BinKind::Smart {
                rules: vec![debut_project::SmartRule {
                    field: rule.field,
                    op: rule.op,
                    value: rule.value,
                }],
            },
            items: Vec::new(),
        };
        self.exec(Command::AddBin(bin))?;
        Ok(id_str(id.0))
    }

    /// Keywords (comma separated or a list) and a 0..=5 rating for a media.
    pub fn set_media_tags(
        &mut self,
        media: &str,
        keywords: Vec<String>,
        rating: u8,
    ) -> Result<(), String> {
        self.exec(Command::SetMediaTags {
            media: MediaId(parse_id(media)?),
            keywords,
            rating,
        })
    }

    pub fn rename_bin(&mut self, id: &str, name: String) -> Result<(), String> {
        self.exec(Command::RenameBin {
            id: BinId(parse_id(id)?),
            name,
        })
    }

    pub fn remove_bin(&mut self, id: &str) -> Result<(), String> {
        self.exec(Command::RemoveBin(BinId(parse_id(id)?)))
    }

    /// Put media in a manual bin (or in none).
    pub fn assign_media(&mut self, media: &str, bin: Option<String>) -> Result<(), String> {
        let bin = match bin {
            Some(b) => Some(BinId(parse_id(&b)?)),
            None => None,
        };
        self.exec(Command::AssignMedia {
            media: MediaId(parse_id(media)?),
            bin,
        })
    }

    /// Create the first sequence (sized from the first media, else 1080p25) with
    /// one video and one audio track, if the project has none.
    pub fn ensure_sequence(&mut self) -> Result<SequenceDto, String> {
        if self.first_sequence().is_err() {
            let (w, h, fr) = self
                .project()
                .and_then(|p| p.media.first())
                .and_then(|m| {
                    self.probed.get(&m.id).map(|&(w, h, _, _)| {
                        (w, h, m.metadata.frame_rate.unwrap_or(FrameRate::FPS_25))
                    })
                })
                .unwrap_or((1920, 1080, FrameRate::FPS_25));
            let seq = Sequence::new(self.ids.fresh(), "Sequence 1", fr, w, h);
            let seq_id = seq.id;
            let v = Track::new(self.ids.fresh(), TrackKind::Video);
            let a = Track::new(self.ids.fresh(), TrackKind::Audio);
            self.exec(Command::Group(vec![
                Command::AddSequence(seq),
                Command::AddTrack {
                    sequence: seq_id,
                    track: v,
                    index: None,
                },
                Command::AddTrack {
                    sequence: seq_id,
                    track: a,
                    index: None,
                },
            ]))?;
        }
        Ok(sequence_dto(self.first_sequence()?))
    }

    /// Insert the whole of `media` on `track` at `at` seconds (ripple).
    pub fn add_clip(&mut self, track: &str, media: &str, at: f64) -> Result<(), String> {
        let (seq_id, fr) = {
            let seq = self.first_sequence()?;
            (seq.id, seq.frame_rate)
        };
        let track_id = TrackId(parse_id(track)?);
        let media_id = MediaId(parse_id(media)?);
        let duration = self
            .probed
            .get(&media_id)
            .map(|p| p.2)
            .ok_or("unknown media")?;
        let clip = Clip::new(
            self.ids.fresh(),
            ClipSource::Media(media_id),
            Rational::ZERO,
            fr.snap(duration).max(fr.frame_duration()),
            Rational::ZERO,
        );
        let target = Target {
            sequence: seq_id,
            track: track_id,
        };
        let cmd = Command::insert(target, frames_of(at, fr), vec![clip], &mut self.ids);
        self.exec(cmd)
    }

    /// Insert a multicam clip of `media` (two or more angles, angle 0 active) on
    /// the first video track and, when every angle has audio, on the first
    /// audio track, at `at` seconds. Its length is the shortest angle (MED-11).
    pub fn add_multicam(&mut self, at: f64, media: Vec<String>) -> Result<(), String> {
        if media.len() < 2 {
            return Err("a multicam clip needs at least two angles".into());
        }
        let angles = media
            .iter()
            .map(|m| Ok(MediaId(parse_id(m)?)))
            .collect::<Result<Vec<_>, String>>()?;
        let (seq_id, fr, video, audio) = {
            let seq = self.first_sequence()?;
            let first = |kind| seq.tracks.iter().find(|t| t.kind == kind).map(|t| t.id);
            (
                seq.id,
                seq.frame_rate,
                first(TrackKind::Video).ok_or("no video track")?,
                first(TrackKind::Audio),
            )
        };
        let mut duration = Rational::from_int(i64::MAX / 4);
        let mut all_audio = true;
        for m in &angles {
            let p = self.probed.get(m).ok_or("unknown media")?;
            duration = duration.min(p.2);
            all_audio &= p.3;
        }
        let duration = fr.snap(duration).max(fr.frame_duration());
        let source = ClipSource::Multicam { angles, active: 0 };
        let mut cmds = Vec::new();
        let mut targets = vec![video];
        if let (Some(a), true) = (audio, all_audio) {
            targets.push(a);
        }
        for track in targets {
            let clip = Clip::new(
                self.ids.fresh(),
                source.clone(),
                Rational::ZERO,
                duration,
                Rational::ZERO,
            );
            let target = Target {
                sequence: seq_id,
                track,
            };
            cmds.push(Command::insert(
                target,
                frames_of(at, fr),
                vec![clip],
                &mut self.ids,
            ));
        }
        self.exec(Command::Group(cmds))
    }

    /// Switch a multicam clip to `angle`. With `cut` and the playhead strictly
    /// inside the clip, blade there first so only the part after the playhead
    /// switches (the live-switch gesture, TL-08). Returns the clip that switched.
    pub fn switch_angle(
        &mut self,
        track: &str,
        clip: &str,
        angle: usize,
        cut: bool,
    ) -> Result<String, String> {
        let (target, clip_id, c) = self.clip_ref(track, clip)?;
        let ClipSource::Multicam { angles, .. } = &c.source else {
            return Err("not a multicam clip".into());
        };
        if angle >= angles.len() {
            return Err(format!("no angle {angle}"));
        }
        let source = ClipSource::Multicam {
            angles: angles.clone(),
            active: angle,
        };
        let at = self.playhead();
        let mut cmds = Vec::new();
        let mut switched = clip_id;
        if cut && c.timeline_in < at && at < c.timeline_out() {
            let tail_id: ClipId = self.ids.fresh();
            cmds.push(Command::Blade {
                target,
                at,
                tail_id,
            });
            switched = tail_id;
        }
        cmds.push(Command::SetClipSource {
            target,
            clip: switched,
            source,
        });
        self.exec(Command::Group(cmds))?;
        Ok(id_str(switched.0))
    }

    /// Add a 5 s title clip at `at` seconds on the topmost video track with room
    /// for it, adding a video track above the others when none has (GFX-01).
    pub fn add_title(&mut self, at: f64, text: String) -> Result<String, String> {
        self.add_title_from(at, text, None)
    }

    /// Title templates (GFX-02) the app can offer.
    pub fn title_templates(&self) -> Vec<TemplateDto> {
        debut_graphics::TEMPLATES
            .iter()
            .map(|t| TemplateDto {
                id: t.id.to_string(),
                name: t.name.to_string(),
                description: t.description.to_string(),
            })
            .collect()
    }

    /// `add_title` with a template: style and animated Transform sized for
    /// this sequence. `None` is the plain default title.
    pub fn add_title_from(
        &mut self,
        at: f64,
        text: String,
        template: Option<String>,
    ) -> Result<String, String> {
        let (seq_id, fr, free_track, above_video, size) = {
            let seq = self.first_sequence()?;
            let start = frames_of(at, fr_of(seq));
            let end = start + Rational::from_int(TITLE_SECONDS);
            let free = seq
                .tracks
                .iter()
                .rev()
                .find(|t| {
                    t.kind == TrackKind::Video
                        && t.clips
                            .iter()
                            .all(|c| c.timeline_out() <= start || c.timeline_in >= end)
                })
                .map(|t| t.id);
            // A new title track goes right above the top video track: later tracks
            // composite on top, and it stays grouped with the video tracks.
            let above_video = seq
                .tracks
                .iter()
                .rposition(|t| t.kind == TrackKind::Video)
                .map_or(seq.tracks.len(), |i| i + 1);
            (
                seq.id,
                seq.frame_rate,
                free,
                above_video,
                (seq.width, seq.height),
            )
        };
        let start = frames_of(at, fr);
        let duration = Rational::from_int(TITLE_SECONDS);
        let (title, effects) = match template.as_deref() {
            None => (
                Title {
                    text,
                    style: TitleStyle::default(),
                },
                Vec::new(),
            ),
            Some(id) => {
                let built =
                    debut_graphics::build_title_template(id, &text, size.0, size.1, duration)
                        .ok_or_else(|| format!("unknown title template {id}"))?;
                (built.title, built.effects)
            }
        };
        let mut clip = Clip::new(
            self.ids.fresh(),
            ClipSource::Title(title),
            start,
            duration,
            Rational::ZERO,
        );
        clip.effects = effects;
        let clip_id = clip.id;
        let mut cmds = Vec::new();
        let track_id = match free_track {
            Some(id) => id,
            None => {
                let track = Track::new(self.ids.fresh(), TrackKind::Video);
                let id = track.id;
                cmds.push(Command::AddTrack {
                    sequence: seq_id,
                    track,
                    index: Some(above_video),
                });
                id
            }
        };
        let target = Target {
            sequence: seq_id,
            track: track_id,
        };
        cmds.push(Command::overwrite(target, clip, &mut self.ids));
        self.exec(Command::Group(cmds))?;
        Ok(id_str(clip_id.0))
    }

    /// Replace a title clip's text and style (GFX-01, GFX-02).
    pub fn set_title(&mut self, track: &str, clip: &str, title: Title) -> Result<(), String> {
        let seq_id = self.first_sequence()?.id;
        let target = Target {
            sequence: seq_id,
            track: TrackId(parse_id(track)?),
        };
        let clip = ClipId(parse_id(clip)?);
        let is_title = self
            .project()
            .and_then(|p| p.sequence(seq_id))
            .and_then(|s| s.track(target.track))
            .and_then(|t| t.clip(clip))
            .map(|c| matches!(c.source, ClipSource::Title(_)))
            .ok_or("no such clip")?;
        if !is_title {
            return Err("not a title clip".into());
        }
        self.exec(Command::SetClipSource {
            target,
            clip,
            source: ClipSource::Title(title),
        })
    }

    pub fn edit(&mut self, op: EditOp) -> Result<(), String> {
        let (seq_id, fr) = {
            let seq = self.first_sequence()?;
            (seq.id, seq.frame_rate)
        };
        let target = |t: &str| -> Result<Target, String> {
            Ok(Target {
                sequence: seq_id,
                track: TrackId(parse_id(t)?),
            })
        };
        let clip = |c: &str| -> Result<ClipId, String> { Ok(ClipId(parse_id(c)?)) };
        let Session { workspace, ids, .. } = self;
        let project = &workspace.as_ref().ok_or("no project open")?.project;
        let cmd = match op {
            EditOp::RippleHead {
                track,
                clip: c,
                delta,
            } => debut_timeline::ripple_head(
                project,
                target(&track)?,
                clip(&c)?,
                frames_of(delta, fr),
            ),
            EditOp::RippleTail {
                track,
                clip: c,
                delta,
            } => debut_timeline::ripple_tail(
                project,
                target(&track)?,
                clip(&c)?,
                frames_of(delta, fr),
            ),
            EditOp::Roll {
                track,
                clip: c,
                delta,
            } => debut_timeline::roll(project, target(&track)?, clip(&c)?, frames_of(delta, fr)),
            EditOp::Slip {
                track,
                clip: c,
                delta,
            } => Ok(debut_timeline::slip(
                target(&track)?,
                clip(&c)?,
                frames_of(delta, fr),
            )),
            EditOp::Slide {
                track,
                clip: c,
                delta,
            } => debut_timeline::slide(project, target(&track)?, clip(&c)?, frames_of(delta, fr)),
            EditOp::Move {
                track,
                clip: c,
                delta,
            } => Ok(Command::Move {
                target: target(&track)?,
                clip: clip(&c)?,
                delta: frames_of(delta, fr),
            }),
            EditOp::Blade { track, at } => Ok(Command::Blade {
                target: target(&track)?,
                at: frames_of(at, fr),
                tail_id: ids.fresh(),
            }),
            EditOp::Extract { track, start, end } => Ok(Command::extract(
                target(&track)?,
                frames_of(start, fr),
                frames_of(end, fr),
                ids,
            )),
            EditOp::Lift { track, start, end } => Ok(Command::lift(
                target(&track)?,
                frames_of(start, fr),
                frames_of(end, fr),
                ids,
            )),
            EditOp::Transition {
                track,
                clip: c,
                duration,
            } => Ok(Command::SetTransition {
                target: target(&track)?,
                clip: clip(&c)?,
                transition: duration.map(|d| Transition {
                    kind: TransitionKind::Dissolve,
                    duration: frames_of(d, fr),
                }),
            }),
            EditOp::Nest { start, end } => project
                .sequence(seq_id)
                .ok_or_else(|| debut_core::Error::NotFound("sequence".into()))
                .and_then(|seq| nest_command(seq, frames_of(start, fr), frames_of(end, fr), ids)),
        }
        .map_err(|e| e.to_string())?;
        self.exec(cmd)
    }

    pub fn transport(&mut self, action: TransportAction) -> Result<(), String> {
        if self.player.is_none() {
            self.sync_player()?;
        }
        let fr = self.first_sequence()?.frame_rate;
        let p = self.player.as_mut().ok_or("no sequence")?;
        match action {
            TransportAction::Play => p.play(),
            TransportAction::Pause => p.pause(),
            TransportAction::Toggle => {
                if p.transport.is_playing() {
                    p.pause()
                } else {
                    p.play()
                }
            }
            TransportAction::Seek { t } => p.seek(frames_of(t, fr)),
            TransportAction::Step { n } => p.step(n),
            TransportAction::Shuttle { forward } => p.shuttle(forward),
        }
        Ok(())
    }

    pub fn tick(&mut self) -> Result<TickDto, String> {
        if let Some(ws) = self.workspace.as_mut() {
            ws.maybe_autosave(Instant::now())
                .map_err(|e| e.to_string())?;
        }
        if self.player.is_none() {
            self.sync_player()?;
        }
        self.apply_preview_quality();
        let p = self.player.as_mut().ok_or("no sequence")?;
        let changed = p.tick().map_err(|e| e.to_string())?.is_some();
        let Stats { dropped, .. } = p.stats();
        Ok(TickDto {
            frame: p.transport.current_frame(),
            position: secs(p.transport.position()),
            playing: p.transport.is_playing(),
            changed,
            dropped,
            preview_divisor: p.preview_divisor(),
        })
    }

    pub fn set_preview_quality(&mut self, q: PreviewQuality) {
        self.preview = q;
        self.frames_since_change = 0;
        self.apply_preview_quality();
    }

    /// Fixed modes set the divisor directly. Auto steps down when a frame costs
    /// more than ~70 % of its display time and back up after a run of cheap ones.
    fn apply_preview_quality(&mut self) {
        let Some(p) = self.player.as_mut() else {
            return;
        };
        let target = match self.preview {
            PreviewQuality::Full => 1,
            PreviewQuality::Half => 2,
            PreviewQuality::Quarter => 4,
            PreviewQuality::Auto => {
                let d = p.preview_divisor();
                let budget = p.transport.frame_rate().frame_duration().as_f64();
                match self.frame_cost.map(|c| c.as_secs_f64()) {
                    Some(cost) if cost > budget * 0.7 && d < 4 => {
                        self.frames_since_change = 0;
                        d * 2
                    }
                    // Stepping up costs ~4x per step; only when there's clear headroom.
                    Some(cost)
                        if cost < budget * 0.12 && d > 1 && self.frames_since_change > 50 =>
                    {
                        self.frames_since_change = 0;
                        d / 2
                    }
                    _ => d,
                }
            }
        };
        p.set_preview_divisor(target);
    }

    /// The current frame as `(width, height, RGBA8 bytes)` at the preview size.
    pub fn frame_pixels(&mut self) -> Result<(u32, u32, Vec<u8>), String> {
        let started = Instant::now();
        let Session {
            player, backend, ..
        } = self;
        let p = player.as_mut().ok_or("no sequence")?;
        let graph = p.current_graph();
        let (w, h, out) = backend
            .render_rgba8(&graph, &mut p.frames, debut_render::Transfer::Srgb)
            .map_err(|e| e.to_string())?;
        let cost = started.elapsed();
        // Exponential moving average so one slow frame doesn't flip the mode.
        self.frame_cost = Some(match self.frame_cost {
            Some(prev) => prev.mul_f32(0.7) + cost.mul_f32(0.3),
            None => cost,
        });
        self.frames_since_change = self.frames_since_change.saturating_add(1);
        Ok((w, h, out))
    }

    /// Scopes of the current frame (PB-08), packed as `Scopes::to_bytes`.
    pub fn scopes(&mut self) -> Result<Vec<u8>, String> {
        let (w, h, px) = self.frame_pixels()?;
        Ok(debut_render::scopes::compute(&px, w, h).to_bytes())
    }
}

#[derive(Serialize)]
pub struct ParamDto {
    pub name: Param,
    /// Value at the playhead (clip-local evaluation).
    pub value: f64,
    /// More than one keyframe, or a single non-held key.
    pub animated: bool,
}

#[derive(Serialize)]
pub struct EffectDto {
    pub index: usize,
    pub kind: String,
    pub params: Vec<ParamDto>,
    /// Non-animated options (mask shape/invert, key colour).
    pub options: EffectOptions,
}

/// The non-keyframed knobs of an effect; every field optional so one struct
/// serves both reading and partial updates.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct EffectOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<MaskShape>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invert: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
}

fn effect_options(e: &Effect) -> EffectOptions {
    match e {
        Effect::Mask(m) => EffectOptions {
            shape: Some(m.shape),
            invert: Some(m.invert),
            color: None,
        },
        Effect::ChromaKey(k) => EffectOptions {
            color: Some(k.color),
            ..Default::default()
        },
        _ => EffectOptions::default(),
    }
}

impl Session {
    fn clip_ref(&self, track: &str, clip: &str) -> Result<(Target, ClipId, Clip), String> {
        let seq = self.first_sequence()?;
        let track_id = TrackId(parse_id(track)?);
        let clip_id = ClipId(parse_id(clip)?);
        let c = seq
            .track(track_id)
            .and_then(|t| t.clip(clip_id))
            .ok_or("clip not found")?
            .clone();
        Ok((
            Target {
                sequence: seq.id,
                track: track_id,
            },
            clip_id,
            c,
        ))
    }

    fn playhead(&self) -> Rational {
        self.player
            .as_ref()
            .map(|p| p.transport.position())
            .unwrap_or(Rational::ZERO)
    }

    /// The clip's effects with each parameter evaluated at the playhead.
    pub fn clip_effects(&self, track: &str, clip: &str) -> Result<Vec<EffectDto>, String> {
        let (_, _, c) = self.clip_ref(track, clip)?;
        let local = self.playhead() - c.timeline_in;
        Ok(c.effects
            .iter()
            .enumerate()
            .map(|(index, e)| EffectDto {
                index,
                kind: e.kind().to_string(),
                options: effect_options(e),
                params: e
                    .params()
                    .iter()
                    .map(|&p| {
                        let curve = e.curve(p).unwrap();
                        ParamDto {
                            name: p,
                            value: curve.eval(local),
                            animated: curve.keys().len() > 1,
                        }
                    })
                    .collect(),
            })
            .collect())
    }

    pub fn add_effect(&mut self, track: &str, clip: &str, kind: &str) -> Result<(), String> {
        let (target, clip_id, _) = self.clip_ref(track, clip)?;
        let effect = match kind {
            "transform" => Effect::Transform(TransformFx::default()),
            "grade" => Effect::Grade(GradeFx::default()),
            "mask" => Effect::Mask(MaskFx::default()),
            "key" => Effect::ChromaKey(KeyFx::default()),
            other => return Err(format!("unknown effect kind {other}")),
        };
        self.exec(Command::AddEffect {
            target,
            clip: clip_id,
            effect,
            index: None,
        })
    }

    /// Change an effect's non-animated options (FX-04 shape/invert, FX-05 key
    /// colour); fields left `None` keep their value.
    pub fn set_effect_options(
        &mut self,
        track: &str,
        clip: &str,
        index: usize,
        opts: EffectOptions,
    ) -> Result<(), String> {
        let (target, clip_id, c) = self.clip_ref(track, clip)?;
        let mut effect = c.effects.get(index).cloned().ok_or("no such effect")?;
        match &mut effect {
            Effect::Mask(m) => {
                if let Some(shape) = opts.shape {
                    m.shape = shape;
                }
                if let Some(invert) = opts.invert {
                    m.invert = invert;
                }
            }
            Effect::ChromaKey(k) => {
                if let Some(color) = opts.color {
                    k.color = color;
                }
            }
            _ => return Err("this effect has no options".into()),
        }
        self.exec(Command::ReplaceEffect {
            target,
            clip: clip_id,
            index,
            effect,
        })
    }

    pub fn remove_effect(&mut self, track: &str, clip: &str, index: usize) -> Result<(), String> {
        let (target, clip_id, _) = self.clip_ref(track, clip)?;
        self.exec(Command::RemoveEffect {
            target,
            clip: clip_id,
            index,
        })
    }

    /// Set a parameter as a constant, or keyframe it at the playhead.
    pub fn set_param(
        &mut self,
        track: &str,
        clip: &str,
        effect: usize,
        param: Param,
        value: f64,
        keyframe: bool,
    ) -> Result<(), String> {
        let (target, clip_id, c) = self.clip_ref(track, clip)?;
        let at = if keyframe {
            let fr = self.first_sequence()?.frame_rate;
            Some(fr.snap(self.playhead()) - c.timeline_in)
        } else {
            None
        };
        self.exec(Command::SetParam {
            target,
            clip: clip_id,
            effect,
            param,
            at,
            value,
        })
    }
}

#[tauri::command]
pub fn clip_effects(
    state: State<'_, Shared>,
    track: String,
    clip: String,
) -> Result<Vec<EffectDto>, String> {
    lock(&state).clip_effects(&track, &clip)
}

#[tauri::command]
pub fn add_effect(
    state: State<'_, Shared>,
    track: String,
    clip: String,
    kind: String,
) -> Result<(), String> {
    lock(&state).add_effect(&track, &clip, &kind)
}

#[tauri::command]
pub fn set_effect_options(
    state: State<'_, Shared>,
    track: String,
    clip: String,
    index: usize,
    options: EffectOptions,
) -> Result<(), String> {
    lock(&state).set_effect_options(&track, &clip, index, options)
}

#[tauri::command]
pub fn remove_effect(
    state: State<'_, Shared>,
    track: String,
    clip: String,
    index: usize,
) -> Result<(), String> {
    lock(&state).remove_effect(&track, &clip, index)
}

#[tauri::command]
pub fn set_param(
    state: State<'_, Shared>,
    track: String,
    clip: String,
    effect: usize,
    param: Param,
    value: f64,
    keyframe: bool,
) -> Result<(), String> {
    lock(&state).set_param(&track, &clip, effect, param, value, keyframe)
}

fn insert_name(e: &AudioEffect) -> String {
    match e {
        AudioEffect::Eq { bands } if bands.iter().all(|b| matches!(b.kind, EqKind::HighPass)) => {
            "Low cut".into()
        }
        AudioEffect::Eq { .. } => "EQ".into(),
        AudioEffect::Compressor { .. } => "Compressor".into(),
        AudioEffect::Limiter { .. } => "Limiter".into(),
        AudioEffect::Gate { .. } => "Gate".into(),
        AudioEffect::DeEsser { .. } => "De-esser".into(),
        AudioEffect::Reverb { .. } => "Reverb".into(),
    }
}

fn insert_preset(kind: &str) -> Option<AudioEffect> {
    Some(match kind {
        "eq_lowcut" => AudioEffect::Eq {
            bands: vec![EqBand {
                kind: EqKind::HighPass,
                frequency_hz: 80.0,
                gain_db: 0.0,
                q: 0.707,
            }],
        },
        "eq_presence" => AudioEffect::Eq {
            bands: vec![EqBand {
                kind: EqKind::Peak,
                frequency_hz: 3000.0,
                gain_db: 3.0,
                q: 1.0,
            }],
        },
        "compressor" => AudioEffect::Compressor {
            threshold_db: -18.0,
            ratio: 3.0,
            attack_ms: 10.0,
            release_ms: 100.0,
            makeup_db: 3.0,
        },
        "limiter" => AudioEffect::Limiter {
            ceiling_db: -1.0,
            release_ms: 50.0,
        },
        "gate" => AudioEffect::Gate {
            threshold_db: -45.0,
            attack_ms: 1.0,
            release_ms: 50.0,
        },
        "de_esser" => AudioEffect::DeEsser {
            frequency_hz: 6000.0,
            threshold_db: -24.0,
            ratio: 6.0,
        },
        "reverb" => AudioEffect::Reverb {
            room: 0.6,
            damping: 0.4,
            mix: 0.25,
        },
        _ => return None,
    })
}

#[derive(Serialize)]
pub struct PresetDto {
    pub name: String,
    pub loudness_lufs: f32,
}

#[derive(Serialize)]
pub struct ExportStatusDto {
    pub id: u64,
    pub name: String,
    pub output: String,
    pub state: String,
    pub frames_done: u64,
    pub frames_total: u64,
    pub loudness_lufs: Option<f32>,
    pub true_peak_db: f32,
    pub error: Option<String>,
}

impl Session {
    fn track_target(&self, track: &str) -> Result<Target, String> {
        let seq = self.first_sequence()?;
        let track_id = TrackId(parse_id(track)?);
        seq.track(track_id).ok_or("track not found")?;
        Ok(Target {
            sequence: seq.id,
            track: track_id,
        })
    }

    pub fn set_track_mix(&mut self, track: &str, mix: TrackMix) -> Result<(), String> {
        let target = self.track_target(track)?;
        self.exec(Command::SetTrackMix { target, mix })
    }

    fn track_inserts(&self, track: &str) -> Result<(Target, Vec<AudioEffect>), String> {
        let target = self.track_target(track)?;
        let effects = self
            .first_sequence()?
            .track(target.track)
            .unwrap()
            .audio_effects
            .clone();
        Ok((target, effects))
    }

    pub fn add_insert(&mut self, track: &str, kind: &str) -> Result<(), String> {
        let (target, mut effects) = self.track_inserts(track)?;
        effects.push(insert_preset(kind).ok_or_else(|| format!("unknown insert {kind}"))?);
        self.exec(Command::SetTrackAudio { target, effects })
    }

    pub fn remove_insert(&mut self, track: &str, index: usize) -> Result<(), String> {
        let (target, mut effects) = self.track_inserts(track)?;
        if index >= effects.len() {
            return Err(format!("no insert {index}"));
        }
        effects.remove(index);
        self.exec(Command::SetTrackAudio { target, effects })
    }

    pub fn export_presets(&self) -> Vec<PresetDto> {
        Preset::all()
            .into_iter()
            .map(|p| PresetDto {
                name: p.name,
                loudness_lufs: p.loudness_lufs,
            })
            .collect()
    }

    /// Queue an export of the whole sequence and make sure a worker is running.
    pub fn export_start(
        &mut self,
        output: String,
        preset: &str,
        normalize: Option<f32>,
        caption_sidecar: bool,
    ) -> Result<u64, String> {
        let preset = Preset::all()
            .into_iter()
            .find(|p| p.name == preset)
            .ok_or_else(|| format!("unknown preset {preset}"))?;
        let seq = self.first_sequence()?.clone();
        if seq.duration() <= Rational::ZERO {
            return Err("the sequence is empty".into());
        }
        // Captions as a sidecar next to the movie (GFX-06), written up front:
        // they do not depend on the render.
        if caption_sidecar && !seq.captions.is_empty() {
            let stem = match output.rfind('.') {
                Some(i) if !output[i..].contains('/') => &output[..i],
                _ => output.as_str(),
            };
            self.export_srt(&format!("{stem}.srt"))?;
        }
        let job = ExportJob {
            sequence: seq,
            range: (Rational::ZERO, self.first_sequence()?.duration()),
            sample_rate: 48_000,
            gain_db: 0.0,
        };
        let media: Vec<(MediaId, String)> = self
            .project()
            .map(|p| p.media.iter().map(|m| (m.id, m.path.clone())).collect())
            .unwrap_or_default();
        let id = self
            .exports
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .submit(preset.name.clone(), job, output, 0);
        let sequences = self
            .project()
            .map(|p| p.sequences.clone())
            .unwrap_or_default();
        let spec = (preset, normalize, media, sequences);
        if self
            .export_worker
            .load(std::sync::atomic::Ordering::Acquire)
        {
            if let Some(shared) = &self.export_specs_shared {
                shared
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id, spec);
            }
        } else {
            self.export_specs.insert(id, spec);
            self.spawn_export_worker();
        }
        Ok(id.0)
    }

    fn spawn_export_worker(&mut self) {
        if self
            .export_worker
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return;
        }
        let queue = Arc::clone(&self.exports);
        let running = Arc::clone(&self.export_worker);
        let specs = std::mem::take(&mut self.export_specs);
        let specs = Arc::new(Mutex::new(specs));
        self.export_specs_shared = Some(Arc::clone(&specs));
        std::thread::Builder::new()
            .name("debut-export".into())
            .spawn(move || {
                loop {
                    let next = queue.lock().unwrap_or_else(|e| e.into_inner()).take_next();
                    let Some((id, mut job, output, control)) = next else {
                        break;
                    };
                    let spec = specs.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                    let result = (|| -> Result<debut_export::Progress, String> {
                        let (preset, normalize, media, sequences) =
                            spec.ok_or("missing export spec")?;
                        let mut frames = debut_engine::FrameSource::new(4);
                        let mut samples = debut_engine::SampleCache::new(48_000);
                        frames.set_sequences(&sequences);
                        samples.set_sequences(&sequences);
                        for (mid, path) in &media {
                            let dec = FfmpegDecoder::open(path).map_err(|e| e.to_string())?;
                            let has_audio = dec.audio_info().is_some();
                            frames.add(*mid, Box::new(dec)).map_err(|e| e.to_string())?;
                            if has_audio {
                                samples
                                    .add(
                                        *mid,
                                        Box::new(
                                            FfmpegDecoder::open(path).map_err(|e| e.to_string())?,
                                        ),
                                    )
                                    .map_err(|e| e.to_string())?;
                            }
                        }
                        if let Some(target) = normalize {
                            let (lufs, tp) =
                                measure_loudness(&job, &mut samples).map_err(|e| e.to_string())?;
                            if let Some(lufs) = lufs {
                                job.gain_db = 20.0 * normalize_gain(lufs, target, tp, -1.0).log10();
                            }
                        }
                        let (w, h) = (job.sequence.width, job.sequence.height);
                        let mut encoder = FfmpegEncoder::create(
                            &output,
                            EncodeSettings {
                                width: w,
                                height: h,
                                frame_rate: job.sequence.frame_rate,
                                crf: preset.quality.max(1),
                                audio: Some(AudioEncodeSettings {
                                    channels: 2,
                                    sample_rate: 48_000,
                                    bitrate: preset.audio_bitrate.max(96_000),
                                }),
                            },
                        )
                        .map_err(|e| e.to_string())?;
                        let mut backend = AnyBackend::detect();
                        let q = Arc::clone(&queue);
                        let progress = match &mut backend {
                            AnyBackend::Cpu(b) => export(
                                &job,
                                b,
                                &mut frames,
                                &mut samples,
                                &mut encoder,
                                &control,
                                |p| {
                                    q.lock()
                                        .unwrap_or_else(|e| e.into_inner())
                                        .report_progress(id, p)
                                },
                            ),
                            AnyBackend::Gpu(b) => export(
                                &job,
                                b.as_mut(),
                                &mut frames,
                                &mut samples,
                                &mut encoder,
                                &control,
                                |p| {
                                    q.lock()
                                        .unwrap_or_else(|e| e.into_inner())
                                        .report_progress(id, p)
                                },
                            ),
                        }
                        .map_err(|e| e.to_string())?;
                        Box::new(encoder).finish().map_err(|e| e.to_string())?;
                        Ok(progress)
                    })();
                    queue
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .finish(id, result);
                }
                running.store(false, std::sync::atomic::Ordering::Release);
            })
            .expect("spawn export worker");
    }

    pub fn export_status(&self) -> Vec<ExportStatusDto> {
        let q = self.exports.lock().unwrap_or_else(|e| e.into_inner());
        q.entries()
            .map(|e| ExportStatusDto {
                id: e.id.0,
                name: e.name.clone(),
                output: e.output.clone(),
                state: match e.state {
                    JobState::Queued => "queued",
                    JobState::Running => "running",
                    JobState::Paused => "paused",
                    JobState::Done => "done",
                    JobState::Failed => "failed",
                    JobState::Cancelled => "cancelled",
                }
                .into(),
                frames_done: e.progress.frames_done,
                frames_total: e.progress.frames_total,
                loudness_lufs: e.progress.loudness_lufs,
                true_peak_db: e.progress.true_peak_db,
                error: e.error.clone(),
            })
            .collect()
    }

    pub fn export_control(&self, id: u64, action: &str) {
        let mut q = self.exports.lock().unwrap_or_else(|e| e.into_inner());
        match action {
            "pause" => q.pause(JobId(id)),
            "resume" => q.resume(JobId(id)),
            _ => q.cancel(JobId(id)),
        }
    }
}

#[tauri::command]
pub fn set_track_mix(state: State<'_, Shared>, track: String, mix: TrackMix) -> Result<(), String> {
    lock(&state).set_track_mix(&track, mix)
}

#[tauri::command]
pub fn add_insert(state: State<'_, Shared>, track: String, kind: String) -> Result<(), String> {
    lock(&state).add_insert(&track, &kind)
}

#[tauri::command]
pub fn remove_insert(state: State<'_, Shared>, track: String, index: usize) -> Result<(), String> {
    lock(&state).remove_insert(&track, index)
}

#[tauri::command]
pub fn export_presets(state: State<'_, Shared>) -> Vec<PresetDto> {
    lock(&state).export_presets()
}

#[tauri::command]
pub fn export_start(
    state: State<'_, Shared>,
    output: String,
    preset: String,
    normalize: Option<f32>,
    caption_sidecar: Option<bool>,
) -> Result<u64, String> {
    lock(&state).export_start(output, &preset, normalize, caption_sidecar.unwrap_or(false))
}

#[tauri::command]
pub fn export_status(state: State<'_, Shared>) -> Vec<ExportStatusDto> {
    lock(&state).export_status()
}

#[tauri::command]
pub fn export_pause(state: State<'_, Shared>, id: u64) {
    lock(&state).export_control(id, "pause")
}

#[tauri::command]
pub fn export_resume(state: State<'_, Shared>, id: u64) {
    lock(&state).export_control(id, "resume")
}

#[tauri::command]
pub fn export_cancel(state: State<'_, Shared>, id: u64) {
    lock(&state).export_control(id, "cancel")
}

#[derive(Serialize, Debug)]
pub struct MarkerDto {
    pub id: String,
    /// Absolute sequence time, also for clip markers.
    pub at: f64,
    pub duration: f64,
    pub color: [u8; 3],
    pub note: String,
    /// `None` for a timeline marker, else the owning clip.
    pub clip: Option<String>,
}

#[derive(Deserialize)]
pub struct MarkerEdit {
    pub note: Option<String>,
    pub color: Option<[u8; 3]>,
    pub duration: Option<f64>,
    pub at: Option<f64>,
}

impl Session {
    /// Timeline markers plus every clip marker, in sequence time.
    pub fn markers(&self) -> Result<Vec<MarkerDto>, String> {
        let seq = self.first_sequence()?;
        let mut out: Vec<MarkerDto> = seq
            .markers
            .iter()
            .map(|m| MarkerDto {
                id: id_str(m.id.0),
                at: secs(m.at),
                duration: secs(m.duration),
                color: m.color,
                note: m.note.clone(),
                clip: None,
            })
            .collect();
        for t in &seq.tracks {
            for c in &t.clips {
                for m in &c.markers {
                    out.push(MarkerDto {
                        id: id_str(m.id.0),
                        at: secs(c.timeline_in + m.at),
                        duration: secs(m.duration),
                        color: m.color,
                        note: m.note.clone(),
                        clip: Some(id_str(c.id.0)),
                    });
                }
            }
        }
        out.sort_by(|a, b| a.at.total_cmp(&b.at));
        Ok(out)
    }

    fn marker_target(&self, id: MarkerId) -> Result<(MarkerTarget, Marker, Rational), String> {
        let seq = self.first_sequence()?;
        if let Some(m) = seq.markers.iter().find(|m| m.id == id) {
            return Ok((
                MarkerTarget {
                    sequence: seq.id,
                    clip: None,
                },
                m.clone(),
                Rational::ZERO,
            ));
        }
        for t in &seq.tracks {
            for c in &t.clips {
                if let Some(m) = c.markers.iter().find(|m| m.id == id) {
                    return Ok((
                        MarkerTarget {
                            sequence: seq.id,
                            clip: Some((t.id, c.id)),
                        },
                        m.clone(),
                        c.timeline_in,
                    ));
                }
            }
        }
        Err("marker not found".into())
    }

    /// Add a timeline marker, or a clip marker when `clip` is given, at `at` seconds.
    pub fn add_marker(
        &mut self,
        at: f64,
        note: String,
        clip: Option<String>,
    ) -> Result<String, String> {
        let seq = self.first_sequence()?;
        let (seq_id, fr) = (seq.id, seq.frame_rate);
        let t = frames_of(at, fr);
        let (target, local) = match clip {
            None => (
                MarkerTarget {
                    sequence: seq_id,
                    clip: None,
                },
                t,
            ),
            Some(cid) => {
                let cid = ClipId(parse_id(&cid)?);
                let (track, c) = seq
                    .tracks
                    .iter()
                    .find_map(|tr| tr.clip(cid).map(|c| (tr.id, c)))
                    .ok_or("clip not found")?;
                (
                    MarkerTarget {
                        sequence: seq_id,
                        clip: Some((track, cid)),
                    },
                    t - c.timeline_in,
                )
            }
        };
        let marker = Marker::new(self.ids.fresh(), local, note);
        let id = id_str(marker.id.0);
        self.exec(Command::AddMarker { target, marker })?;
        Ok(id)
    }

    pub fn update_marker(&mut self, id: &str, edit: MarkerEdit) -> Result<(), String> {
        let fr = self.first_sequence()?.frame_rate;
        let (target, mut m, base) = self.marker_target(MarkerId(parse_id(id)?))?;
        if let Some(n) = edit.note {
            m.note = n;
        }
        if let Some(c) = edit.color {
            m.color = c;
        }
        if let Some(d) = edit.duration {
            m.duration = frames_of(d.max(0.0), fr);
        }
        if let Some(a) = edit.at {
            m.at = frames_of(a, fr) - base;
        }
        self.exec(Command::UpdateMarker { target, marker: m })
    }

    pub fn remove_marker(&mut self, id: &str) -> Result<(), String> {
        let (target, m, _) = self.marker_target(MarkerId(parse_id(id)?))?;
        self.exec(Command::RemoveMarker { target, id: m.id })
    }

    /// Write the marker list (tab-separated timecodes) to `path`.
    pub fn export_markers(&self, path: &str) -> Result<usize, String> {
        let seq = self.first_sequence()?;
        let mut rows: Vec<(Rational, &Marker)> = seq.markers.iter().map(|m| (m.at, m)).collect();
        for t in &seq.tracks {
            for c in &t.clips {
                rows.extend(c.markers.iter().map(|m| (c.timeline_in + m.at, m)));
            }
        }
        let text = debut_project::marker_list(&rows, seq.frame_rate);
        self.store
            .write(path, text.as_bytes())
            .map_err(|e| e.to_string())?;
        Ok(rows.len())
    }
}

// ---- captions (GFX-05, GFX-06) -------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CaptionDto {
    pub id: String,
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Deserialize)]
pub struct CaptionEdit {
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub text: Option<String>,
}

fn caption_dto(c: &Caption) -> CaptionDto {
    CaptionDto {
        id: id_str(c.id.0),
        start: secs(c.start),
        end: secs(c.end),
        text: c.text.clone(),
    }
}

impl Session {
    pub fn captions(&self) -> Result<Vec<CaptionDto>, String> {
        Ok(self
            .first_sequence()?
            .captions
            .iter()
            .map(caption_dto)
            .collect())
    }

    /// Add a caption over `[start, end)` seconds; returns its id.
    pub fn add_caption(&mut self, start: f64, end: f64, text: String) -> Result<String, String> {
        let seq = self.first_sequence()?;
        let (seq_id, fr) = (seq.id, seq.frame_rate);
        let caption = Caption::new(
            self.ids.fresh(),
            frames_of(start, fr),
            frames_of(end, fr),
            text,
        );
        let id = id_str(caption.id.0);
        self.exec(Command::AddCaption {
            sequence: seq_id,
            caption,
        })?;
        Ok(id)
    }

    pub fn update_caption(&mut self, id: &str, edit: CaptionEdit) -> Result<(), String> {
        let seq = self.first_sequence()?;
        let (seq_id, fr) = (seq.id, seq.frame_rate);
        let cid = CaptionId(parse_id(id)?);
        let mut c = seq
            .captions
            .iter()
            .find(|c| c.id == cid)
            .cloned()
            .ok_or("caption not found")?;
        if let Some(s) = edit.start {
            c.start = frames_of(s, fr);
        }
        if let Some(e) = edit.end {
            c.end = frames_of(e, fr);
        }
        if let Some(t) = edit.text {
            c.text = t;
        }
        self.exec(Command::UpdateCaption {
            sequence: seq_id,
            caption: c,
        })
    }

    pub fn remove_caption(&mut self, id: &str) -> Result<(), String> {
        let seq_id = self.first_sequence()?.id;
        self.exec(Command::RemoveCaption {
            sequence: seq_id,
            id: CaptionId(parse_id(id)?),
        })
    }

    /// Read an .srt file and add every cue (one undoable step); returns the count.
    pub fn import_srt(&mut self, path: &str) -> Result<usize, String> {
        let bytes = self.store.read(path).map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&bytes);
        let cues = debut_graphics::parse_srt(&text).map_err(|e| e.to_string())?;
        let seq_id = self.first_sequence()?.id;
        let cmds: Vec<Command> = cues
            .into_iter()
            .map(|(start, end, text)| Command::AddCaption {
                sequence: seq_id,
                caption: Caption::new(self.ids.fresh(), start, end, text),
            })
            .collect();
        let n = cmds.len();
        if n > 0 {
            self.exec(Command::Group(cmds))?;
        }
        Ok(n)
    }

    pub fn caption_settings(&self) -> Result<CaptionSettings, String> {
        Ok(self.first_sequence()?.caption_settings.clone())
    }

    pub fn set_caption_settings(&mut self, settings: CaptionSettings) -> Result<(), String> {
        let sequence = self.first_sequence()?.id;
        self.exec(Command::SetCaptionSettings { sequence, settings })
    }

    /// Write the sequence's captions to `path`: WebVTT for a `.vtt` extension,
    /// SubRip otherwise; returns the count.
    pub fn export_srt(&self, path: &str) -> Result<usize, String> {
        let seq = self.first_sequence()?;
        let cues = seq
            .captions
            .iter()
            .map(|c| (c.start, c.end, c.text.as_str()));
        let text = if path.to_lowercase().ends_with(".vtt") {
            debut_graphics::format_vtt(cues)
        } else {
            debut_graphics::format_srt(cues)
        };
        self.store
            .write(path, text.as_bytes())
            .map_err(|e| e.to_string())?;
        Ok(seq.captions.len())
    }
}

#[tauri::command]
pub fn captions(state: State<'_, Shared>) -> Result<Vec<CaptionDto>, String> {
    lock(&state).captions()
}

#[tauri::command]
pub fn add_caption(
    state: State<'_, Shared>,
    start: f64,
    end: f64,
    text: String,
) -> Result<String, String> {
    lock(&state).add_caption(start, end, text)
}

#[tauri::command]
pub fn update_caption(
    state: State<'_, Shared>,
    id: String,
    edit: CaptionEdit,
) -> Result<(), String> {
    lock(&state).update_caption(&id, edit)
}

#[tauri::command]
pub fn remove_caption(state: State<'_, Shared>, id: String) -> Result<(), String> {
    lock(&state).remove_caption(&id)
}

#[tauri::command]
pub fn caption_settings(state: State<'_, Shared>) -> Result<CaptionSettings, String> {
    lock(&state).caption_settings()
}

#[tauri::command]
pub fn set_caption_settings(
    state: State<'_, Shared>,
    settings: CaptionSettings,
) -> Result<(), String> {
    lock(&state).set_caption_settings(settings)
}

#[tauri::command]
pub fn import_srt(state: State<'_, Shared>, path: String) -> Result<usize, String> {
    lock(&state).import_srt(&path)
}

#[tauri::command]
pub fn export_srt(state: State<'_, Shared>, path: String) -> Result<usize, String> {
    lock(&state).export_srt(&path)
}

#[tauri::command]
pub fn markers(state: State<'_, Shared>) -> Result<Vec<MarkerDto>, String> {
    lock(&state).markers()
}

#[tauri::command]
pub fn add_marker(
    state: State<'_, Shared>,
    at: f64,
    note: String,
    clip: Option<String>,
) -> Result<String, String> {
    lock(&state).add_marker(at, note, clip)
}

#[tauri::command]
pub fn update_marker(state: State<'_, Shared>, id: String, edit: MarkerEdit) -> Result<(), String> {
    lock(&state).update_marker(&id, edit)
}

#[tauri::command]
pub fn remove_marker(state: State<'_, Shared>, id: String) -> Result<(), String> {
    lock(&state).remove_marker(&id)
}

#[tauri::command]
pub fn export_markers(state: State<'_, Shared>, path: String) -> Result<usize, String> {
    lock(&state).export_markers(&path)
}

#[tauri::command]
pub fn import_media(state: State<'_, Shared>, path: String) -> Result<MediaDto, String> {
    lock(&state).import_media(path)
}

#[tauri::command]
pub fn media_list(state: State<'_, Shared>) -> Result<Vec<MediaDto>, String> {
    lock(&state).media_list()
}

#[tauri::command]
pub fn bins(state: State<'_, Shared>) -> Result<Vec<BinDto>, String> {
    lock(&state).bins()
}

#[tauri::command]
pub fn add_bin(
    state: State<'_, Shared>,
    name: String,
    filter: Option<String>,
) -> Result<String, String> {
    lock(&state).add_bin(name, filter)
}

#[tauri::command]
pub fn add_smart_bin(
    state: State<'_, Shared>,
    name: String,
    rule: RuleDto,
) -> Result<String, String> {
    lock(&state).add_smart_bin(name, rule)
}

#[tauri::command]
pub fn set_media_tags(
    state: State<'_, Shared>,
    media: String,
    keywords: Vec<String>,
    rating: u8,
) -> Result<(), String> {
    lock(&state).set_media_tags(&media, keywords, rating)
}

#[tauri::command]
pub fn rename_bin(state: State<'_, Shared>, id: String, name: String) -> Result<(), String> {
    lock(&state).rename_bin(&id, name)
}

#[tauri::command]
pub fn remove_bin(state: State<'_, Shared>, id: String) -> Result<(), String> {
    lock(&state).remove_bin(&id)
}

#[tauri::command]
pub fn assign_media(
    state: State<'_, Shared>,
    media: String,
    bin: Option<String>,
) -> Result<(), String> {
    lock(&state).assign_media(&media, bin)
}

#[tauri::command]
pub fn ensure_sequence(state: State<'_, Shared>) -> Result<SequenceDto, String> {
    lock(&state).ensure_sequence()
}

#[tauri::command]
pub fn sequences(state: State<'_, Shared>) -> Result<Vec<SequenceListDto>, String> {
    lock(&state).sequences()
}

#[tauri::command]
pub fn open_sequence(state: State<'_, Shared>, id: String) -> Result<SequenceDto, String> {
    lock(&state).open_sequence(&id)
}

#[tauri::command]
pub fn sequence(state: State<'_, Shared>) -> Result<SequenceDto, String> {
    let s = lock(&state);
    Ok(sequence_dto(s.first_sequence()?))
}

#[tauri::command]
pub fn add_clip(
    state: State<'_, Shared>,
    track: String,
    media: String,
    at: f64,
) -> Result<(), String> {
    lock(&state).add_clip(&track, &media, at)
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EditOp {
    RippleHead {
        track: String,
        clip: String,
        delta: f64,
    },
    RippleTail {
        track: String,
        clip: String,
        delta: f64,
    },
    Roll {
        track: String,
        clip: String,
        delta: f64,
    },
    Slip {
        track: String,
        clip: String,
        delta: f64,
    },
    Slide {
        track: String,
        clip: String,
        delta: f64,
    },
    Move {
        track: String,
        clip: String,
        delta: f64,
    },
    Blade {
        track: String,
        at: f64,
    },
    Extract {
        track: String,
        start: f64,
        end: f64,
    },
    Lift {
        track: String,
        start: f64,
        end: f64,
    },
    /// Set (duration in seconds) or clear (`None`) the dissolve into `clip`.
    Transition {
        track: String,
        clip: String,
        duration: Option<f64>,
    },
    /// Collapse `[start, end)` on every track into a new sequence and put one
    /// compound clip per track in its place (TL-07).
    Nest {
        start: f64,
        end: f64,
    },
}

#[tauri::command]
pub fn add_multicam(state: State<'_, Shared>, at: f64, media: Vec<String>) -> Result<(), String> {
    lock(&state).add_multicam(at, media)
}

#[tauri::command]
pub fn switch_angle(
    state: State<'_, Shared>,
    track: String,
    clip: String,
    angle: usize,
    cut: bool,
) -> Result<String, String> {
    lock(&state).switch_angle(&track, &clip, angle, cut)
}

#[derive(Serialize)]
pub struct TemplateDto {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[tauri::command]
pub fn add_title(
    state: State<'_, Shared>,
    at: f64,
    text: String,
    template: Option<String>,
) -> Result<String, String> {
    lock(&state).add_title_from(at, text, template)
}

#[tauri::command]
pub fn title_templates(state: State<'_, Shared>) -> Vec<TemplateDto> {
    lock(&state).title_templates()
}

#[tauri::command]
pub fn set_title(
    state: State<'_, Shared>,
    track: String,
    clip: String,
    title: Title,
) -> Result<(), String> {
    lock(&state).set_title(&track, &clip, title)
}

#[tauri::command]
pub fn edit(state: State<'_, Shared>, op: EditOp) -> Result<(), String> {
    lock(&state).edit(op)
}

// ---- playback -------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TransportAction {
    Play,
    Pause,
    Toggle,
    Seek { t: f64 },
    Step { n: i64 },
    Shuttle { forward: bool },
}

#[derive(Serialize)]
pub struct TickDto {
    pub frame: i64,
    pub position: f64,
    pub playing: bool,
    pub changed: bool,
    pub dropped: u64,
    /// Preview resolution in use: 1, 2 or 4 (PB-03).
    pub preview_divisor: u32,
}

/// Playback resolution selector (PB-03).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewQuality {
    Full,
    Half,
    Quarter,
    Auto,
}

#[tauri::command]
pub fn transport(state: State<'_, Shared>, action: TransportAction) -> Result<(), String> {
    lock(&state).transport(action)
}

/// Advance the player; the UI calls this once per animation frame and fetches
/// pixels when `changed`.
#[tauri::command]
pub fn tick(state: State<'_, Shared>) -> Result<TickDto, String> {
    lock(&state).tick()
}

#[tauri::command]
pub fn set_preview_quality(state: State<'_, Shared>, quality: PreviewQuality) {
    lock(&state).set_preview_quality(quality)
}

#[tauri::command]
pub fn scopes(state: State<'_, Shared>) -> Result<tauri::ipc::Response, String> {
    Ok(tauri::ipc::Response::new(lock(&state).scopes()?))
}

/// The current frame as `width * height * 4` bytes of RGBA8, preceded by two
/// little-endian u32 (width, height).
#[tauri::command]
pub fn frame_pixels(state: State<'_, Shared>) -> Result<tauri::ipc::Response, String> {
    let (w, h, rgba) = lock(&state).frame_pixels()?;
    let mut bytes = Vec::with_capacity(8 + rgba.len());
    bytes.extend_from_slice(&w.to_le_bytes());
    bytes.extend_from_slice(&h.to_le_bytes());
    bytes.extend_from_slice(&rgba);
    Ok(tauri::ipc::Response::new(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/debut-platform-native/tests/fixtures/test_25fps_2s.mp4"
    );

    /// The whole desktop flow the UI drives, headless: import, sequence, insert,
    /// play against the silent audio output, pull frames, edit, undo.
    #[test]
    fn desktop_session_flow() {
        let dir = std::env::temp_dir().join(format!("debut-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.debut").to_string_lossy().into_owned();
        let mut s = Session::new();
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

        // Effects: add a grade, set exposure as a constant, then keyframe opacity.
        let clip_id = seq.tracks[0].clips[0].id.clone();
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
        let sum = |px: &[u8]| {
            px.chunks(4)
                .map(|p| p[0] as u64 + p[1] as u64 + p[2] as u64)
                .sum::<u64>()
        };
        assert!(sum(&half) > 0);
        s.remove_effect(&v, &clip_id, 1).unwrap();
        let (_, _, full) = s.frame_pixels().unwrap();
        assert!(
            sum(&full) > sum(&half),
            "removing the opacity ramp brightens the frame"
        );
        assert!(s.remove_effect(&v, &clip_id, 7).is_err());

        // Pixel checks below index a 64x36 frame, so pin the preview (Auto may
        // step down on a loaded machine).
        s.set_preview_quality(PreviewQuality::Full);
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
        let at = |px: &[u8], x: usize, y: usize| {
            let i = (y * 64 + x) * 4;
            px[i] as u32 + px[i + 1] as u32 + px[i + 2] as u32
        };
        assert_eq!(at(&masked, 1, 1), 0, "corner is masked out");
        assert!(at(&masked, 32, 18) > 60, "centre is kept");
        s.set_effect_options(
            &v,
            &clip_id,
            mask_ix,
            EffectOptions {
                invert: Some(true),
                shape: Some(MaskShape::Ellipse),
                color: None,
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
        s.edit(EditOp::Lift {
            track: lt_track.clone(),
            start: 1.0,
            end: 6.0,
        })
        .unwrap();
        // Captions (GFX-05/06): burned in near the bottom while active, SRT round trip.
        s.transport(TransportAction::Seek { t: 1.4 }).unwrap();
        let (_, _, bare) = s.frame_pixels().unwrap();
        let cap = s.add_caption(1.0, 2.0, "Hello".into()).unwrap();
        let (_, h, captioned) = s.frame_pixels().unwrap();
        // On a 36-line frame the scaled caption box spans roughly rows 13..34.
        let split = (h as usize / 3) * 64 * 4;
        let changed = |a: &[u8], b: &[u8]| a.iter().zip(b).filter(|(x, y)| x != y).count();
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

        // Lift the title again so the export below covers the original 2.4 s.
        s.edit(EditOp::Lift {
            track: title_track.id.clone(),
            start: 1.0,
            end: 6.0,
        })
        .unwrap();
        assert_eq!(sequence_dto(s.first_sequence().unwrap()).duration, 2.4);

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
            .export_start(out.clone(), "YouTube 1080p", Some(-14.0), true)
            .unwrap();
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
        let mut s2 = Session::new();
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

    /// `cargo test -p debut-desktop -- --ignored bench` prints the frame cost per
    /// preview divisor on a generated 1080p clip: what Auto is working with.
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
        let mut s = Session::new();
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
}
