//! Bridges platform decoders to the render graph: implements
//! [`debut_render::FrameProvider`] over one decoder per media, with a small
//! per-media cache of linearized frames so scrubbing back a few frames is free.
//!
//! Frames stay 8-bit display-encoded RGBA all the way to the backend upload;
//! `compose` inserts the input transform for the media's tagged color space
//! (FX-08), so linearization happens on the GPU.

use debut_core::{Error, MediaId, Rational, Result, SequenceId};
use debut_platform::Decoder;
use debut_render::FrameProvider;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
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
    /// A single image: its one frame stands for every time.
    still: bool,
}

pub struct FrameSource {
    sources: HashMap<MediaId, Source>,
    cache_depth: usize,
    /// Rasterized titles by content hash (GFX-01); a title re-renders only when
    /// its text or style changes.
    titles: Mutex<HashMap<u64, Arc<debut_render::Image8>>>,
    /// Every sequence of the project, for compound clips (TL-07).
    sequences: HashMap<SequenceId, Arc<debut_project::Sequence>>,
    /// Where title fonts come from; without it titles are left out.
    platform: Option<Arc<dyn debut_platform::Platform>>,
    /// Parsed fonts by family (`None`: the platform had none).
    fonts: Mutex<HashMap<String, Option<debut_graphics::Font>>>,
    /// The last error a plugin effect reported; the layer rendered without it.
    plugin_error: Option<String>,
}

impl FrameSource {
    pub fn new(cache_depth: usize) -> Self {
        Self {
            sources: HashMap::new(),
            cache_depth: cache_depth.max(1),
            titles: Mutex::new(HashMap::new()),
            sequences: HashMap::new(),
            platform: None,
            fonts: Mutex::new(HashMap::new()),
            plugin_error: None,
        }
    }

    /// The platform title fonts are requested from.
    pub fn set_platform(&mut self, platform: Arc<dyn debut_platform::Platform>) {
        self.platform = Some(platform);
    }

    fn font(&self, family: &str) -> Option<debut_graphics::Font> {
        let mut fonts = self.fonts.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(hit) = fonts.get(family) {
            return hit.clone();
        }
        let font = self
            .platform
            .as_ref()?
            .font(family)
            .and_then(|b| debut_graphics::Font::from_bytes(&b).ok());
        fonts.insert(family.to_string(), font.clone());
        font
    }

    /// Replace the set of sequences compound clips can refer to.
    pub fn set_sequences(&mut self, all: &[debut_project::Sequence]) {
        self.sequences = all.iter().map(|s| (s.id, Arc::new(s.clone()))).collect();
    }

    pub fn sequence(&self, id: SequenceId) -> Option<Arc<debut_project::Sequence>> {
        self.sequences.get(&id).cloned()
    }

