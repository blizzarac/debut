//! The engine's command surface (PLT-01): one `Session` per open project,
//! used by the desktop shell over Tauri IPC and, as the web platform layer
//! lands, by the WASM shell. All project mutation goes through the command
//! history; the player is rebuilt from the active sequence after every edit.
//! Platform specifics (decoders, encoders, audio out, files) come in through
//! [`Platform`], so nothing here depends on a particular OS or browser.

use crate::{Player, Stats, Workspace};
use debut_audio::normalize_gain;
use debut_command::{Command, MarkerTarget, Target};
use debut_core::id::{BinId, CaptionId, MarkerId, TemplateId};
use debut_core::{ClipId, FrameRate, IdGen, MediaId, Rational, SequenceId, Timecode, TrackId};
use debut_export::{export, measure_loudness, ExportJob, ExportQueue, JobId, JobState, Preset};
use debut_platform::audio_out::AudioOut;
use debut_platform::{AudioEncodeSettings, EncodeSettings, FileStore, Platform};
use debut_project::media_ref::{MediaMetadata, MediaRef};
use debut_project::{
    schema, AudioEffect, Bin, Caption, CaptionSettings, Clip, ClipSource, Duck, Effect, EqBand,
    EqKind, GradeFx, KeyFx, Marker, MaskFx, MaskShape, Param, Project, SavedTitleTemplate,
    Sequence, Shape, SpeedKey, Title, TitleStyle, Track, TrackKind, TrackMix, TransformFx,
    Transition, TransitionKind,
};
use debut_render::AnyBackend;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

mod ai;
mod captions;
mod collab;
mod effects;
mod export;
mod ingest;
mod markers;
mod media;
mod mixer;
mod multicam;
mod playback;
mod plugins;
mod proxies;
mod shapes;
mod shortcuts;
mod snapshots;
mod targeting;
mod telemetry;
mod timeline;
mod titles;
mod waveforms;

pub use self::collab::{CollabDto, LockDto, PeerDto};
pub use self::mixer::DuckDto;
use self::mixer::*;
use self::proxies::ProxyJobs;
pub use self::proxies::ProxyStatusDto;
pub use self::shortcuts::{
    BindingDto, ChordDto, KeyOverrideDto, KeymapDto, ShortcutActionDto, ShortcutsDto,
};
pub use self::snapshots::{ClipChangeDto, SnapshotDiffDto, SnapshotDto};
pub use self::targeting::TargetingDto;
pub use self::telemetry::TelemetryDto;
use self::waveforms::WaveformCache;
pub use self::{
    ai::*, captions::*, effects::*, export::*, ingest::*, markers::*, media::*, multicam::*,
    playback::*, plugins::*, timeline::*, titles::*,
};

