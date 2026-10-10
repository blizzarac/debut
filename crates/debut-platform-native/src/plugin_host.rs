//! OpenFX and CLAP plugins, run in the `debut-plugin-host` helper process
//! (FX-15, AUD-09, NFR-07). A crash kills only the helper: the call that hit
//! it fails with a message, and the next call starts a fresh helper.
//!
//! Video and audio each get their own helper, so a slow filter never holds up
//! audio. Scanning loads one binary per request; a binary that crashes the
//! helper is reported as a problem and the scan goes on.

use debut_core::{Error, Result};
use debut_platform::plugin_host::{
    AudioJob, PluginHost, PluginProblem, Reply, Request, ScanResult, VideoJob,
};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

/// The environment variable naming extra plugin directories (`:`-separated).
pub const PLUGIN_PATH_VAR: &str = "DEBUT_PLUGIN_PATH";
/// The environment variable naming the helper executable.
pub const HELPER_VAR: &str = "DEBUT_PLUGIN_HOST";

struct Helper {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct NativePluginHost {
    helper: PathBuf,
    dirs: Vec<String>,
    video: Mutex<Option<Helper>>,
    audio: Mutex<Option<Helper>>,
    scan: Mutex<Option<Helper>>,
}

impl NativePluginHost {
    pub fn new(helper: PathBuf, dirs: Vec<String>) -> Self {
        Self {
            helper,
            dirs,
            video: Mutex::new(None),
            audio: Mutex::new(None),
            scan: Mutex::new(None),
        }
    }

    /// The helper next to the running executable (or named by
    /// `DEBUT_PLUGIN_HOST`), searching `DEBUT_PLUGIN_PATH` and the standard
    /// OpenFX and CLAP locations; `None` when there is no helper.
    pub fn from_environment() -> Option<Self> {
        let helper = std::env::var_os(HELPER_VAR)
            .map(PathBuf::from)
            .or_else(|| {
                let exe = std::env::current_exe().ok()?;
                let name = format!("debut-plugin-host{}", std::env::consts::EXE_SUFFIX);
                Some(exe.parent()?.join(name))
            })
            .filter(|p| p.is_file())?;
        Some(Self::new(helper, default_dirs()))
    }

    pub fn helper(&self) -> &Path {
        &self.helper
    }

    fn spawn(&self) -> Result<Helper> {
        let mut child = Command::new(&self.helper)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| {
                Error::Unsupported(format!(
                    "cannot start the plugin host {}: {e}",
                    self.helper.display()
                ))
            })?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = BufReader::new(child.stdout.take().expect("piped"));
        Ok(Helper {
            child,
            stdin,
            stdout,
        })
    }

    /// Send one request (with `payload`) and read the reply; the samples a
    /// `Processed` reply carries come back in `payload`. On any pipe failure
    /// the helper is dropped (it crashed) and the next call starts another.
    fn call(
        &self,
        slot: &Mutex<Option<Helper>>,
        req: &Request,
        payload: &mut Vec<u8>,
    ) -> Result<Reply> {
        let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(self.spawn()?);
        }
        let helper = guard.as_mut().expect("just spawned");
        match exchange(helper, req, payload) {
            Ok(reply) => Ok(reply),
            Err(e) => {
                let status = helper
                    .child
                    .try_wait()
                    .ok()
                    .flatten()
                    .map(|s| format!(" ({s})"))
                    .unwrap_or_default();
                *guard = None;
                Err(Error::Other(format!(
                    "the plugin host stopped{status}: {e}"
                )))
            }
        }
    }

    fn run(&self, slot: &Mutex<Option<Helper>>, req: Request, samples: &mut [f32]) -> Result<()> {
        let mut payload: Vec<u8> = samples.iter().flat_map(|s| s.to_ne_bytes()).collect();
        match self.call(slot, &req, &mut payload)? {
            Reply::Processed { bytes } if bytes == samples.len() * 4 => {
                for (s, b) in samples.iter_mut().zip(payload.chunks_exact(4)) {
                    *s = f32::from_ne_bytes([b[0], b[1], b[2], b[3]]);
                }
                Ok(())
            }
            Reply::Failed { error } => Err(Error::InvalidArgument(error)),
            other => Err(Error::Other(format!(
                "unexpected plugin host reply {other:?}"
            ))),
        }
    }
}

