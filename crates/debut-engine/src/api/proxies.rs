//! Proxy media (MED-05): build 1/2 or 1/4 resolution copies on a background
//! job, and play from them while "use proxies" is on. Proxies are found by
//! their path next to the project plus a completion marker, so they need no
//! undo step and survive reopening the project. Audio and export always use
//! the original files.

use super::*;
use debut_media::proxy;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum JobState {
    Queued,
    Running(f32),
    Done,
    Failed(String),
}

#[derive(Clone, Debug)]
pub(crate) struct ProxyJob {
    state: JobState,
    /// The player has picked up the finished proxy.
    applied: bool,
}

pub(crate) type ProxyJobs = Arc<Mutex<std::collections::HashMap<MediaId, ProxyJob>>>;

/// One media's proxy state for the media panel.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ProxyStatusDto {
    pub media: String,
    /// "none", "queued", "running", "ready" or "failed".
    pub state: String,
    pub progress: f32,
    pub error: Option<String>,
    /// The proxy file in use when ready.
    pub path: Option<String>,
}

/// Transcode `source` into `dest` at 1/`divisor`, then write the marker.
fn build_one(
    platform: &dyn Platform,
    source: &str,
    dest: &str,
    divisor: u8,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<(), String> {
    let mut dec = platform.open_decoder(source).map_err(|e| e.to_string())?;
    dec.select(true, false);
    let info = dec.video_info().ok_or("media has no video stream")?.clone();
    let (width, height) = proxy::proxy_size(info.width, info.height, divisor);
    let mut enc = platform
        .create_encoder(
            dest,
            EncodeSettings {
                width,
                height,
                frame_rate: info.frame_rate,
                crf: 23,
                audio: None,
                encoder: None,
                hdr: None,
                smart: None,
            },
        )
        .map_err(|e| e.to_string())?;
    proxy::build(dec.as_mut(), enc.as_mut(), divisor, progress).map_err(|e| e.to_string())?;
    enc.finish().map_err(|e| e.to_string())?;
    let marker = proxy::Marker {
        source: source.to_string(),
        divisor,
    };
    let json = serde_json::to_vec(&marker).map_err(|e| e.to_string())?;
    platform
        .file_store()
        .write(&proxy::marker_path(dest), &json)
        .map_err(|e| e.to_string())
}

impl Session {
    fn project_path(&self) -> Result<String, String> {
        Ok(self
            .workspace
            .as_ref()
            .ok_or("no project open")?
            .path
            .clone())
    }

    /// The finished proxy of `media` (at `path`), smallest divisor first, if any.
    pub(crate) fn proxy_file(&self, media: MediaId, path: &str) -> Option<String> {
        let project = self.project_path().ok()?;
        proxy::DIVISORS.iter().find_map(|&d| {
            let p = proxy::proxy_path(&project, media, path, d);
            let marker = self.store.read(&proxy::marker_path(&p)).ok()?;
            let m: proxy::Marker = serde_json::from_slice(&marker).ok()?;
            (m.source == path && self.store.exists(&p)).then_some(p)
        })
    }

    /// What the player should decode video from for `media`.
    pub(crate) fn video_path(&self, media: MediaId, path: &str) -> String {
        if self.use_proxies {
            self.proxy_file(media, path)
                .unwrap_or_else(|| path.to_string())
        } else {
            path.to_string()
        }
    }

    /// Queue proxies at 1/`divisor` (2 or 4) for `media`; ones already queued
    /// or running are skipped. One background job works through the list.
    pub fn create_proxies(&mut self, media: Vec<String>, divisor: u8) -> Result<(), String> {
        if !proxy::DIVISORS.contains(&divisor) {
            return Err(format!("proxy size must be 1/{:?}", proxy::DIVISORS));
        }
        let project = self.project_path()?;
        let mut work = Vec::new();
        {
            let mut jobs = self.proxies.lock().unwrap_or_else(|e| e.into_inner());
            for m in media {
                let id = MediaId(parse_id(&m)?);
                let path = self
                    .project()
                    .and_then(|p| p.media.iter().find(|r| r.id == id))
                    .map(|r| r.path.clone())
                    .ok_or("unknown media")?;
                let busy = jobs
                    .get(&id)
                    .is_some_and(|j| matches!(j.state, JobState::Queued | JobState::Running(_)));
                if busy {
                    continue;
                }
                jobs.insert(
                    id,
                    ProxyJob {
                        state: JobState::Queued,
                        applied: false,
                    },
                );
                let dest = proxy::proxy_path(&project, id, &path, divisor);
                work.push((id, path, dest));
            }
        }
        if work.is_empty() {
            return Ok(());
        }
        let (platform, jobs) = (Arc::clone(&self.platform), Arc::clone(&self.proxies));
        let set = move |jobs: &ProxyJobs, id: MediaId, state: JobState| {
            if let Some(j) = jobs.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&id) {
                j.state = state;
            }
        };
        self.platform
            .spawn(
                "debut-proxy",
                Box::new(move || {
                    for (id, source, dest) in work {
                        set(&jobs, id, JobState::Running(0.0));
                        let mut last = 0.0f32;
                        let mut progress = |p: f32| {
                            // Publish in 1% steps; the lock is shared with the UI.
                            if p - last >= 0.01 || p >= 1.0 {
                                last = p;
                                set(&jobs, id, JobState::Running(p));
                            }
                            true
                        };
                        let state = match build_one(
                            platform.as_ref(),
                            &source,
                            &dest,
                            divisor,
                            &mut progress,
                        ) {
                            Ok(()) => JobState::Done,
                            Err(e) => JobState::Failed(e),
                        };
                        set(&jobs, id, state);
                    }
                }),
            )
            .map_err(|e| e.to_string())
    }

    /// Proxy state of every media. A proxy that finished since the last call
    /// is switched into the player when proxies are in use.
    pub fn proxy_status(&mut self) -> Result<Vec<ProxyStatusDto>, String> {
        let media: Vec<(MediaId, String)> = self
            .project()
            .map(|p| p.media.iter().map(|m| (m.id, m.path.clone())).collect())
            .unwrap_or_default();
        let mut fresh = Vec::new();
        let jobs: std::collections::HashMap<MediaId, ProxyJob> = {
            let mut jobs = self.proxies.lock().unwrap_or_else(|e| e.into_inner());
            for (id, j) in jobs.iter_mut() {
                if j.state == JobState::Done && !j.applied {
                    j.applied = true;
                    fresh.push(*id);
                }
            }
            jobs.clone()
        };
        if self.use_proxies && !fresh.is_empty() {
            if let Some(p) = &mut self.player {
                fresh.iter().for_each(|id| p.forget_media(*id));
            }
            self.sync_player()?;
        }
        Ok(media
            .into_iter()
            .map(|(id, path)| {
                let ready = self.proxy_file(id, &path);
                let (state, progress, error) = match jobs.get(&id).map(|j| &j.state) {
                    Some(JobState::Queued) => ("queued", 0.0, None),
                    Some(JobState::Running(p)) => ("running", *p, None),
                    Some(JobState::Failed(e)) if ready.is_none() => {
                        ("failed", 0.0, Some(e.clone()))
                    }
                    _ if ready.is_some() => ("ready", 1.0, None),
                    _ => ("none", 0.0, None),
                };
                ProxyStatusDto {
                    media: id_str(id.0),
                    state: state.into(),
                    progress,
                    error,
                    path: ready,
                }
            })
            .collect())
    }

    pub fn use_proxies(&self) -> bool {
        self.use_proxies
    }

    /// Play video from proxies where they exist (or back from the originals).
    pub fn set_use_proxies(&mut self, on: bool) -> Result<(), String> {
        if self.use_proxies == on {
            return Ok(());
        }
        self.use_proxies = on;
        let media: Vec<MediaId> = self
            .project()
            .map(|p| p.media.iter().map(|m| m.id).collect())
            .unwrap_or_default();
        if let Some(p) = &mut self.player {
            media.iter().for_each(|id| p.forget_media(*id));
        }
        self.sync_player()
    }
}
