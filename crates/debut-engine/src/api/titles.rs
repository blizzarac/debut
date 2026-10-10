//! Title clips and title templates (GFX-01, GFX-02).

use super::*;

/// Default length of a freshly added title.
pub(crate) const TITLE_SECONDS: i64 = 5;

#[derive(Serialize)]
pub struct TemplateDto {
    pub id: String,
    pub name: String,
    pub description: String,
    /// True for a project-saved template (removable), false for a built-in.
    pub saved: bool,
}

impl Session {
    /// Add a 5 s title clip at `at` seconds on the topmost video track with room
    /// for it, adding a video track above the others when none has (GFX-01).
    pub fn add_title(&mut self, at: f64, text: String) -> Result<String, String> {
        self.add_title_from(at, text, None)
    }

    /// Title templates (GFX-02) the app can offer: built-ins, then the
    /// project's saved ones (ids prefixed `saved:`).
    pub fn title_templates(&self) -> Vec<TemplateDto> {
        let mut out: Vec<TemplateDto> = debut_graphics::TEMPLATES
            .iter()
            .map(|t| TemplateDto {
                id: t.id.to_string(),
                name: t.name.to_string(),
                description: t.description.to_string(),
                saved: false,
            })
            .collect();
        if let Some(p) = self.project() {
            out.extend(p.title_templates.iter().map(|t| TemplateDto {
                id: format!("saved:{}", id_str(t.id.0)),
                name: t.name.clone(),
                description: format!(
                    "Saved: {} px {}{}",
                    t.style.size_px,
                    t.style.font,
                    if t.effects.is_empty() {
                        ""
                    } else {
                        ", animated"
                    }
                ),
                saved: true,
            }));
        }
        out
    }

    /// Save a title clip's style and effect stack as a named template.
    pub fn save_title_template(
        &mut self,
        track: &str,
        clip: &str,
        name: String,
    ) -> Result<String, String> {
        let (_, _, c) = self.clip_ref(track, clip)?;
        let ClipSource::Title(title) = &c.source else {
            return Err("not a title clip".into());
        };
        let template = SavedTitleTemplate {
            id: self.ids.fresh(),
            name,
            style: title.style.clone(),
            effects: c.effects.clone(),
        };
        let id = format!("saved:{}", id_str(template.id.0));
        self.exec(Command::AddTitleTemplate(template))?;
        Ok(id)
    }

    pub fn remove_title_template(&mut self, id: &str) -> Result<(), String> {
        let raw = id
            .strip_prefix("saved:")
            .ok_or("built-in templates cannot be removed")?;
        self.exec(Command::RemoveTitleTemplate(TemplateId(parse_id(raw)?)))
    }

    /// `add_title` with a template: style and animated Transform sized for
    /// this sequence. `None` is the plain default title.
    pub fn add_title_from(
        &mut self,
        at: f64,
        text: String,
        template: Option<String>,
    ) -> Result<String, String> {
        let size = {
            let seq = self.first_sequence()?;
            (seq.width, seq.height)
        };
        let duration = Rational::from_int(TITLE_SECONDS);
        let (title, effects) = match template.as_deref() {
            None => (
                Title {
                    text,
                    style: TitleStyle::default(),
                },
                Vec::new(),
            ),
            Some(id) if id.starts_with("saved:") => {
                let raw = TemplateId(parse_id(&id["saved:".len()..])?);
                let t = self
                    .project()
                    .and_then(|p| p.title_templates.iter().find(|t| t.id == raw))
                    .ok_or_else(|| format!("unknown saved template {id}"))?;
                (
                    Title {
                        text,
                        style: t.style.clone(),
                    },
                    t.effects.clone(),
                )
            }
            Some(id) => {
                let built =
                    debut_graphics::build_title_template(id, &text, size.0, size.1, duration)
                        .ok_or_else(|| format!("unknown title template {id}"))?;
                (built.title, built.effects)
            }
        };
        self.add_generated(at, ClipSource::Title(title), effects)
    }

    /// Put a 5 s generated clip (title, shape) at `at` seconds on the topmost
    /// video track with room for it, adding a video track above the others
    /// when none has. One undo step; returns the clip id.
    pub(crate) fn add_generated(
        &mut self,
        at: f64,
        source: ClipSource,
        effects: Vec<Effect>,
    ) -> Result<String, String> {
        let (seq_id, fr, free_track, above_video) = {
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
            // A new track goes right above the top video track: later tracks
            // composite on top, and it stays grouped with the video tracks.
            let above_video = seq
                .tracks
                .iter()
                .rposition(|t| t.kind == TrackKind::Video)
                .map_or(seq.tracks.len(), |i| i + 1);
            (seq.id, seq.frame_rate, free, above_video)
        };
        let start = frames_of(at, fr);
        let duration = Rational::from_int(TITLE_SECONDS);
        let mut clip = Clip::new(self.ids.fresh(), source, start, duration, Rational::ZERO);
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
}