fn exchange(h: &mut Helper, req: &Request, payload: &mut Vec<u8>) -> std::io::Result<Reply> {
    let mut line = serde_json::to_string(req).map_err(std::io::Error::other)?;
    line.push('\n');
    h.stdin.write_all(line.as_bytes())?;
    h.stdin.write_all(payload)?;
    h.stdin.flush()?;
    let mut text = String::new();
    if h.stdout.read_line(&mut text)? == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "no reply",
        ));
    }
    let reply: Reply = serde_json::from_str(text.trim()).map_err(std::io::Error::other)?;
    if let Reply::Processed { bytes } = reply {
        payload.resize(bytes, 0);
        h.stdout.read_exact(payload)?;
    }
    Ok(reply)
}

/// `DEBUT_PLUGIN_PATH`, then the standard locations: `/usr/OFX/Plugins`
/// (OpenFX), `~/.clap` and `/usr/lib/clap` (CLAP); macOS adds its library folders.
pub fn default_dirs() -> Vec<String> {
    let mut dirs: Vec<String> = std::env::var(PLUGIN_PATH_VAR)
        .map(|v| {
            v.split(':')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();
    if cfg!(target_os = "macos") {
        dirs.push("/Library/OFX/Plugins".into());
        dirs.push(format!("{home}/Library/Audio/Plug-Ins/CLAP"));
        dirs.push("/Library/Audio/Plug-Ins/CLAP".into());
    } else {
        dirs.push("/usr/OFX/Plugins".into());
        dirs.push(format!("{home}/.clap"));
        dirs.push("/usr/lib/clap".into());
    }
    dirs
}

impl PluginHost for NativePluginHost {
    fn scan(&self) -> Result<ScanResult> {
        let files = match self.call(
            &self.scan,
            &Request::List {
                dirs: self.dirs.clone(),
            },
            &mut Vec::new(),
        )? {
            Reply::Listed { files } => files,
            other => {
                return Err(Error::Other(format!(
                    "unexpected plugin host reply {other:?}"
                )))
            }
        };
        let mut result = ScanResult::default();
        for (kind, path) in files {
            let req = Request::Describe {
                kind,
                path: path.clone(),
            };
            match self.call(&self.scan, &req, &mut Vec::new()) {
                Ok(Reply::Described { plugins }) => result.plugins.extend(plugins),
                Ok(Reply::Failed { error }) => result.problems.push(PluginProblem { path, error }),
                Ok(other) => result.problems.push(PluginProblem {
                    path,
                    error: format!("unexpected reply {other:?}"),
                }),
                Err(e) => result.problems.push(PluginProblem {
                    path,
                    error: e.to_string(),
                }),
            }
        }
        // Scanning loaded every binary: start clean.
        *self.scan.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(result)
    }

    fn process_video(&self, job: &VideoJob, rgba: &mut [f32]) -> Result<()> {
        let req = Request::Video {
            job: job.clone(),
            bytes: rgba.len() * 4,
        };
        self.run(&self.video, req, rgba)
    }

    fn process_audio(&self, job: &AudioJob, samples: &mut [f32]) -> Result<()> {
        let req = Request::Audio {
            job: job.clone(),
            bytes: samples.len() * 4,
        };
        self.run(&self.audio, req, samples)
    }

    fn release(&self, instance: u64) {
        let alive = self.audio.lock().map(|g| g.is_some()).unwrap_or(false);
        if alive {
            let _ = self.call(&self.audio, &Request::Release { instance }, &mut Vec::new());
        }
    }
}
