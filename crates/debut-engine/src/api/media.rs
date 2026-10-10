//! Media import, listing, relink (MED-05), bins (MED-07), keywords and ratings (MED-08).

use super::*;

#[derive(Serialize)]
pub struct MediaDto {
    pub id: String,
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub frame_rate: [i64; 2],
    pub has_audio: bool,
    /// Bins this media is in (manual membership and smart matches).
    #[serde(default)]
    pub bins: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub rating: u8,
    /// False when the file is missing or unreadable (MED-05); clips show a slate.
    pub online: bool,
    /// Start timecode and reel from the file's tags (MED-04).
    #[serde(default)]
    pub timecode: Option<String>,
    #[serde(default)]
    pub reel: Option<String>,
    /// Variable frame rate: playback holds frames across its gaps (MED-03).
    #[serde(default)]
    pub vfr: bool,
}

#[derive(Serialize)]
pub struct BinDto {
    pub id: String,
    pub name: String,
    pub smart: bool,
    /// The smart bin's rule as "field op value", if it is a smart bin.
    pub filter: Option<String>,
    pub count: usize,
}

/// A smart-bin rule as the app sends it.
#[derive(Clone, Debug, Deserialize)]
pub struct RuleDto {
    pub field: String,
    pub op: String,
    pub value: String,
}

/// How long a still image runs when it is put on the timeline.
pub const STILL_DURATION: Rational = Rational { num: 5, den: 1 };

/// (width, height, duration, has_audio) of an opened file: 0 x 0 for sound
/// only, `STILL_DURATION` for a single image; `None` for neither.
pub(crate) fn media_info(dec: &dyn debut_platform::Decoder) -> Option<(u32, u32, Rational, bool)> {
    let has_audio = dec.audio_info().is_some();
    match (dec.video_info(), dec.audio_info()) {
        (Some(v), _) => {
            let duration = if v.duration == Rational::ZERO {
                STILL_DURATION
            } else {
                v.duration
            };
            Some((v.width, v.height, duration, has_audio))
        }
        (None, Some(a)) => Some((0, 0, a.duration, true)),
        (None, None) => None,
    }
}

impl Session {
    pub fn import_media(&mut self, path: String) -> Result<MediaDto, String> {
        let media = self.probe_media(&path)?;
        let id = media.id;
        let start_timecode = media.metadata.start_timecode;
        let reel = media.metadata.reel.clone();
        let vfr = media.metadata.variable_frame_rate;
        self.exec(Command::AddMedia(media))?;
        let (width, height, duration, has_audio) = self.probed[&id];
        let fr = self
            .project()
            .and_then(|p| p.media.iter().find(|m| m.id == id))
            .and_then(|m| m.metadata.frame_rate)
            .unwrap_or(FrameRate::FPS_25);
        Ok(MediaDto {
            id: id_str(id.0),
            path,
            width,
            height,
            duration: secs(duration),
            frame_rate: [fr.0.num, fr.0.den],
            has_audio,
            bins: Vec::new(),
            keywords: Vec::new(),
            rating: 0,
            online: true,
            timecode: start_timecode.map(|t| t.to_string()),
            reel,
            vfr,
        })
    }

    /// Open a file and describe it as a new `MediaRef` (not yet added).
    pub(crate) fn probe_media(&mut self, path: &str) -> Result<MediaRef, String> {
        let path = path.to_string();
        let dec = self
            .platform
            .open_decoder(&path)
            .map_err(|e| e.to_string())?;
        let info = media_info(dec.as_ref()).ok_or("the file has neither picture nor sound")?;
        let still = dec
            .video_info()
            .is_some_and(|v| v.duration == Rational::ZERO);
        let frame_rate = dec.video_info().map(|v| v.frame_rate);
        let tags = dec.tags();
        let start_timecode = tags.timecode.as_deref().and_then(Timecode::parse);
        self.project_mut()?;
        let id: MediaId = self.ids.fresh();
        let media = MediaRef {
            id,
            path: path.clone(),
            online: true,
            metadata: MediaMetadata {
                frame_rate,
                still,
                audio_channels: dec.audio_info().map(|a| a.channels).unwrap_or(0),
                start_timecode,
                reel: tags.reel.clone(),
                variable_frame_rate: dec.video_info().is_some_and(|v| v.variable_frame_rate),
                camera: tags.camera.clone(),
                ..Default::default()
            },
            proxies: vec![],
            keywords: vec![],
            rating: 0,
        };
        self.probed.insert(id, info);
        Ok(media)
    }

    /// Point `media` at `path` (relink, MED-05): the player reloads it.
    pub fn relink_media(&mut self, media: &str, path: String) -> Result<(), String> {
        let id = MediaId(parse_id(media)?);
        let dec = self
            .platform
            .open_decoder(&path)
            .map_err(|e| format!("cannot open {path}: {e}"))?;
        let info = media_info(dec.as_ref()).ok_or("the file has neither picture nor sound")?;
        self.probed.insert(id, info);
        self.offline.remove(&id);
        self.forget_waveform(id);
        if let Some(p) = &mut self.player {
            p.forget_media(id);
        }
        self.exec(Command::SetMediaPath { media: id, path })
    }

