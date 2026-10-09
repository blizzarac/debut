//! Text rasterization for titles (GFX-01): a TrueType font (bytes from the
//! platform) through
//! `fontdue`, simple multi-line layout, fill colour, optional stroke-like outline
//! (dilated alpha), drop shadow and background box. Output is straight-alpha
//! RGBA8 in sRGB, sized to the text, for the render graph's `Image` node.

use debut_core::{Error, Result};
use debut_project::title::{TextAlign, TitleStyle};
use std::sync::Arc;

/// A rasterized title.
#[derive(Clone, Debug, PartialEq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

/// A parsed font face, cheap to clone. The platform supplies the bytes
/// (`Platform::font`); this crate never touches the file system.
#[derive(Clone)]
pub struct Font(Arc<fontdue::Font>);

impl Font {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
            .map(|f| Font(Arc::new(f)))
            .map_err(|e| Error::Other(format!("font: {e}")))
    }
}

impl std::fmt::Debug for Font {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Font")
    }
}

struct Glyph {
    x: f32,
    y: f32,
    w: usize,
    h: usize,
    coverage: Vec<u8>,
}

/// Rasterize `text` with `style`. Lines are split on `\n`; the image is sized to
/// the text plus padding for stroke, shadow and background.
pub fn render(text: &str, style: &TitleStyle, font: &Font) -> Result<Raster> {
    let font = &font.0;
    let size = style.size_px.max(4.0);
    let metrics = font
        .horizontal_line_metrics(size)
        .ok_or_else(|| Error::Other("font has no horizontal metrics".into()))?;
    let line_h = (metrics.new_line_size * style.line_height).max(1.0);
    let lines: Vec<&str> = if text.is_empty() {
        vec![" "]
    } else {
        text.split('\n').collect()
    };

    // Layout: per line, glyph positions and width.
    let mut laid: Vec<(Vec<Glyph>, f32)> = Vec::new();
    for line in &lines {
        let mut x = 0.0f32;
        let mut glyphs = Vec::new();
        let mut prev: Option<char> = None;
        for ch in line.chars() {
            if let Some(p) = prev {
                x += font.horizontal_kern(p, ch, size).unwrap_or(0.0);
            }
            let (m, bitmap) = font.rasterize(ch, size);
            glyphs.push(Glyph {
                x: x + m.xmin as f32,
                y: -(m.ymin as f32) - m.height as f32,
                w: m.width,
                h: m.height,
                coverage: bitmap,
            });
            x += m.advance_width + style.letter_spacing;
            prev = Some(ch);
        }
        laid.push((glyphs, x.max(0.0)));
    }
    let text_h = (line_h * lines.len() as f32).ceil();
    let pad = (style.stroke_px + style.shadow_px.abs() + style.padding_px).ceil() + 2.0;
    let text_w = laid
        .iter()
        .map(|l| l.1)
        .fold(0.0f32, f32::max)
        .max(style.min_width_px - 2.0 * pad)
        .ceil();
    let width = (text_w + 2.0 * pad) as u32;
    let height = (text_h + 2.0 * pad) as u32;
    let (wu, hu) = (width as usize, height as usize);

    // Coverage mask of the glyphs (0..255) at final positions.
    let mut mask = vec![0u8; wu * hu];
    for (li, (glyphs, line_w)) in laid.iter().enumerate() {
        let align_dx = match style.align {
            TextAlign::Left => 0.0,
            TextAlign::Center => (text_w - line_w) / 2.0,
            TextAlign::Right => text_w - line_w,
        };
        let baseline = pad + line_h * li as f32 + metrics.ascent;
        for g in glyphs {
            let ox = (pad + align_dx + g.x).round() as i64;
            let oy = (baseline + g.y).round() as i64;
            for gy in 0..g.h {
                for gx in 0..g.w {
                    let (x, y) = (ox + gx as i64, oy + gy as i64);
                    if x < 0 || y < 0 || x >= wu as i64 || y >= hu as i64 {
                        continue;
                    }
                    let i = y as usize * wu + x as usize;
                    mask[i] = mask[i].max(g.coverage[gy * g.w + gx]);
                }
            }
        }
    }

    let stroke = if style.stroke_px > 0.0 {
        Some(dilate(&mask, wu, hu, style.stroke_px))
    } else {
        None
    };
    let shadow = if style.shadow_px != 0.0 {
        Some(blur(
            &stroke.clone().unwrap_or_else(|| mask.clone()),
            wu,
            hu,
            style.shadow_px.abs(),
        ))
    } else {
        None
    };

    // Composite layers back to front: background, shadow, stroke, fill (straight alpha).
    let mut out = vec![0u8; wu * hu * 4];
    let put = |out: &mut [u8], i: usize, color: [u8; 4], a: f32| {
        let a = a * color[3] as f32 / 255.0;
        if a <= 0.0 {
            return;
        }
        let d = &mut out[i * 4..i * 4 + 4];
        let da = d[3] as f32 / 255.0;
        let oa = a + da * (1.0 - a);
        for c in 0..3 {
            let s = color[c] as f32;
            let dst = d[c] as f32;
            d[c] =
                (((s * a + dst * da * (1.0 - a)) / oa.max(1e-6)).round()).clamp(0.0, 255.0) as u8;
        }
        d[3] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
    };
    if style.background[3] > 0 {
        for i in 0..wu * hu {
            put(&mut out, i, style.background, 1.0);
        }
    }
    if let Some(sh) = &shadow {
        let (dx, dy) = (
            style.shadow_px.round() as i64,
            style.shadow_px.round() as i64,
        );
        for y in 0..hu as i64 {
            for x in 0..wu as i64 {
                let (sx, sy) = (x - dx, y - dy);
                if sx < 0 || sy < 0 || sx >= wu as i64 || sy >= hu as i64 {
                    continue;
                }
                let a = sh[sy as usize * wu + sx as usize] as f32 / 255.0;
                put(
                    &mut out,
                    y as usize * wu + x as usize,
                    style.shadow_color,
                    a,
                );
            }
        }
    }
    if let Some(st) = &stroke {
        for (i, &v) in st.iter().enumerate().take(wu * hu) {
            put(&mut out, i, style.stroke_color, v as f32 / 255.0);
        }
    }
    for (i, &v) in mask.iter().enumerate().take(wu * hu) {
        put(&mut out, i, style.color, v as f32 / 255.0);
    }
    Ok(Raster {
        width,
        height,
        rgba8: out,
    })
}

