//! Bridges platform decoders to the render graph: implements
//! [`debut_render::FrameProvider`] over one decoder per media, with a small
//! per-media cache of linearized frames so scrubbing back a few frames is free.
//!
//! Frames arrive as 8-bit display-encoded RGBA and leave as linear-light `f32`
//! premultiplied RGBA. The transfer function is sRGB for now; the OCIO pipeline
//! (FX-08) replaces this with the clip's tagged input transform.

use debut_core::{Error, MediaId, Rational, Result};
use debut_platform::Decoder;
use debut_render::{FrameProvider, Rgba};
use std::collections::{HashMap, VecDeque};

fn srgb_to_linear(v: u8) -> f32 {
    let c = v as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn lut() -> &'static [f32; 256] {
    static LUT: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    LUT.get_or_init(|| std::array::from_fn(|i| srgb_to_linear(i as u8)))
}

/// Convert display-encoded RGBA8 to linear premultiplied f32.
pub fn linearize(rgba8: &[u8]) -> Vec<Rgba> {
    let lut = lut();
    rgba8
        .chunks_exact(4)
        .map(|p| {
            let a = p[3] as f32 / 255.0;
            [
                lut[p[0] as usize] * a,
                lut[p[1] as usize] * a,
                lut[p[2] as usize] * a,
                a,
            ]
        })
        .collect()
}

struct Cached {
    pts: Rational,
    width: u32,
    height: u32,
    pixels: Vec<Rgba>,
}

struct Source {
    decoder: Box<dyn Decoder>,
    frame_duration: Rational,
    recent: VecDeque<Cached>,
    /// pts of the last frame pulled from the decoder, to tell forward from backward.
    last_pts: Option<Rational>,
}

pub struct FrameSource {
    sources: HashMap<MediaId, Source>,
    cache_depth: usize,
}

impl FrameSource {
    pub fn new(cache_depth: usize) -> Self {
        Self {
            sources: HashMap::new(),
            cache_depth: cache_depth.max(1),
        }
    }

    pub fn add(&mut self, media: MediaId, decoder: Box<dyn Decoder>) -> Result<()> {
        let info = decoder
            .video_info()
            .ok_or_else(|| Error::InvalidArgument("media has no video stream".into()))?;
        let frame_duration = info.frame_rate.frame_duration();
        self.sources.insert(
            media,
            Source {
                decoder,
                frame_duration,
                recent: VecDeque::new(),
                last_pts: None,
            },
        );
        Ok(())
    }

    pub fn dimensions(&self, media: MediaId) -> Option<(u32, u32)> {
        self.sources
            .get(&media)
            .and_then(|s| s.decoder.video_info())
            .map(|v| (v.width, v.height))
    }
}

impl FrameProvider for FrameSource {
    fn frame(&mut self, media: MediaId, t: Rational) -> Result<(u32, u32, Vec<Rgba>)> {
        let src = self
            .sources
            .get_mut(&media)
            .ok_or_else(|| Error::NotFound(format!("media {media:?}")))?;
        let covers = |c: &Cached| c.pts <= t && t < c.pts + src.frame_duration;

        if let Some(c) = src.recent.iter().find(|c| covers(c)) {
            return Ok((c.width, c.height, c.pixels.clone()));
        }

        // Decode forward if the target is just ahead; otherwise seek first.
        let far = match src.last_pts {
            Some(last) => t < last || t > last + src.frame_duration * Rational::from_int(8),
            None => true,
        };
        if far {
            src.decoder.seek(t)?;
            src.recent.clear();
        }

        let mut best: Option<Cached> = None;
        while let Some(f) = src.decoder.next_video()? {
            src.last_pts = Some(f.pts);
            let c = Cached {
                pts: f.pts,
                width: f.width,
                height: f.height,
                pixels: linearize(&f.rgba8),
            };
            let done = covers(&c) || f.pts > t;
            if f.pts > t && best.is_some() {
                // Overshot: keep the previous frame (covers t up to the next pts).
                src.recent.push_back(c);
                break;
            }
            best = Some(c);
            if done {
                break;
            }
        }
        let best = best.ok_or_else(|| Error::NotFound(format!("no frame at {t} in {media:?}")))?;
        let out = (best.width, best.height, best.pixels.clone());
        src.recent.push_back(best);
        while src.recent.len() > self.cache_depth {
            src.recent.pop_front();
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linearize_uses_srgb_and_premultiplies() {
        let px = linearize(&[255, 0, 128, 255, 255, 255, 255, 0]);
        assert_eq!(px[0][0], 1.0);
        assert!((px[0][2] - 0.2158605).abs() < 1e-5);
        assert_eq!(px[1], [0.0, 0.0, 0.0, 0.0]);
    }
}