    /// Every media in the project with its bins; probes files not seen yet in
    /// this session (after opening a project) and caches the result.
    pub fn media_list(&mut self) -> Result<Vec<MediaDto>, String> {
        let media: Vec<MediaRef> = self.project().map(|p| p.media.clone()).unwrap_or_default();
        for m in &media {
            if self.probed.contains_key(&m.id) {
                continue;
            }
            match self.platform.open_decoder(&m.path) {
                Ok(dec) => {
                    if let Some(info) = media_info(dec.as_ref()) {
                        self.probed.insert(m.id, info);
                    }
                    self.offline.remove(&m.id);
                }
                Err(_) => {
                    self.offline.insert(m.id);
                }
            }
        }
        let bins = self.project().map(|p| p.bins.clone()).unwrap_or_default();
        Ok(media
            .iter()
            .map(|m| {
                let (w, h, d, a) =
                    self.probed
                        .get(&m.id)
                        .copied()
                        .unwrap_or((0, 0, Rational::ZERO, false));
                let fr = m.metadata.frame_rate.unwrap_or(FrameRate::FPS_25);
                MediaDto {
                    id: id_str(m.id.0),
                    path: m.path.clone(),
                    width: w,
                    height: h,
                    duration: secs(d),
                    frame_rate: [fr.0.num, fr.0.den],
                    has_audio: a,
                    bins: bins
                        .iter()
                        .filter(|b| b.contains(m))
                        .map(|b| id_str(b.id.0))
                        .collect(),
                    keywords: m.keywords.clone(),
                    rating: m.rating,
                    online: !self.offline.contains(&m.id) && self.store.exists(&m.path),
                    timecode: m.metadata.start_timecode.map(|t| t.to_string()),
                    reel: m.metadata.reel.clone(),
                    vfr: m.metadata.variable_frame_rate,
                }
            })
            .collect())
    }

    pub fn bins(&self) -> Result<Vec<BinDto>, String> {
        let p = self.project().ok_or("no project open")?;
        Ok(p.bins
            .iter()
            .map(|b| BinDto {
                id: id_str(b.id.0),
                name: b.name.clone(),
                smart: b.is_smart(),
                filter: match &b.kind {
                    debut_project::BinKind::Smart { rules } => Some(
                        rules
                            .iter()
                            .map(|r| format!("{} {} {}", r.field, r.op, r.value))
                            .collect::<Vec<_>>()
                            .join(" and "),
                    ),
                    _ => None,
                },
                count: p.media.iter().filter(|m| b.contains(m)).count(),
            })
            .collect())
    }

    /// New bin; with `filter` it is a smart bin matching file names containing it.
    pub fn add_bin(&mut self, name: String, filter: Option<String>) -> Result<String, String> {
        match filter.filter(|f| !f.trim().is_empty()) {
            Some(f) => self.add_smart_bin(
                name,
                RuleDto {
                    field: "name".into(),
                    op: "contains".into(),
                    value: f.trim().to_string(),
                },
            ),
            None => {
                let id: BinId = self.ids.fresh();
                self.exec(Command::AddBin(Bin::manual(id, name)))?;
                Ok(id_str(id.0))
            }
        }
    }

    /// New smart bin with one rule (name/path/keyword contains, rating gte, ...).
    pub fn add_smart_bin(&mut self, name: String, rule: RuleDto) -> Result<String, String> {
        const FIELDS: &[&str] = &[
            "name", "path", "reel", "camera", "audio", "keyword", "rating",
        ];
        if !FIELDS.contains(&rule.field.as_str()) {
            return Err(format!("unknown rule field {}", rule.field));
        }
        let id: BinId = self.ids.fresh();
        let bin = Bin {
            id,
            name,
            kind: debut_project::BinKind::Smart {
                rules: vec![debut_project::SmartRule {
                    field: rule.field,
                    op: rule.op,
                    value: rule.value,
                }],
            },
            items: Vec::new(),
        };
        self.exec(Command::AddBin(bin))?;
        Ok(id_str(id.0))
    }

    /// Keywords (comma separated or a list) and a 0..=5 rating for a media.
    pub fn set_media_tags(
        &mut self,
        media: &str,
        keywords: Vec<String>,
        rating: u8,
    ) -> Result<(), String> {
        self.exec(Command::SetMediaTags {
            media: MediaId(parse_id(media)?),
            keywords,
            rating,
        })
    }

    pub fn rename_bin(&mut self, id: &str, name: String) -> Result<(), String> {
        self.exec(Command::RenameBin {
            id: BinId(parse_id(id)?),
            name,
        })
    }

    pub fn remove_bin(&mut self, id: &str) -> Result<(), String> {
        self.exec(Command::RemoveBin(BinId(parse_id(id)?)))
    }

    /// Put media in a manual bin (or in none).
    pub fn assign_media(&mut self, media: &str, bin: Option<String>) -> Result<(), String> {
        let bin = match bin {
            Some(b) => Some(BinId(parse_id(&b)?)),
            None => None,
        };
        self.exec(Command::AssignMedia {
            media: MediaId(parse_id(media)?),
            bin,
        })
    }
}
