//! Proxy media (MED-05): small, quick-to-decode copies of camera files for
//! editing. A proxy is video only at 1/2 or 1/4 resolution; audio and export
//! always read the original. Proxies live next to the project as
//! `<project>.<media stem>.<id>.proxy<divisor>.mp4`. A `.done` marker that names
//! the source is written once a proxy is complete, so a half-written file or a
//! proxy of a since-relinked source is never used.

use debut_core::{MediaId, Rational, Result};
use debut_platform::codec::{Decoder, Encoder, VideoFrame};
use serde::{Deserialize, Serialize};

/// Resolutions offered: 1/2 and 1/4.
pub const DIVISORS: [u8; 2] = [2, 4];

/// Where the proxy of `media` (at `path`) for the project at `project_path`
/// goes.
pub fn proxy_path(project_path: &str, media: MediaId, path: &str, divisor: u8) -> String {
    let base = project_path.strip_suffix(".debut").unwrap_or(project_path);
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let stem: String = file
        .rsplit_once('.')
        .map_or(file, |(s, _)| s)
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    format!(
        "{base}.{stem}.{:x}.proxy{divisor}.mp4",
        media.0 & 0xffff_ffff
    )
}

/// The completion marker next to a proxy.
pub fn marker_path(proxy: &str) -> String {
    format!("{proxy}.done")
}

/// What a completion marker records.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub source: String,
    pub divisor: u8,
}

/// Proxy frame size: the source divided, rounded down to even (H.264 4:2:0).
pub fn proxy_size(width: u32, height: u32, divisor: u8) -> (u32, u32) {
    let d = divisor.max(1) as u32;
    (((width / d) & !1).max(2), ((height / d) & !1).max(2))
}

/// Box-filter an RGBA8 frame down to `proxy_size`.
pub fn downscale(px: &[u8], width: u32, height: u32, divisor: u8) -> (u32, u32, Vec<u8>) {
    let (w, h) = proxy_size(width, height, divisor);
    let d = divisor.max(1) as usize;
    let (sw, sh) = (width as usize, height as usize);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for yy in (y * d)..((y + 1) * d).min(sh) {
                for xx in (x * d)..((x + 1) * d).min(sw) {
                    let i = (yy * sw + xx) * 4;
                    for c in 0..4 {
                        acc[c] += px[i + c] as u32;
                    }
                    n += 1;
                }
            }
            let n = n.max(1);
            out.extend(acc.iter().map(|a| ((a + n / 2) / n) as u8));
        }
    }
    (w, h, out)
}

