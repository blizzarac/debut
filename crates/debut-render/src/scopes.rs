//! Video scopes (PB-08) computed on the CPU from a display-referred RGBA8 frame:
//! luma waveform, vectorscope and RGB histogram. Small, fixed-size images so the
//! UI can draw them every frame.

pub const WAVEFORM_W: usize = 256;
pub const WAVEFORM_H: usize = 128;
pub const VECTOR_SIZE: usize = 128;

#[derive(Clone, Debug, PartialEq)]
pub struct Scopes {
    /// `WAVEFORM_W x WAVEFORM_H`, row 0 = white; value = hit density 0..255.
    pub waveform: Vec<u8>,
    /// `VECTOR_SIZE x VECTOR_SIZE`, centre = neutral; value = hit density 0..255.
    pub vectorscope: Vec<u8>,
    /// Per channel, 256 bins.
    pub histogram: [[u32; 256]; 3],
}

/// Rec.709 luma and Cb/Cr from 8-bit RGB (display-referred).
fn ycbcr(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let cb = (b - y) / 1.8556;
    let cr = (r - y) / 1.5748;
    (y, cb, cr)
}

pub fn compute(rgba8: &[u8], width: u32, height: u32) -> Scopes {
    let (w, h) = (width as usize, height as usize);
    let mut wave = vec![0u32; WAVEFORM_W * WAVEFORM_H];
    let mut vec_ = vec![0u32; VECTOR_SIZE * VECTOR_SIZE];
    let mut hist = [[0u32; 256]; 3];
    for y in 0..h {
        for x in 0..w {
            let p = &rgba8[(y * w + x) * 4..][..4];
            hist[0][p[0] as usize] += 1;
            hist[1][p[1] as usize] += 1;
            hist[2][p[2] as usize] += 1;
            let (luma, cb, cr) = ycbcr(p[0], p[1], p[2]);
            let col = x * WAVEFORM_W / w.max(1);
            let row = ((1.0 - luma.clamp(0.0, 1.0)) * (WAVEFORM_H - 1) as f32) as usize;
            wave[row * WAVEFORM_W + col] += 1;
            let vx = ((cb.clamp(-0.5, 0.5) + 0.5) * (VECTOR_SIZE - 1) as f32) as usize;
            let vy = ((0.5 - cr.clamp(-0.5, 0.5)) * (VECTOR_SIZE - 1) as f32) as usize;
            vec_[vy * VECTOR_SIZE + vx] += 1;
        }
    }
    // Log-compress densities to 0..255 so sparse detail stays visible.
    let to_u8 = |counts: Vec<u32>| -> Vec<u8> {
        let max = counts.iter().copied().max().unwrap_or(0).max(1) as f32;
        counts
            .into_iter()
            .map(|c| {
                if c == 0 {
                    0
                } else {
                    (((c as f32).ln_1p() / max.ln_1p()) * 255.0).round() as u8
                }
            })
            .collect()
    };
    Scopes {
        waveform: to_u8(wave),
        vectorscope: to_u8(vec_),
        histogram: hist,
    }
}

impl Scopes {
    /// Packed little-endian bytes for IPC: waveform, vectorscope, then 3x256 u32.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out =
            Vec::with_capacity(self.waveform.len() + self.vectorscope.len() + 3 * 256 * 4);
        out.extend_from_slice(&self.waveform);
        out.extend_from_slice(&self.vectorscope);
        for ch in &self.histogram {
            for v in ch {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_grey_lands_on_one_waveform_row_and_the_vectorscope_centre() {
        let px: Vec<u8> = (0..16 * 8).flat_map(|_| [128u8, 128, 128, 255]).collect();
        let s = compute(&px, 16, 8);
        assert_eq!(
            s.waveform.iter().filter(|v| **v > 0).count(),
            16,
            "one row, 16 columns"
        );
        let c = VECTOR_SIZE / 2;
        let centre = &s.vectorscope[(c - 1) * VECTOR_SIZE..(c + 1) * VECTOR_SIZE];
        assert!(centre.iter().any(|v| *v > 0));
        assert_eq!(s.histogram[0][128], 128);
        assert_eq!(s.histogram[1].iter().sum::<u32>(), 128);
    }

    #[test]
    fn saturated_red_sits_in_the_upper_left_of_the_vectorscope() {
        let px: Vec<u8> = (0..4).flat_map(|_| [255u8, 0, 0, 255]).collect();
        let s = compute(&px, 2, 2);
        let (y, x) = s
            .vectorscope
            .iter()
            .enumerate()
            .find(|(_, v)| **v > 0)
            .map(|(i, _)| (i / VECTOR_SIZE, i % VECTOR_SIZE))
            .unwrap();
        assert!(y < VECTOR_SIZE / 2, "positive Cr is up (row {y})");
        assert!(x < VECTOR_SIZE / 2, "negative Cb is left (col {x})");
        let bytes = s.to_bytes();
        assert_eq!(
            bytes.len(),
            WAVEFORM_W * WAVEFORM_H + VECTOR_SIZE * VECTOR_SIZE + 3 * 256 * 4
        );
    }
}
