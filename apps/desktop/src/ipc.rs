//! IPC commands. All project mutation goes through the command history; the
//! player is rebuilt from the project's sequence after every edit.

use debut_audio::normalize_gain;
use debut_command::{Command, Target};
use debut_core::{ClipId, FrameRate, IdGen, MediaId, Rational, TrackId};
use debut_engine::{Player, Stats, Workspace};
use debut_export::job::to_rgba8;
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
    schema, AudioEffect, Clip, ClipSource, Effect, EqBand, EqKind, GradeFx, Param, Project,
    Sequence, Track, TrackKind, TrackMix, TransformFx,
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
    player: Option<Player>,
    audio_out: Option<Box<dyn AudioOut>>,
    backend: AnyBackend,
    /// Media probed so far: id -> (width, height, duration, has_audio).
    probed: std::collections::HashMap<MediaId, (u32, u32, Rational, bool)>,
    exports: Arc<Mutex<ExportQueue>>,
    export_worker: Arc<std::sync::atomic::AtomicBool>,
    /// Per-job preset, normalization target and media paths (shared with the worker).
    export_specs: std::collections::HashMap<JobId, ExportSpec>,
    export_specs_shared: Option<Arc<Mutex<std::collections::HashMap<JobId, ExportSpec>>>>,
}

type ExportSpec = (Preset, Option<f32>, Vec<(MediaId, String)>);

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
            player: None,
            audio_out: None,
            backend: AnyBackend::detect(),
            probed: Default::default(),
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

    fn first_sequence(&self) -> Result<&Sequence, String> {
        self.project()
            .and_then(|p| p.sequences.first())
            .ok_or_else(|| "no sequence".to_string())
    }

    /// Rebuild the player's view of the sequence after an edit, keeping transport
    /// state; create it on first use.
    fn sync_player(&mut self) -> Result<(), String> {
        let Some(seq) = self.project().and_then(|p| p.sequences.first()).cloned() else {
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
}

#[derive(Serialize)]
pub struct ClipDto {
    pub id: String,
    pub media: Option<String>,
    pub timeline_in: f64,
    pub duration: f64,
    pub source_in: f64,
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
                        timeline_in: secs(c.timeline_in),
                        duration: secs(c.duration),
                        source_in: secs(c.source_in),
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
        let p = self.player.as_mut().ok_or("no sequence")?;
        let changed = p.tick().map_err(|e| e.to_string())?.is_some();
        let Stats { dropped, .. } = p.stats();
        Ok(TickDto {
            frame: p.transport.current_frame(),
            position: secs(p.transport.position()),
            playing: p.transport.is_playing(),
            changed,
            dropped,
        })
    }

    /// The current frame as `(width, height, RGBA8 bytes)`.
    pub fn frame_pixels(&mut self) -> Result<(u32, u32, Vec<u8>), String> {
        let Session {
            player, backend, ..
        } = self;
        let p = player.as_mut().ok_or("no sequence")?;
        let graph = p.current_graph();
        let (w, h, px) = backend
            .render_pixels(&graph, &mut p.frames)
            .map_err(|e| e.to_string())?;
        Ok((w, h, to_rgba8(&px)))
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
            other => return Err(format!("unknown effect kind {other}")),
        };
        self.exec(Command::AddEffect {
            target,
            clip: clip_id,
            effect,
            index: None,
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
    ) -> Result<u64, String> {
        let preset = Preset::all()
            .into_iter()
            .find(|p| p.name == preset)
            .ok_or_else(|| format!("unknown preset {preset}"))?;
        let seq = self.first_sequence()?.clone();
        if seq.duration() <= Rational::ZERO {
            return Err("the sequence is empty".into());
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
        let spec = (preset, normalize, media);
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
                        let (preset, normalize, media) = spec.ok_or("missing export spec")?;
                        let mut frames = debut_engine::FrameSource::new(4);
                        let mut samples = debut_engine::SampleCache::new(48_000);
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
) -> Result<u64, String> {
    lock(&state).export_start(output, &preset, normalize)
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

#[tauri::command]
pub fn import_media(state: State<'_, Shared>, path: String) -> Result<MediaDto, String> {
    lock(&state).import_media(path)
}

#[tauri::command]
pub fn ensure_sequence(state: State<'_, Shared>) -> Result<SequenceDto, String> {
    lock(&state).ensure_sequence()
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
        assert_eq!(dto.tracks[1].mix.gain_db, -6.0);
        assert_eq!(dto.tracks[1].inserts, vec!["Limiter".to_string()]);
        assert!(s.add_insert(&a, "nope").is_err());

        // Export through the queue worker, normalized to -14 LUFS.
        let out = dir.join("out.mp4").to_string_lossy().into_owned();
        let job_id = s
            .export_start(out.clone(), "YouTube 1080p", Some(-14.0))
            .unwrap();
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
}
