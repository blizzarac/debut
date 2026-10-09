//! Clip effect stacks, masks, keying and tracking (FX-01 .. FX-06).

use super::*;

#[derive(Serialize)]
pub struct ParamDto {
    pub name: Param,
    /// Value at the playhead (clip-local evaluation).
    pub value: f64,
    /// More than one keyframe, or a single non-held key.
    pub animated: bool,
}

#[derive(Serialize)]
pub struct EffectDto {
    pub index: usize,
    pub kind: String,
    pub params: Vec<ParamDto>,
    /// Non-animated options (mask shape/invert, key colour).
    pub options: EffectOptions,
}

/// The non-keyframed knobs of an effect; every field optional so one struct
/// serves both reading and partial updates.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct EffectOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<MaskShape>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invert: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
    /// Polygon mask vertices in sequence pixels from the frame centre.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<[f32; 2]>>,
    /// Bézier handles per point, `[in_x, in_y, out_x, out_y]` relative to it;
    /// empty = straight edges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handles: Option<Vec<[f32; 4]>>,
    /// Setting only: true computes even handles for a smooth curve through
    /// the points, false makes every point a corner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smooth: Option<bool>,
}

pub(crate) fn effect_options(e: &Effect) -> EffectOptions {
    match e {
        Effect::Mask(m) => EffectOptions {
            shape: Some(m.shape),
            invert: Some(m.invert),
            color: None,
            points: Some(m.points.clone()),
            handles: Some(m.handles.clone()),
            smooth: None,
        },
        Effect::ChromaKey(k) => EffectOptions {
            color: Some(k.color),
            ..Default::default()
        },
        _ => EffectOptions::default(),
    }
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct TrackResultDto {
    pub keys: usize,
    /// Weakest normalised match met, 1 = identical; below 0.5 the track stopped.
    pub weakest_match: f32,
}

impl Session {
    /// The clip's effects with each parameter evaluated at the playhead.
    pub fn clip_effects(&self, track: &str, clip: &str) -> Result<Vec<EffectDto>, String> {
        let (_, _, c) = self.clip_ref(track, clip)?;
        let local = self.playhead() - c.timeline_in;
        Ok(c.effects
            .iter()
            .enumerate()
            .map(|(index, e)| EffectDto {
                index,
                kind: e.kind().to_string(),
                options: effect_options(e),
                params: e
                    .params()
                    .iter()
                    .map(|&p| {
                        let curve = e.curve(p).unwrap();
                        ParamDto {
                            name: p,
                            value: curve.eval(local),
                            animated: curve.keys().len() > 1,
                        }
                    })
                    .collect(),
            })
            .collect())
    }

    pub fn add_effect(&mut self, track: &str, clip: &str, kind: &str) -> Result<(), String> {
        let (target, clip_id, _) = self.clip_ref(track, clip)?;
        let effect = match kind {
            "transform" => Effect::Transform(TransformFx::default()),
            "grade" => Effect::Grade(GradeFx::default()),
            "mask" => Effect::Mask(MaskFx::default()),
            "key" => Effect::ChromaKey(KeyFx::default()),
            other => return Err(format!("unknown effect kind {other}")),
        };
        self.exec(Command::AddEffect {
            target,
            clip: clip_id,
            effect,
            index: None,
        })
    }

    /// Track the picture under a mask effect from the playhead for `seconds`
    /// (FX-06) and keyframe the mask's position to follow it. Returns the
    /// number of keys written and the weakest match quality met; stops early
    /// when the match drops below `MIN_MATCH`.
    pub fn track_mask(
        &mut self,
        track: &str,
        clip: &str,
        effect: usize,
        seconds: f64,
    ) -> Result<TrackResultDto, String> {
        const MIN_MATCH: f32 = 0.5;
        let (target, clip_id, c) = self.clip_ref(track, clip)?;
        let (seq_w, seq_h, fr) = {
            let s = self.first_sequence()?;
            (s.width as f32, s.height as f32, s.frame_rate)
        };
        let Some(Effect::Mask(m)) = c.effects.get(effect) else {
            return Err("not a mask effect".into());
        };
        let start = fr.snap(self.playhead()).max(c.timeline_in);
        let end = (start + frames_of(seconds, fr)).min(c.timeline_out());
        if end <= start {
            return Err("nothing to track: move the playhead inside the clip".into());
        }
        let (media, _) = c.media_at(start).ok_or("only media clips can be tracked")?;
        let (sw, sh) = self
            .probed
            .get(&media)
            .map(|p| (p.0 as f32, p.1 as f32))
            .ok_or("media not probed")?;
        // Sequence <-> source frame coordinates through the fit used by compose.
        let fit = (seq_w / sw).min(seq_h / sh);
        let to_src = |x: f32, y: f32| [x / fit + sw * 0.5, y / fit + sh * 0.5];
        let to_seq = |p: [f32; 2]| [(p[0] - sw * 0.5) * fit, (p[1] - sh * 0.5) * fit];
        let local0 = start - c.timeline_in;
        let center = to_src(m.x.eval(local0) as f32, m.y.eval(local0) as f32);
        let size = ((m.width.eval(local0).min(m.height.eval(local0)) as f32 / fit) * 0.5)
            .clamp(8.0, 64.0) as u32;
        let search = (size / 2).max(4);

        if self.player.is_none() {
            self.sync_player()?;
        }
        let player = self.player.as_mut().ok_or("no player")?;
        use debut_render::FrameProvider;
        let mut t = start;
        let (w, h, px) = player
            .frames
            .frame(media, c.source_at(t))
            .map_err(|e| e.to_string())?;
        let mut tracker = debut_render::Tracker::new(&px, w, h, center, size, search);
        let (mut cx, mut cy) = (m.x.clone(), m.y.clone());
        let mut keys = 0usize;
        let mut weakest = 1.0f32;
        let set = |curve: &mut debut_core::Curve, at: Rational, v: f32| {
            curve.set(at, v as f64, debut_core::Interp::Linear);
        };
        let first = to_seq(center);
        set(&mut cx, local0, first[0]);
        set(&mut cy, local0, first[1]);
        keys += 1;
        loop {
            t += fr.frame_duration();
            if t >= end {
                break;
            }
            let (w, h, px) = player
                .frames
                .frame(media, c.source_at(t))
                .map_err(|e| e.to_string())?;
            let (pos, score) = tracker.step(&px, w, h);
            weakest = weakest.min(score);
            if score < MIN_MATCH {
                break;
            }
            let p = to_seq(pos);
            let local = t - c.timeline_in;
            set(&mut cx, local, p[0]);
            set(&mut cy, local, p[1]);
            keys += 1;
        }
        self.exec(Command::Group(vec![
            Command::SetCurve {
                target,
                clip: clip_id,
                effect,
                param: Param::MaskX,
                curve: cx,
            },
            Command::SetCurve {
                target,
                clip: clip_id,
                effect,
                param: Param::MaskY,
                curve: cy,
            },
        ]))?;
        Ok(TrackResultDto {
            keys,
            weakest_match: weakest,
        })
    }

    /// Change an effect's non-animated options (FX-04 shape/invert, FX-05 key
    /// colour); fields left `None` keep their value.
    pub fn set_effect_options(
        &mut self,
        track: &str,
        clip: &str,
        index: usize,
        opts: EffectOptions,
    ) -> Result<(), String> {
        let (target, clip_id, c) = self.clip_ref(track, clip)?;
        let mut effect = c.effects.get(index).cloned().ok_or("no such effect")?;
        match &mut effect {
            Effect::Mask(m) => {
                if let Some(shape) = opts.shape {
                    m.shape = shape;
                }
                if let Some(invert) = opts.invert {
                    m.invert = invert;
                }
                if let Some(points) = opts.points {
                    if points.len() > debut_render::POLY_MAX_EDIT_POINTS {
                        return Err(format!(
                            "a polygon mask takes at most {} points",
                            debut_render::POLY_MAX_EDIT_POINTS
                        ));
                    }
                    // New points keep a smooth outline smooth; explicit
                    // handles below override.
                    let was_smooth = !m.handles.is_empty();
                    m.points = points;
                    m.handles = if was_smooth {
                        debut_render::smooth_handles(&m.points)
                    } else {
                        Vec::new()
                    };
                }
                if let Some(handles) = opts.handles {
                    if !handles.is_empty() && handles.len() != m.points.len() {
                        return Err("give one handle set per point".into());
                    }
                    if handles.iter().flatten().any(|v| !v.is_finite()) {
                        return Err("handles must be numbers".into());
                    }
                    m.handles = handles;
                }
                match opts.smooth {
                    Some(true) => m.handles = debut_render::smooth_handles(&m.points),
                    Some(false) => m.handles.clear(),
                    None => {}
                }
                // Switching to a polygon with no vertices yet: start from a
                // diamond the size of the rectangle, so something is visible.
                if m.shape == MaskShape::Polygon && m.points.len() < 3 {
                    let (hw, hh) = (
                        m.width.eval(Rational::ZERO) as f32 * 0.5,
                        m.height.eval(Rational::ZERO) as f32 * 0.5,
                    );
                    m.points = vec![[0.0, -hh], [hw, 0.0], [0.0, hh], [-hw, 0.0]];
                }
            }
            Effect::ChromaKey(k) => {
                if let Some(color) = opts.color {
                    k.color = color;
                }
            }
            _ => return Err("this effect has no options".into()),
        }
        self.exec(Command::ReplaceEffect {
            target,
            clip: clip_id,
            index,
            effect,
        })
    }

    pub fn remove_effect(&mut self, track: &str, clip: &str, index: usize) -> Result<(), String> {
        let (target, clip_id, _) = self.clip_ref(track, clip)?;
        self.exec(Command::RemoveEffect {
            target,
            clip: clip_id,
            index,
        })
    }

    /// Set a parameter as a constant, or keyframe it at the playhead.
    pub fn set_param(
        &mut self,
        track: &str,
        clip: &str,
        effect: usize,
        param: Param,
        value: f64,
        keyframe: bool,
    ) -> Result<(), String> {
        let (target, clip_id, c) = self.clip_ref(track, clip)?;
        let at = if keyframe {
            let fr = self.first_sequence()?.frame_rate;
            Some(fr.snap(self.playhead()) - c.timeline_in)
        } else {
            None
        };
        self.exec(Command::SetParam {
            target,
            clip: clip_id,
            effect,
            param,
            at,
            value,
        })
    }
}
