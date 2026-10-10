//! Local transcription through whisper.cpp's command-line tool (GFX-04).
//! The audio goes to a temporary WAV file and `whisper-cli` writes its JSON
//! next to it; nothing leaves the machine. Running the tool as a separate
//! process keeps the model's memory and any crash out of the app, and works
//! with whichever whisper.cpp build (CPU, CUDA, Metal) the user installed.

use debut_core::{Error, Result};
use debut_platform::ai::{Transcriber, TranscriptSegment};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

pub const CLI_VAR: &str = "DEBUT_WHISPER_CLI";
pub const MODEL_VAR: &str = "DEBUT_WHISPER_MODEL";

pub struct WhisperCli {
    exe: PathBuf,
    model: PathBuf,
}

impl WhisperCli {
    pub fn new(exe: impl Into<PathBuf>, model: impl Into<PathBuf>) -> Result<Self> {
        let (exe, model) = (exe.into(), model.into());
        if !exe.is_file() {
            return Err(Error::NotFound(format!(
                "no whisper tool at {}",
                exe.display()
            )));
        }
        if !model.is_file() {
            return Err(Error::NotFound(format!("no model at {}", model.display())));
        }
        Ok(Self { exe, model })
    }

    /// `DEBUT_WHISPER_CLI` (else `whisper-cli` on the PATH) with
    /// `DEBUT_WHISPER_MODEL` (else the first `ggml-*.bin` in debut's model
    /// folder); `None` unless both exist.
    pub fn from_environment() -> Option<Self> {
        let exe = std::env::var_os(CLI_VAR)
            .map(PathBuf::from)
            .or_else(|| on_path("whisper-cli"))?;
        let model = std::env::var_os(MODEL_VAR)
            .map(PathBuf::from)
            .or_else(first_model)?;
        Self::new(exe, model).ok()
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// debut's model folder: `~/.local/share/debut/models` (XDG), or
/// `~/Library/Application Support/debut/models` on macOS.
pub fn model_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME")?;
        return Some(PathBuf::from(home).join("Library/Application Support/debut/models"));
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("debut/models"))
}

fn first_model() -> Option<PathBuf> {
    let mut models: Vec<PathBuf> = std::fs::read_dir(model_dir()?)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
            name.is_some_and(|n| n.starts_with("ggml-") && n.ends_with(".bin"))
        })
        .collect();
    models.sort();
    models.into_iter().next()
}

/// 16-bit PCM mono WAV at `rate`.
fn wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let data = samples.len() as u32 * 2;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
    }
    out
}

/// Segments from whisper-cli's `-oj` output (offsets in milliseconds).
pub fn parse_json(text: &str) -> Result<Vec<TranscriptSegment>> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| Error::Other(format!("whisper output: {e}")))?;
    let list = v["transcription"]
        .as_array()
        .ok_or_else(|| Error::Other("whisper output has no transcription".into()))?;
    Ok(list
        .iter()
        .filter_map(|seg| {
            let from = seg["offsets"]["from"].as_f64()?;
            let to = seg["offsets"]["to"].as_f64()?;
            let text = seg["text"].as_str()?.trim().to_string();
            (!text.is_empty()).then_some(TranscriptSegment {
                start: from / 1000.0,
                end: to / 1000.0,
                text,
            })
        })
        .collect())
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Transcriber for WhisperCli {
    fn transcribe(
        &self,
        samples: &[f32],
        language: Option<&str>,
    ) -> Result<Vec<TranscriptSegment>> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = TempDir(std::env::temp_dir().join(format!(
            "debut-whisper-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        std::fs::create_dir_all(&dir.0).map_err(|e| Error::Other(e.to_string()))?;
        let input = dir.0.join("audio.wav");
        std::fs::write(&input, wav(samples, 16_000)).map_err(|e| Error::Other(e.to_string()))?;
        let base = dir.0.join("transcript");
        let threads = std::thread::available_parallelism()
            .map(|n| n.get().min(8))
            .unwrap_or(4);
        let out = Command::new(&self.exe)
            .arg("-m")
            .arg(&self.model)
            .arg("-f")
            .arg(&input)
            .args(["-oj", "-np", "-t", &threads.to_string()])
            .args(["-l", language.unwrap_or("auto")])
            .arg("-of")
            .arg(&base)
            .output()
            .map_err(|e| Error::Other(format!("cannot run {}: {e}", self.exe.display())))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let tail: Vec<&str> = err.lines().rev().take(3).collect();
            return Err(Error::Other(format!(
                "whisper failed ({}): {}",
                out.status,
                tail.into_iter().rev().collect::<Vec<_>>().join(" / ")
            )));
        }
        let json = std::fs::read_to_string(base.with_extension("json"))
            .map_err(|e| Error::Other(format!("whisper wrote no transcript: {e}")))?;
        parse_json(&json)
    }

    fn describe(&self) -> String {
        format!(
            "whisper.cpp ({}) with {}",
            file(&self.exe),
            file(&self.model)
        )
    }
}

fn file(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_whisper_cli_json() {
        let text = r#"{"systeminfo":"x","transcription":[
            {"timestamps":{"from":"00:00:00,000","to":"00:00:01,500"},"offsets":{"from":0,"to":1500},"text":" Hello there."},
            {"timestamps":{"from":"00:00:01,500","to":"00:00:02,000"},"offsets":{"from":1500,"to":2000},"text":"  "},
            {"timestamps":{"from":"00:00:02,000","to":"00:00:03,250"},"offsets":{"from":2000,"to":3250},"text":" General Kenobi."}]}"#;
        let segs = parse_json(text).unwrap();
        assert_eq!(
            segs,
            vec![
                TranscriptSegment {
                    start: 0.0,
                    end: 1.5,
                    text: "Hello there.".into()
                },
                TranscriptSegment {
                    start: 2.0,
                    end: 3.25,
                    text: "General Kenobi.".into()
                },
            ]
        );
    }

    /// Against a real whisper.cpp build: set DEBUT_WHISPER_CLI and
    /// DEBUT_WHISPER_MODEL and run with `--ignored`.
    #[test]
    #[ignore]
    fn runs_a_real_whisper_cli() {
        let w = WhisperCli::from_environment().expect("DEBUT_WHISPER_CLI and DEBUT_WHISPER_MODEL");
        let tone: Vec<f32> = (0..32_000)
            .map(|i| (i as f32 * 0.05).sin() * 0.25)
            .collect();
        let segments = w.transcribe(&tone, Some("en")).unwrap();
        eprintln!("{} -> {segments:?}", w.describe());
    }

    #[test]
    fn writes_16k_mono_wav() {
        let w = wav(&[0.0, 1.0, -1.0], 16_000);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), 16_000);
        assert_eq!(w.len(), 44 + 6);
        assert_eq!(i16::from_le_bytes([w[46], w[47]]), 32767);
    }
}