/// Max-filter the mask by `r` pixels (a cheap round outline).
fn dilate(mask: &[u8], w: usize, h: usize, r: f32) -> Vec<u8> {
    let ri = r.ceil() as i64;
    let r2 = r * r;
    let mut out = vec![0u8; w * h];
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let mut m = 0u8;
            for dy in -ri..=ri {
                for dx in -ri..=ri {
                    if (dx * dx + dy * dy) as f32 > r2 + 0.5 {
                        continue;
                    }
                    let (sx, sy) = (x + dx, y + dy);
                    if sx >= 0 && sy >= 0 && sx < w as i64 && sy < h as i64 {
                        m = m.max(mask[sy as usize * w + sx as usize]);
                    }
                }
            }
            out[y as usize * w + x as usize] = m;
        }
    }
    out
}

/// Separable box blur, radius ~`r`.
fn blur(mask: &[u8], w: usize, h: usize, r: f32) -> Vec<u8> {
    let ri = (r.ceil() as usize).max(1);
    let n = (2 * ri + 1) as f32;
    let mut tmp = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut s = 0.0;
            for dx in 0..=2 * ri {
                let sx = x as i64 + dx as i64 - ri as i64;
                if sx >= 0 && sx < w as i64 {
                    s += mask[y * w + sx as usize] as f32;
                }
            }
            tmp[y * w + x] = s / n;
        }
    }
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut s = 0.0;
            for dy in 0..=2 * ri {
                let sy = y as i64 + dy as i64 - ri as i64;
                if sy >= 0 && sy < h as i64 {
                    s += tmp[sy as usize * w + x];
                }
            }
            out[y * w + x] = (s / n).round() as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_platform::Platform;

    /// The system font through the native platform, as the engine gets it.
    fn render(text: &str, style: &TitleStyle) -> Result<Raster> {
        let bytes = debut_platform_native::NativePlatform::new()
            .font(&style.font)
            .expect("a system or fallback font");
        super::render(text, style, &Font::from_bytes(&bytes)?)
    }

    #[test]
    fn renders_text_with_alpha_only_where_glyphs_are() {
        let style = TitleStyle::default();
        let r = render("Hello", &style).unwrap();
        assert!(r.width > 20 && r.height > 10, "{}x{}", r.width, r.height);
        let alphas: Vec<u8> = r.rgba8.chunks(4).map(|p| p[3]).collect();
        let lit = alphas.iter().filter(|a| **a > 0).count();
        assert!(
            lit > 50 && lit < alphas.len() / 2,
            "{lit} of {}",
            alphas.len()
        );
        // Corners are transparent (padding), fill is white.
        assert_eq!(alphas[0], 0);
        let white = r
            .rgba8
            .chunks(4)
            .filter(|p| p[3] == 255)
            .all(|p| p[0] == 255 && p[1] == 255 && p[2] == 255);
        assert!(white);
    }

    #[test]
    fn stroke_shadow_and_background_grow_the_coverage() {
        let plain = render("Ab", &TitleStyle::default()).unwrap();
        let fancy = render(
            "Ab",
            &TitleStyle {
                stroke_px: 2.0,
                stroke_color: [0, 0, 0, 255],
                shadow_px: 3.0,
                background: [0, 0, 0, 128],
                ..TitleStyle::default()
            },
        )
        .unwrap();
        let count = |r: &Raster| r.rgba8.chunks(4).filter(|p| p[3] > 0).count();
        assert!(
            count(&fancy) == (fancy.width * fancy.height) as usize,
            "background fills everything"
        );
        assert!(fancy.width > plain.width && fancy.height > plain.height);
        let multi = render("one\ntwo\nthree", &TitleStyle::default()).unwrap();
        assert!(multi.height > plain.height * 2);
        assert!(
            render(
                "x",
                &TitleStyle {
                    font: "definitely-not-a-font".into(),
                    ..TitleStyle::default()
                }
            )
            .is_ok(),
            "falls back to a system font"
        );
    }
}
