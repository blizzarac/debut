//! Transport, preview quality, frame and scope readback (PB-01 .. PB-08).

use super::*;

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

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct HwDecodeDto {
    /// A hardware device opens on this machine (it may still turn streams down).
    pub available: bool,
    pub enabled: bool,
    pub media: Vec<DecodePathDto>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct DecodePathDto {
    pub media: String,
    pub name: String,
    /// "software", "requested" (no frame decoded yet), "hardware" or "fallback".
    pub mode: String,
    /// The device API, or why hardware decoding fell back.
    pub detail: String,
}

#[derive(Serialize)]
pub struct TickDto {
    pub frame: i64,
    pub position: f64,
    pub playing: bool,
    pub changed: bool,
    pub dropped: u64,
    /// Preview resolution in use: 1, 2 or 4 (PB-03).
    pub preview_divisor: u32,
}

/// Playback resolution selector (PB-03).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewQuality {
    Full,
    Half,
    Quarter,
    Auto,
}

impl Session {
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
        if let Some(ws) = self.workspace.as_mut() {
            ws.maybe_autosave(self.platform.now())
                .map_err(|e| e.to_string())?;
        }
        if self.player.is_none() {
            self.sync_player()?;
        }
        self.apply_preview_quality();
        let p = self.player.as_mut().ok_or("no sequence")?;
        let changed = p.tick().map_err(|e| e.to_string())?.is_some();
        let Stats { dropped, .. } = p.stats();
        Ok(TickDto {
            frame: p.transport.current_frame(),
            position: secs(p.transport.position()),
            playing: p.transport.is_playing(),
            changed,
            dropped,
            preview_divisor: p.preview_divisor(),
        })
    }

    /// Decode video on the GPU where a device takes it (NFR-09); the
    /// player's decoders reopen with the new setting.
    pub fn set_hardware_decode(&mut self, on: bool) -> Result<(), String> {
        if self.platform.hardware_decode() == on {
            return Ok(());
        }
        self.platform.set_hardware_decode(on);
        let media: Vec<MediaId> = self
            .project()
            .map(|p| p.media.iter().map(|m| m.id).collect())
            .unwrap_or_default();
        if let Some(p) = &mut self.player {
            media.iter().for_each(|id| p.forget_media(*id));
        }
        self.sync_player()
    }

    /// Whether hardware decoding is available and on, and how each open
    /// media's video is actually being decoded.
    pub fn hardware_decode_status(&self) -> HwDecodeDto {
        let names: std::collections::HashMap<MediaId, String> = self
            .project()
            .map(|p| {
                p.media
                    .iter()
                    .map(|m| {
                        (
                            m.id,
                            m.path.rsplit(['/', '\\']).next().unwrap_or("").to_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut media: Vec<DecodePathDto> = self
            .player
            .as_ref()
            .map(|p| p.frames.decode_paths())
            .unwrap_or_default()
            .into_iter()
            .map(|(id, path)| {
                let (mode, detail) = match path {
                    debut_platform::DecodePath::Software => ("software".to_string(), String::new()),
                    debut_platform::DecodePath::Requested(api) => ("requested".to_string(), api),
                    debut_platform::DecodePath::Hardware(api) => ("hardware".to_string(), api),
                    debut_platform::DecodePath::Fallback { reason, .. } => {
                        ("fallback".to_string(), reason)
                    }
                };
                DecodePathDto {
                    media: id_str(id.0),
                    name: names.get(&id).cloned().unwrap_or_default(),
                    mode,
                    detail,
                }
            })
            .collect();
        media.sort_by(|a, b| a.name.cmp(&b.name));
        HwDecodeDto {
            available: self.platform.capabilities().hardware_decode,
            enabled: self.platform.hardware_decode(),
            media,
        }
    }

    pub fn set_preview_quality(&mut self, q: PreviewQuality) {
        self.preview = q;
        self.frames_since_change = 0;
        self.apply_preview_quality();
    }

    /// Fixed modes set the divisor directly. Auto steps down when a frame costs
    /// more than ~70 % of its display time and back up after a run of cheap ones.
    pub(crate) fn apply_preview_quality(&mut self) {
        let Some(p) = self.player.as_mut() else {
            return;
        };
        let target = match self.preview {
            PreviewQuality::Full => 1,
            PreviewQuality::Half => 2,
            PreviewQuality::Quarter => 4,
            PreviewQuality::Auto => {
                let d = p.preview_divisor();
                let budget = p.transport.frame_rate().frame_duration().as_f64();
                match self.frame_cost.map(|c| c.as_secs_f64()) {
                    Some(cost) if cost > budget * 0.7 && d < 4 => {
                        self.frames_since_change = 0;
                        d * 2
                    }
                    // Stepping up costs ~4x per step; only when there's clear headroom.
                    Some(cost)
                        if cost < budget * 0.12 && d > 1 && self.frames_since_change > 50 =>
                    {
                        self.frames_since_change = 0;
                        d / 2
                    }
                    _ => d,
                }
            }
        };
        p.set_preview_divisor(target);
    }

    /// The current frame as `(width, height, RGBA8 bytes)` at the preview size.
    pub fn frame_pixels(&mut self) -> Result<(u32, u32, Vec<u8>), String> {
        let started = self.platform.now();
        let Session {
            player, backend, ..
        } = self;
        let p = player.as_mut().ok_or("no sequence")?;
        let graph = p.current_graph();
        let (w, h, out) = backend
            .render_rgba8(&graph, &mut p.frames, debut_render::Transfer::Srgb)
            .map_err(|e| e.to_string())?;
        let cost = self.platform.now().saturating_sub(started);
        // Exponential moving average so one slow frame doesn't flip the mode.
        self.frame_cost = Some(match self.frame_cost {
            Some(prev) => prev.mul_f32(0.7) + cost.mul_f32(0.3),
            None => cost,
        });
        self.frames_since_change = self.frames_since_change.saturating_add(1);
        Ok((w, h, out))
    }

    /// Scopes of the current frame (PB-08), packed as `Scopes::to_bytes`.
    pub fn scopes(&mut self) -> Result<Vec<u8>, String> {
        let (w, h, px) = self.frame_pixels()?;
        Ok(debut_render::scopes::compute(&px, w, h).to_bytes())
    }
}