pub struct Session {
    /// Codecs, audio output and files for this target.
    platform: Arc<dyn Platform>,
    workspace: Option<Workspace>,
    store: Arc<dyn FileStore>,
    /// Commands replayed from the journal when the current file was opened.
    recovered: usize,
    ids: IdGen,
    /// The sequence shown in the timeline (TL-07): a nested one while it is
    /// opened for editing, else the project's first.
    active: Option<SequenceId>,
    /// Media whose file could not be opened this session (MED-05).
    offline: std::collections::HashSet<MediaId>,
    /// Waveform peaks per media, built in the background (AUD-04).
    waveforms: WaveformCache,
    /// Proxy jobs (MED-05) and whether the player decodes from proxies.
    proxies: ProxyJobs,
    use_proxies: bool,
    /// Edits follow a clip's linked partners on other tracks (TL-05).
    linked_selection: bool,
    /// Source patching and track targeting per sequence (TL-05).
    patches: std::collections::HashMap<SequenceId, self::targeting::Patch>,
    /// Shared editing session, when joined (COL).
    collab: Option<self::collab::Collab>,
    /// AI features opted into (GFX-04); off by default.
    ai_enabled: bool,
    /// The program monitor window, when open (PB-05).
    program: Option<debut_render::SurfaceViewer>,
    program_error: Option<String>,
    /// The last plugin scan (FX-15, AUD-09).
    plugin_scan: Option<debut_platform::plugin_host::ScanResult>,
    /// Per-user settings (plugin approvals, telemetry consent).
    settings: crate::settings::Settings,
    /// Approved plugin binaries, shared with `plugin_host` (NFR-13).
    trusted: crate::trust::Trusted,
    /// The platform's plugin host behind the approval check.
    plugin_host: Option<Arc<crate::trust::TrustedHost>>,
    /// Usage counts and crash reports, when the user opted in (NFR-15).
    telemetry: Option<crate::telemetry::Report>,
    telemetry_saved: std::time::Duration,
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

/// Licensing facts for the About panel (NFR-14).
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct AboutDto {
    pub version: String,
    /// The codec library's license as it reports it ("GPL version 2 or
    /// later", "LGPL version 2.1 or later"), when the platform has one.
    pub codec_license: Option<String>,
    pub codec_configuration: Option<String>,
    /// True when the codec library is a GPL build: a distribution bundling
    /// it falls under the GPL.
    pub codec_gpl: bool,
}

#[derive(Serialize)]
pub struct FileStatus {
    pub path: Option<String>,
    pub dirty: bool,
    pub recovered: usize,
}

impl Session {
    pub fn new(platform: Arc<dyn Platform>) -> Self {
        let settings = crate::settings::Settings::load(platform.as_ref());
        let trusted: crate::trust::Trusted =
            Arc::new(std::sync::RwLock::new(settings.trusted_plugins.clone()));
        let plugin_host = platform.plugins().map(|inner| {
            Arc::new(crate::trust::TrustedHost::new(
                inner,
                platform.file_store(),
                Arc::clone(&trusted),
            ))
        });
        let telemetry = settings.telemetry.then(|| {
            let mut r = crate::telemetry::Report::load(platform.as_ref());
            r.sessions += 1;
            r
        });
        Self {
            telemetry,
            telemetry_saved: std::time::Duration::ZERO,
            settings,
            trusted,
            plugin_host,
            store: platform.file_store(),
            platform,
            workspace: None,
            recovered: 0,
            ids: IdGen::random(),
            active: None,
            offline: Default::default(),
            waveforms: Default::default(),
            proxies: Default::default(),
            use_proxies: false,
            linked_selection: true,
            patches: Default::default(),
            collab: None,
            plugin_scan: None,
            program: None,
            ai_enabled: false,
            program_error: None,
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

    /// Start a new, empty project with a default file path.
    pub fn new_project(&mut self, name: String) -> Result<(), String> {
        let id = self.ids.fresh();
        let path = Self::default_path(&name);
        self.start(Project::new(id, name), path)
    }

    /// Start a session around a project given as JSON (schema-migrated).
    pub fn open_project_json(&mut self, json: &str) -> Result<(), String> {
        let project = schema::from_json(json).map_err(|e| e.to_string())?;
        let path = Self::default_path(&project.name);
        self.start(project, path)
    }

    /// The open project as JSON.
    pub fn project_json(&self) -> Result<String, String> {
        let p = self.project().ok_or("no project open")?;
        schema::to_json(p).map_err(|e| e.to_string())
    }

    /// Run a raw command given as JSON (the generic edit entry point).
    pub fn execute_json(&mut self, command_json: &str) -> Result<(), String> {
        let cmd: Command = serde_json::from_str(command_json).map_err(|e| e.to_string())?;
        self.exec(cmd)
    }

    pub fn undo(&mut self) -> Result<bool, String> {
        // In a shared session an undo is an ordinary edit for the others.
        let shared = self.collab.is_some().then(|| {
            self.workspace
                .as_ref()
                .and_then(|w| w.history.peek_undo())
                .map(|(apply, back)| (apply.clone(), back.clone()))
        });
        if let Some(Some((apply, _))) = &shared {
            self.collab_before(apply)?;
        }
        let r = self.workspace_mut()?.undo().map_err(|e| e.to_string())?;
        if let (true, Some(Some((apply, back)))) = (r, shared) {
            self.collab_after(apply, back);
        }
        self.sync_player()?;
        Ok(r)
    }

    pub fn redo(&mut self) -> Result<bool, String> {
        let shared = self.collab.is_some().then(|| {
            self.workspace
                .as_ref()
                .and_then(|w| w.history.peek_redo())
                .map(|(apply, back)| (apply.clone(), back.clone()))
        });
        if let Some(Some((apply, _))) = &shared {
            self.collab_before(apply)?;
        }
        let r = self.workspace_mut()?.redo().map_err(|e| e.to_string())?;
        if let (true, Some(Some((apply, back)))) = (r, shared) {
            self.collab_after(apply, back);
        }
        self.sync_player()?;
        Ok(r)
    }

    pub fn can_undo(&self) -> bool {
        self.workspace
            .as_ref()
            .is_some_and(|w| w.history.can_undo())
    }

    /// The sequence shown in the timeline.
    pub fn sequence(&self) -> Result<SequenceDto, String> {
        Ok(sequence_dto(self.first_sequence()?))
    }

    pub(crate) fn project(&self) -> Option<&Project> {
        self.workspace.as_ref().map(|w| &w.project)
    }

    pub(crate) fn project_mut(&mut self) -> Result<&mut Project, String> {
        self.workspace
            .as_mut()
            .map(|w| &mut w.project)
            .ok_or_else(|| "no project open".to_string())
    }

    /// Version and licensing facts (NFR-14).
    pub fn about(&self) -> AboutDto {
        let codec = self.platform.codec_license();
        AboutDto {
            version: env!("CARGO_PKG_VERSION").to_string(),
            codec_gpl: codec
                .as_ref()
                .is_some_and(|(l, _)| l.starts_with("GPL") || l.contains(" GPL")),
            codec_license: codec.as_ref().map(|(l, _)| l.clone()),
            codec_configuration: codec.map(|(_, c)| c),
        }
    }

    /// Steps in the undo history so far; pass it to `group_undo_since` to
    /// turn a run of edits into one undo step (scripts, NFR-11).
    pub fn undo_mark(&self) -> usize {
        self.workspace.as_ref().map_or(0, |w| w.undo_mark())
    }

    /// Make every edit since `mark` a single undo step; returns how many
    /// edits it holds.
    pub fn group_undo_since(&mut self, mark: usize) -> usize {
        self.workspace.as_mut().map_or(0, |w| w.squash_since(mark))
    }

    pub(crate) fn workspace_mut(&mut self) -> Result<&mut Workspace, String> {
        self.workspace
            .as_mut()
            .ok_or_else(|| "no project open".to_string())
    }

    /// Where a new project lives until Save As: `~/debut-projects/<name>.debut`.
    pub(crate) fn default_path(name: &str) -> String {
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
            if let Ok(dec) = self.platform.open_decoder(&path) {
                if let Some(info) = media_info(dec.as_ref()) {
                    self.probed.insert(id, info);
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

    pub(crate) fn exec(&mut self, cmd: Command) -> Result<(), String> {
        if let Some(t) = self.telemetry.as_mut() {
            t.edit(&cmd);
        }
        let inverse = self.collab_before(&cmd)?;
        let sent = inverse.as_ref().map(|_| cmd.clone());
        self.workspace_mut()?
            .execute(cmd)
            .map_err(|e| e.to_string())?;
        if let (Some(cmd), Some(inverse)) = (sent, inverse) {
            self.collab_after(cmd, inverse);
        }
        self.sync_player()
    }

    /// The sequence being edited: the opened nested sequence when one is
    /// active (and still exists), else the project's first.
    pub(crate) fn first_sequence(&self) -> Result<&Sequence, String> {
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
    pub(crate) fn sync_player(&mut self) -> Result<(), String> {
        let Some(seq) = self.first_sequence().ok().cloned() else {
            self.player = None;
            return Ok(());
        };
        // (id, original, file to decode video from: a proxy when in use).
        let media: Vec<(MediaId, String, String)> = self
            .project()
            .map(|p| {
                p.media
                    .iter()
                    .map(|m| (m.id, m.path.clone(), self.video_path(m.id, &m.path)))
                    .collect()
            })
            .unwrap_or_default();
        if self.player.is_none() {
            let (mut player, sink) = Player::new(seq.clone());
            player.frames.set_platform(Arc::clone(&self.platform));
            player.frames.set_plugins(self.plugin_host());
            player.samples.set_plugins(self.plugin_host());
            let mut out: Box<dyn AudioOut> = self.platform.open_audio_out();
            out.start(Box::new(sink)).map_err(|e| e.to_string())?;
            self.audio_out = Some(out);
            self.player = Some(player);
        }
        let player = self.player.as_mut().unwrap();
        for (id, path, video_path) in media {
            if player.has_media(id) {
                continue;
            }
            // A missing or unreadable file must not take the whole project down:
            // its clips show the offline slate until it is relinked (MED-05).
            let video = match self.platform.open_decoder(&video_path) {
                Ok(d) => d,
                Err(_) => {
                    self.offline.insert(id);
                    continue;
                }
            };
            self.offline.remove(&id);
            // A sound-only file has no picture to show.
            if video.video_info().is_none() {
                if video.audio_info().is_some() {
                    player
                        .add_media(id, None, Some(video))
                        .map_err(|e| e.to_string())?;
                }
                continue;
            }
            // Audio always comes from the original (proxies are video only).
            let audio = if video_path == path {
                if video.audio_info().is_some() {
                    Some(
                        self.platform
                            .open_decoder(&path)
                            .map_err(|e| e.to_string())?,
                    )
                } else {
                    None
                }
            } else {
                self.platform
                    .open_decoder(&path)
                    .ok()
                    .filter(|d| d.audio_info().is_some())
            };
            player
                .add_media(id, Some(video), audio)
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

    pub(crate) fn clip_ref(
        &self,
        track: &str,
        clip: &str,
    ) -> Result<(Target, ClipId, Clip), String> {
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

    pub(crate) fn playhead(&self) -> Rational {
        self.player
            .as_ref()
            .map(|p| p.transport.position())
            .unwrap_or(Rational::ZERO)
    }
}

#[cfg(test)]
mod tests;
