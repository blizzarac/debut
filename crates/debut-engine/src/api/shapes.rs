//! Shape clips (GFX-03).

use super::*;

impl Session {
    /// Add a 5 s shape clip at `at` seconds, centred like a title, on the
    /// topmost video track with room for it.
    pub fn add_shape(&mut self, at: f64, shape: Shape) -> Result<String, String> {
        check_shape(&shape)?;
        self.add_generated(at, ClipSource::Shape(shape), Vec::new())
    }

    /// Replace a shape clip's geometry and colours.
    pub fn set_shape(&mut self, track: &str, clip: &str, shape: Shape) -> Result<(), String> {
        check_shape(&shape)?;
        let seq_id = self.first_sequence()?.id;
        let target = Target {
            sequence: seq_id,
            track: TrackId(parse_id(track)?),
        };
        let clip = ClipId(parse_id(clip)?);
        let is_shape = self
            .project()
            .and_then(|p| p.sequence(seq_id))
            .and_then(|s| s.track(target.track))
            .and_then(|t| t.clip(clip))
            .map(|c| matches!(c.source, ClipSource::Shape(_)))
            .ok_or("no such clip")?;
        if !is_shape {
            return Err("not a shape clip".into());
        }
        self.exec(Command::SetClipSource {
            target,
            clip,
            source: ClipSource::Shape(shape),
        })
    }
}

/// Shapes are rasterized at their authored size: keep it sane.
fn check_shape(shape: &Shape) -> Result<(), String> {
    let ok = |v: f32| v.is_finite() && (1.0..=16_384.0).contains(&v);
    if !ok(shape.width) || !ok(shape.height) {
        return Err("shape size must be 1 to 16384 px".into());
    }
    if !shape.stroke_px.is_finite() || !(0.0..=1_000.0).contains(&shape.stroke_px) {
        return Err("stroke must be 0 to 1000 px".into());
    }
    Ok(())
}
