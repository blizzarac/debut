//! Bridges platform decoders to the render graph: implements
//! [`debut_render::FrameProvider`] over one decoder per media, with a small
//! per-media cache of linearized frames so scrubbing back a few frames is free.
//!
//! Frames stay 8-bit display-encoded RGBA all the way to the backend upload;
//! `compose` inserts the input transform for the media's tagged color space
//! (FX-08), so linearization happens on the GPU.

use debut_core::{Error, MediaId, Rational, Result};
use debut_platform::Decoder;
use debut_render::FrameProvider;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

struct Cached {
    pts: Rational,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
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
    /// Rasterized titles by content hash (GFX-01); a title re-renders only when
    /// its text or style changes.
    titles: Mutex<HashMap<u64, Arc<debut_render::Image8>>>,
}

impl FrameSource {
    pub fn new(cache_depth: usize) -> Self {
        Self {
            sources: HashMap::new(),
            cache_depth: cache_depth.max(1),
            titles: Mutex::new(HashMap::new()),
        }
    }

    /// The raster for `title`, rendering it on first use. `None` when the text
    /// cannot be rasterized (no usable font).
    pub fn title(&self, title: &debut_project::Title) -> Option<Arc<debut_render::Image8>> {
        let hash = title.hash();
        let mut cache = self.titles.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(img) = cache.get(&hash) {
            return Some(Arc::clone(img));
        }
        let raster = debut_graphics::render_title(&title.text, &title.style).ok()?;
        let img = Arc::new(debut_render::Image8 {
            hash,
            width: raster.width,
            height: raster.height,
            rgba8: raster.rgba8,
        });
        if cache.len() > 64 {
            cache.clear();
        }
        cache.insert(hash, Arc::clone(&img));
        Some(img)
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
    fn frame(&mut self, media: MediaId, t: Rational) -> Result<(u32, u32, Vec<u8>)> {
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
                pixels: f.rgba8,
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
