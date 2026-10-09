//! IPC commands. All project mutation goes through the command history; the
//! player is rebuilt from the project's sequence after every edit.

use debut_command::{Command, History, Journal, MemoryJournal, Target};
use debut_core::{ClipId, FrameRate, IdGen, MediaId, Rational, TrackId};
use debut_engine::{Player, Stats};
use debut_export::job::to_rgba8;
use debut_platform::audio_out::AudioOut;
use debut_platform::Decoder;
use debut_platform_native::audio_out::{CpalAudioOut, SilentAudioOut};
use debut_platform_native::codec::FfmpegDecoder;
use debut_project::media_ref::{MediaMetadata, MediaRef};
use debut_project::{schema, Clip, ClipSource, Project, Sequence, Track, TrackKind};
use debut_render::AnyBackend;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::State;

pub struct Session {
    project: Option<Project>,
    history: History,
    journal: MemoryJournal,
    ids: IdGen,
    player: Option<Player>,
    audio_out: Option<Box<dyn AudioOut>>,
    backend: AnyBackend,
    /// Media probed so far: id -> (width, height, duration, has_audio).
    probed: std::collections::HashMap<MediaId, (u32, u32, Rational, bool)>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            project: None,
            history: History::default(),
            journal: MemoryJournal::default(),
            ids: IdGen::random(),
            player: None,
            audio_out: None,
            backend: AnyBackend::detect(),
            probed: Default::default(),
        }
    }

    fn project_mut(&mut self) -> Result<&mut Project, String> {
        self.project
            .as_mut()
            .ok_or_else(|| "no project open".to_string())
    }

    fn exec(&mut self, cmd: Command) -> Result<(), String> {
        let Session {
            project,
            history,
            journal,
            ..
        } = self;
        let p = project.as_mut().ok_or("no project open")?;
        history
            .execute(p, cmd, journal as &mut dyn Journal)
            .map_err(|e| e.to_string())?;
        self.sync_player()
    }

    fn first_sequence(&self) -> Result<&Sequence, String> {
        self.project
            .as_ref()
            .and_then(|p| p.sequences.first())
            .ok_or_else(|| "no sequence".to_string())
    }

    /// Rebuild the player's view of the sequence after an edit, keeping transport
    /// state; create it on first use.
    fn sync_player(&mut self) -> Result<(), String> {
        let Some(seq) = self
            .project
            .as_ref()
            .and_then(|p| p.sequences.first())
            .cloned()
        else {
            self.player = None;
            return Ok(());
        };
        let media: Vec<(MediaId, String)> = self
            .project
            .as_ref()
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

#[tauri::command]
pub fn new_project(state: State<'_, Shared>, name: String) {
    let mut s = lock(&state);
    let id = s.ids.fresh();
    s.project = Some(Project::new(id, name));
    s.history = History::default();
    s.journal = MemoryJournal::default();
    s.player = None;
}

#[tauri::command]
pub fn open_project(state: State<'_, Shared>, json: String) -> Result<(), String> {
    let project = schema::from_json(&json).map_err(|e| e.to_string())?;
    let mut s = lock(&state);
    s.project = Some(project);
    s.history = History::default();
    s.journal = MemoryJournal::default();
    s.player = None;
    s.sync_player()
}

#[tauri::command]
pub fn project_json(state: State<'_, Shared>) -> Result<String, String> {
    let s = lock(&state);
    let p = s.project.as_ref().ok_or("no project open")?;
    schema::to_json(p).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn execute(state: State<'_, Shared>, command_json: String) -> Result<(), String> {
    let cmd: Command = serde_json::from_str(&command_json).map_err(|e| e.to_string())?;
    lock(&state).exec(cmd)
}

#[tauri::command]
pub fn undo(state: State<'_, Shared>) -> Result<bool, String> {
    let mut s = lock(&state);
    let Session {
        project,
        history,
        journal,
        ..
    } = &mut *s;
    let p = project.as_mut().ok_or("no project open")?;
    let r = history
        .undo(p, journal as &mut dyn Journal)
        .map_err(|e| e.to_string())?;
    s.sync_player()?;
    Ok(r)
}

#[tauri::command]
pub fn redo(state: State<'_, Shared>) -> Result<bool, String> {
    let mut s = lock(&state);
    let Session {
        project,
        history,
        journal,
        ..
    } = &mut *s;
    let p = project.as_mut().ok_or("no project open")?;
    let r = history
        .redo(p, journal as &mut dyn Journal)
        .map_err(|e| e.to_string())?;
    s.sync_player()?;
    Ok(r)
}

#[tauri::command]
pub fn can_undo(state: State<'_, Shared>) -> bool {
    lock(&state).history.can_undo()
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
                .project
                .as_ref()
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
        let Session { project, ids, .. } = self;
        let project = project.as_ref().ok_or("no project open")?;
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
        let mut s = Session::new();
        let id = s.ids.fresh();
        s.project = Some(Project::new(id, "t"));

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
        let Session {
            project,
            history,
            journal,
            ..
        } = &mut s;
        history
            .undo(project.as_mut().unwrap(), journal as &mut dyn Journal)
            .unwrap();
        s.sync_player().unwrap();
        assert_eq!(
            sequence_dto(s.first_sequence().unwrap()).tracks[0].clips[0].duration,
            2.0
        );
        assert_eq!(s.player.as_ref().unwrap().sequence.duration().as_f64(), 2.4);
    }
}
