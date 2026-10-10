//! On-device AI (GFX-04): transcription into captions, and searching them.
//! Off until the user switches AI features on; models run locally through
//! the platform's AI host, and audio never leaves the machine.

use super::*;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct AiDto {
    /// The user has switched AI features on.
    pub enabled: bool,
    /// A local transcription engine and model are set up.
    pub available: bool,
    pub backend: Option<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct TranscribeDto {
    pub segments: usize,
    pub captions_added: usize,
    pub backend: String,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct CaptionHit {
    pub id: String,
    pub start: f64,
    pub text: String,
}

/// The engine's working rate for speech models.
const SPEECH_RATE: u32 = 16_000;

impl Session {
    pub fn ai_status(&self) -> AiDto {
        let t = self.platform.transcriber();
        AiDto {
            enabled: self.ai_enabled,
            available: t.is_some(),
            backend: t.map(|t| t.describe()),
        }
    }

    /// Opt in to (or out of) AI features for this session.
    pub fn set_ai_enabled(&mut self, on: bool) {
        self.ai_enabled = on;
    }

    /// Use this local engine and model for transcription.
    pub fn configure_transcriber(&mut self, engine: &str, model: &str) -> Result<AiDto, String> {
        self.platform
            .configure_transcriber(engine, model)
            .map_err(|e| e.to_string())?;
        Ok(self.ai_status())
    }

    /// Transcribe a clip's audio and add its speech as captions over the
    /// clip, in one undo step. `language` is a code ("en", "de"); `None`
    /// lets the model detect it.
    pub fn transcribe_clip(
        &mut self,
        track: &str,
        clip: &str,
        language: Option<String>,
    ) -> Result<TranscribeDto, String> {
        if !self.ai_enabled {
            return Err(
                "AI features are off: switch them on to transcribe (audio stays on this machine)"
                    .into(),
            );
        }
        let transcriber = self
            .platform
            .transcriber()
            .ok_or("no local transcription engine is set up")?;
        let (_, _, c) = self.clip_ref(track, clip)?;
        if c.speed <= Rational::ZERO || !c.ramp.is_empty() {
            return Err(
                "transcribe clips at normal, constant speed (not frozen, reversed or ramped)"
                    .into(),
            );
        }
        let (media, source_in) = c
            .media_at(c.timeline_in)
            .ok_or("this clip has no media to transcribe")?;
        let path = self
            .project()
            .and_then(|p| p.media.iter().find(|m| m.id == media))
            .map(|m| m.path.clone())
            .ok_or("media not found")?;
        let decoder = self
            .platform
            .open_decoder(&path)
            .map_err(|e| e.to_string())?;
        let mut cache = crate::SampleCache::new(SPEECH_RATE);
        cache.add(media, decoder).map_err(|e| e.to_string())?;
        let source_len = c.duration * c.speed;
        let frames = (source_len * Rational::from_int(SPEECH_RATE as i64))
            .round()
            .max(0) as usize;
        let mut buf = Vec::new();
        let channels =
            debut_audio::SampleSource::read(&mut cache, media, source_in, frames, &mut buf)
                .map_err(|e| e.to_string())? as usize;
        let mono: Vec<f32> = buf
            .chunks(channels.max(1))
            .map(|f| f.iter().sum::<f32>() / f.len() as f32)
            .collect();
        let segments = transcriber
            .transcribe(&mono, language.as_deref())
            .map_err(|e| e.to_string())?;

        // Source seconds -> sequence time, kept inside the clip.
        let seq = self.first_sequence()?;
        let (seq_id, fr) = (seq.id, seq.frame_rate);
        let speed = c.speed.as_f64();
        let (clip_in, clip_out) = (secs(c.timeline_in), secs(c.timeline_out()));
        let mut commands = Vec::new();
        for s in &segments {
            let start = (clip_in + s.start / speed).max(clip_in);
            let end = (clip_in + s.end / speed).min(clip_out);
            let (start, end) = (frames_of(start, fr), frames_of(end, fr));
            if end <= start {
                continue;
            }
            commands.push(Command::AddCaption {
                sequence: seq_id,
                caption: Caption::new(self.ids.fresh(), start, end, s.text.clone()),
            });
        }
        let added = commands.len();
        if added > 0 {
            self.exec(Command::Group(commands))?;
        }
        Ok(TranscribeDto {
            segments: segments.len(),
            captions_added: added,
            backend: transcriber.describe(),
        })
    }

    /// Captions whose text contains `query` (any case), in time order.
    pub fn search_captions(&self, query: &str) -> Result<Vec<CaptionHit>, String> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let mut hits: Vec<CaptionHit> = self
            .first_sequence()?
            .captions
            .iter()
            .filter(|c| c.text.to_lowercase().contains(&q))
            .map(|c| CaptionHit {
                id: id_str(c.id.0),
                start: secs(c.start),
                text: c.text.clone(),
            })
            .collect();
        hits.sort_by(|a, b| a.start.total_cmp(&b.start));
        Ok(hits)
    }
}
