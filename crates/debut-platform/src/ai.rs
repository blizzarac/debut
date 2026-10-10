//! The on-device AI host (GFX-04): model work runs locally, out of process,
//! and only when the user has switched AI features on. Today that is speech
//! transcription; the trait leaves room for other local models.

use debut_core::Result;

/// A stretch of recognised speech, in seconds from the start of the audio.
#[derive(Clone, Debug, PartialEq)]
pub struct TranscriptSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

pub trait Transcriber: Send + Sync {
    /// Speech in 16 kHz mono samples to timed text, on this machine.
    /// `language` is a code such as "en"; `None` lets the model detect it.
    fn transcribe(&self, samples: &[f32], language: Option<&str>)
        -> Result<Vec<TranscriptSegment>>;
    /// What runs the model (for the settings panel), e.g. its model file.
    fn describe(&self) -> String;
}