    /// The raster for `title`, rendering it on first use. `None` when the text
    /// cannot be rasterized (no usable font).
    pub fn title(&self, title: &debut_project::Title) -> Option<Arc<debut_render::Image8>> {
        let hash = title.hash();
        let mut cache = self.titles.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(img) = cache.get(&hash) {
            return Some(Arc::clone(img));
        }
        let font = self.font(&title.style.font)?;
        let raster = debut_graphics::render_title(&title.text, &title.style, &font).ok()?;
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

    /// The raster for `shape` (GFX-03), rendered on first use; it shares the
    /// title cache.
    pub fn shape(&self, shape: &debut_project::Shape) -> Option<Arc<debut_render::Image8>> {
        let hash = shape.hash();
        let mut cache = self.titles.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(img) = cache.get(&hash) {
            return Some(Arc::clone(img));
        }
        let raster = debut_graphics::render_shape(shape);
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

    pub fn add(&mut self, media: MediaId, mut decoder: Box<dyn Decoder>) -> Result<()> {
        // Video only: otherwise the decoder queues all the audio it passes.
        decoder.select(true, false);
        let info = decoder
            .video_info()
            .ok_or_else(|| Error::InvalidArgument("media has no video stream".into()))?;
        let frame_duration = info.frame_rate.frame_duration();
        let still = info.duration == Rational::ZERO;
        self.sources.insert(
            media,
            Source {
                decoder,
                frame_duration,
                recent: VecDeque::new(),
                last_pts: None,
                still,
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

/// Size of the slate shown for media that has no decoder (offline or not yet
/// linked, MED-05): 16:9 so it fits a frame like footage would.
pub const OFFLINE_SIZE: (u32, u32) = (320, 180);

/// The offline slate: dark grey with a lighter diagonal band, so a missing
/// file is obvious in the viewer and the export without failing them.
pub fn offline_frame() -> Vec<u8> {
    let (w, h) = OFFLINE_SIZE;
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let band = ((x + y) / 24) % 2 == 0;
            let v = if band { 96 } else { 48 };
            px.extend_from_slice(&[v, v, v, 255]);
        }
    }
    px
}

impl FrameSource {
    /// How each media's video is being decoded (NFR-09).
    pub fn decode_paths(&self) -> Vec<(MediaId, debut_platform::DecodePath)> {
        self.sources
            .iter()
            .map(|(id, s)| (*id, s.decoder.decode_path()))
            .collect()
    }

    pub fn plugin_error(&self) -> Option<String> {
        self.plugin_error.clone()
    }

    /// Drop a media's decoder and cache (before relinking it).
    pub fn remove(&mut self, media: MediaId) {
        self.sources.remove(&media);
    }

    pub fn has(&self, media: MediaId) -> bool {
        self.sources.contains_key(&media)
    }
}

impl FrameProvider for FrameSource {
    fn plugin(
        &mut self,
        op: &debut_render::PluginOp,
        w: u32,
        h: u32,
        px: &mut [debut_render::Rgba],
    ) -> Result<bool> {
        let Some(host) = self.platform.as_ref().and_then(|p| p.plugins()) else {
            return Ok(false);
        };
        let job = debut_platform::plugin_host::VideoJob {
            plugin: debut_platform::PluginRef {
                kind: debut_platform::PluginKind::OpenFx,
                path: op.path.clone(),
                index: op.index,
            },
            width: w,
            height: h,
            frame: op.frame,
            fps: op.fps,
            params: op.params.clone(),
        };
        match host.process_video(&job, px.as_flattened_mut()) {
            Ok(()) => {
                self.plugin_error = None;
                Ok(true)
            }
            Err(e) => {
                self.plugin_error = Some(e.to_string());
                Err(e)
            }
        }
    }

    fn frame(&mut self, media: MediaId, t: Rational) -> Result<(u32, u32, Vec<u8>)> {
        let Some(src) = self.sources.get_mut(&media) else {
            return Ok((OFFLINE_SIZE.0, OFFLINE_SIZE.1, offline_frame()));
        };
        if src.still {
            if let Some(c) = src.recent.front() {
                return Ok((c.width, c.height, c.pixels.clone()));
            }
            src.decoder.seek(Rational::ZERO)?;
            let f = src
                .decoder
                .next_video()?
                .ok_or_else(|| Error::NotFound(format!("no picture in {media:?}")))?;
            let out = (f.width, f.height, f.rgba8.clone());
            src.recent.push_back(Cached {
                pts: f.pts,
                width: f.width,
                height: f.height,
                pixels: f.rgba8,
            });
            return Ok(out);
        }
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

        // Stepping forward, the frame on screen may already be here: with a
        // variable frame rate (MED-03) it holds across a gap, and the next
        // frame decoded can lie beyond `t`.
        let mut best: Option<Cached> = if far {
            None
        } else {
            src.recent
                .iter()
                .filter(|c| c.pts <= t)
                .max_by_key(|c| c.pts)
                .cloned()
        };
        let keep = |recent: &mut VecDeque<Cached>, c: Cached| {
            if !recent.iter().any(|r| r.pts == c.pts) {
                recent.push_back(c);
            }
        };
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
                keep(&mut src.recent, c);
                break;
            }
            // Keep the frames passed on the way: reverse playback (TL-09)
            // then finds the previous frames here instead of seeking again.
            if let Some(prev) = best.replace(c) {
                keep(&mut src.recent, prev);
                if src.recent.len() > self.cache_depth {
                    src.recent.pop_front();
                }
            }
            if done {
                break;
            }
        }
        let best = best.ok_or_else(|| Error::NotFound(format!("no frame at {t} in {media:?}")))?;
        let out = (best.width, best.height, best.pixels.clone());
        keep(&mut src.recent, best);
        while src.recent.len() > self.cache_depth {
            src.recent.pop_front();
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::IdGen;

    /// Whatever order frames are asked for in, a time inside a variable-rate
    /// gap shows the frame before it (MED-03): stepping forward from the
    /// last frame before the gap once showed the one after it.
    #[test]
    fn holds_the_frame_before_a_variable_rate_gap() {
        let p = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../debut-platform-native/tests/fixtures/vfr_64x36_2s.mp4"
        );
        let open = || Box::new(debut_platform_native::codec::FfmpegDecoder::open(p).unwrap());
        let mut direct = open();
        let mut frames = Vec::new();
        while let Some(f) = direct.next_video().unwrap() {
            frames.push((f.pts, f.rgba8));
        }
        let mut fs = FrameSource::new(8);
        let id: MediaId = IdGen::new(1).fresh();
        fs.add(id, open()).unwrap();
        // Frames at 0.30 s, then 0.667 s; asked forwards, then backwards.
        let times = [0, 4, 28, 32, 36, 48, 64, 68, 64, 36, 32];
        let expect = [
            0.0,
            1.0 / 30.0,
            8.0 / 30.0,
            0.3,
            0.3,
            0.3,
            0.3,
            20.0 / 30.0,
            0.3,
            0.3,
            0.3,
        ];
        for (t, want) in times.iter().zip(expect) {
            let (_, _, px) = fs.frame(id, Rational::new(*t, 100)).unwrap();
            let got = frames
                .iter()
                .find(|(_, b)| *b == px)
                .map(|(p, _)| p.as_f64());
            assert!(
                got.is_some_and(|g| (g - want).abs() < 1e-9),
                "at {t}/100 s: {got:?}, want {want}"
            );
        }
    }
}