/// Transcode `decoder`'s video into `encoder` at 1/`divisor` size, as a
/// constant-rate stream whose frame n shows the source at n / rate: a source
/// that starts late repeats its first frame, gaps repeat the last one. So a
/// proxy's frame times are the original's source times. `progress` gets
/// 0..=1; returning false from it cancels.
pub fn build(
    decoder: &mut dyn Decoder,
    encoder: &mut dyn Encoder,
    divisor: u8,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<()> {
    let info = decoder
        .video_info()
        .ok_or_else(|| debut_core::Error::InvalidArgument("media has no video stream".into()))?
        .clone();
    let fd = info.frame_rate.frame_duration();
    let half = fd * Rational::new(1, 2);
    let time = |n: i64| info.frame_rate.frame_to_time(n);
    let total = (info.duration * info.frame_rate.0).ceil().max(1) as f32;
    let mut n = 0i64;
    let mut cur: Option<VideoFrame> = None;
    let mut last_pts = Rational::ZERO;
    let mut emit = |frame: &VideoFrame, n: &mut i64| -> Result<bool> {
        encoder.push_video(&VideoFrame {
            pts: time(*n),
            ..frame.clone()
        })?;
        *n += 1;
        Ok(progress((*n as f32 / total).min(1.0)))
    };
    while let Some(f) = decoder.next_video()? {
        let (w, h, px) = downscale(&f.rgba8, f.width, f.height, divisor);
        let small = VideoFrame {
            pts: f.pts,
            width: w,
            height: h,
            rgba8: px,
        };
        while time(n) + half < f.pts {
            if !emit(cur.as_ref().unwrap_or(&small), &mut n)? {
                return Err(debut_core::Error::Other("cancelled".into()));
            }
        }
        last_pts = f.pts;
        cur = Some(small);
    }
    if let Some(c) = cur {
        let end = info.duration.max(last_pts + fd);
        loop {
            if !emit(&c, &mut n)? {
                return Err(debut_core::Error::Other("cancelled".into()));
            }
            if time(n) + half >= end {
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::FrameRate;
    use debut_platform::codec::{AudioBlock, AudioInfo, VideoInfo};

    #[test]
    fn paths_sizes_and_box_filter() {
        let p = proxy_path("/p/Cut 1.debut", MediaId(0x1_2345_6789), "/m/A cam.mov", 2);
        assert_eq!(p, "/p/Cut 1.A_cam.23456789.proxy2.mp4");
        assert_eq!(marker_path(&p), format!("{p}.done"));
        assert_eq!(proxy_size(1920, 1080, 4), (480, 270 & !1));
        assert_eq!(proxy_size(5, 5, 4), (2, 2));
        // 4x2 → 2x1 at /2: each output averages a 2x2 block.
        let px: Vec<u8> = [0u8, 100, 10, 30, 200, 100, 20, 40]
            .iter()
            .flat_map(|v| [*v, *v, *v, 255])
            .collect();
        let (w, h, out) = downscale(&px, 4, 2, 2);
        assert_eq!((w, h), (2, 2));
        assert_eq!(out.len(), 2 * 2 * 4);
        assert_eq!(out[0], 100); // (0+100+200+100)/4
    }

    /// Frames at `pts` (seconds × 100), 4x4 grey of value = index.
    struct Frames(Vec<i64>, usize);
    impl Decoder for Frames {
        fn video_info(&self) -> Option<&VideoInfo> {
            static INFO: std::sync::OnceLock<VideoInfo> = std::sync::OnceLock::new();
            Some(INFO.get_or_init(|| VideoInfo {
                width: 4,
                height: 4,
                frame_rate: FrameRate::FPS_25,
                duration: Rational::new(20, 100),
                codec: "test".into(),
            }))
        }
        fn audio_info(&self) -> Option<&AudioInfo> {
            None
        }
        fn seek(&mut self, _: Rational) -> Result<()> {
            Ok(())
        }
        fn next_video(&mut self) -> Result<Option<VideoFrame>> {
            let i = self.1;
            self.1 += 1;
            Ok(self.0.get(i).map(|p| VideoFrame {
                pts: Rational::new(*p, 100),
                width: 4,
                height: 4,
                rgba8: vec![i as u8; 64],
            }))
        }
        fn next_audio(&mut self) -> Result<Option<AudioBlock>> {
            Ok(None)
        }
    }

    #[derive(Default)]
    struct Sink(Vec<(Rational, u32, u8)>);
    impl Encoder for Sink {
        fn push_video(&mut self, f: &VideoFrame) -> Result<()> {
            self.0.push((f.pts, f.width, f.rgba8[0]));
            Ok(())
        }
        fn push_audio(&mut self, _: &AudioBlock) -> Result<()> {
            Ok(())
        }
        fn finish(self: Box<Self>) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn build_resamples_to_constant_rate_from_zero() {
        // Starts at 0.08 s, skips 0.16: 25 fps output from 0 to 0.20 s.
        let mut dec = Frames(vec![8, 12, 20], 0);
        let mut sink = Sink::default();
        let mut seen = 0.0;
        build(&mut dec, &mut sink, 2, &mut |p| {
            seen = p;
            true
        })
        .unwrap();
        let values: Vec<u8> = sink.0.iter().map(|f| f.2).collect();
        // 0.00, 0.04 repeat the first frame; 0.08 first; 0.12 second; 0.16
        // repeats it; 0.20 third (the last frame runs one frame past duration).
        assert_eq!(values, vec![0, 0, 0, 1, 1, 2]);
        assert_eq!(sink.0[3].0, Rational::new(12, 100));
        assert_eq!(sink.0[0].1, 2, "half width");
        assert!(seen >= 1.0);
        // Cancelling stops with an error.
        let mut dec = Frames(vec![0, 4, 8], 0);
        assert!(build(&mut dec, &mut Sink::default(), 2, &mut |_| false).is_err());
    }
}
