//! Captions, caption settings and subtitle files (GFX-05, GFX-06).

use super::*;

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

pub(crate) fn caption_dto(c: &Caption) -> CaptionDto {
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
