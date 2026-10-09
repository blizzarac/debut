//! Audio waveforms for the timeline (AUD-04): a peak cache per media, built on
//! a background job the first time it is asked for.

use super::*;
use debut_audio::{Peaks, PeaksBuilder, SampleSource};

/// Most buckets one request may ask for (a few screens wide).
const MAX_BUCKETS: usize = 8192;

/// Per media: `None` while the peaks are being built.
pub(crate) type WaveformCache = Arc<Mutex<std::collections::HashMap<MediaId, Option<Arc<Peaks>>>>>;

/// Decode all of `path`'s audio, mixed to mono, into peaks. Media without an
/// audio stream (or that cannot be opened) gets empty peaks: silence.
fn build_peaks(platform: &dyn Platform, media: MediaId, path: &str, duration: Rational) -> Peaks {
    const RATE: u32 = 48_000;
    const BLOCK: usize = RATE as usize;
    let mut builder = PeaksBuilder::new(RATE);
    let Ok(dec) = platform.open_decoder(path) else {
        return builder.finish();
    };
    if dec.audio_info().is_none() {
        return builder.finish();
    }
    let mut cache = crate::SampleCache::new(RATE);
    if cache.add(media, dec).is_err() {
        return builder.finish();
    }
    let total = (duration * Rational::from_int(RATE as i64)).round().max(0) as usize;
    let (mut done, mut buf) = (0usize, Vec::new());
    while done < total {
        let n = BLOCK.min(total - done);
        let start = Rational::new(done as i64, RATE as i64);
        let Ok(ch) = cache.read(media, start, n, &mut buf) else {
            break;
        };
        let ch = (ch as usize).max(1);
        let mono: Vec<f32> = buf
            .chunks(ch)
            .map(|f| f.iter().sum::<f32>() / ch as f32)
            .collect();
        builder.push(&mono);
        done += n;
    }
    builder.finish()
}

impl Session {
    /// `buckets` [min, max] pairs of `media`'s audio between source times
    /// `start` and `end` seconds. `None` while the peaks are still being
    /// built; ask again shortly.
    pub fn waveform(
        &mut self,
        media: &str,
        start: f64,
        end: f64,
        buckets: usize,
    ) -> Result<Option<Vec<[f32; 2]>>, String> {
        let id = MediaId(parse_id(media)?);
        let buckets = buckets.min(MAX_BUCKETS);
        {
            let cache = self.waveforms.lock().unwrap_or_else(|e| e.into_inner());
            match cache.get(&id) {
                Some(Some(peaks)) => return Ok(Some(peaks.range(start, end, buckets))),
                Some(None) => return Ok(None),
                None => {}
            }
        }
        let path = self
            .project()
            .and_then(|p| p.media.iter().find(|m| m.id == id))
            .map(|m| m.path.clone())
            .ok_or("unknown media")?;
        let duration = self.probed.get(&id).map(|p| p.2).unwrap_or(Rational::ZERO);
        self.waveforms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, None);
        let (platform, cache) = (Arc::clone(&self.platform), Arc::clone(&self.waveforms));
        let job = Box::new(move || {
            let peaks = build_peaks(platform.as_ref(), id, &path, duration);
            cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(id, Some(Arc::new(peaks)));
        });
        if let Err(e) = self.platform.spawn("debut-waveform", job) {
            // Forget the pending entry so a later request retries.
            self.waveforms
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            return Err(e.to_string());
        }
        Ok(None)
    }

    /// Drop a media's cached peaks (after relinking it to another file).
    pub(crate) fn forget_waveform(&mut self, id: MediaId) {
        self.waveforms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
    }
}
