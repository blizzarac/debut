//! LUT import and application (FX-11): Adobe/Resolve `.cube` 1D and 3D LUTs,
//! applied with trilinear interpolation over the LUT's declared domain.

use debut_core::{Error, Result};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct Lut3d {
    pub size: usize,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    /// `size³` RGB triples, red fastest (the .cube order).
    pub data: Vec<[f32; 3]>,
    /// Content hash, so graphs can be hashed without walking the table.
    pub hash: u64,
}

impl Lut3d {
    pub fn identity(size: usize) -> Arc<Self> {
        let mut data = Vec::with_capacity(size * size * size);
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    let s = (size - 1) as f32;
                    data.push([r as f32 / s, g as f32 / s, b as f32 / s]);
                }
            }
        }
        Arc::new(Self::new(size, [0.0; 3], [1.0; 3], data))
    }

    pub fn new(
        size: usize,
        domain_min: [f32; 3],
        domain_max: [f32; 3],
        data: Vec<[f32; 3]>,
    ) -> Self {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        size.hash(&mut h);
        for v in domain_min
            .iter()
            .chain(&domain_max)
            .chain(data.iter().flatten())
        {
            v.to_bits().hash(&mut h);
        }
        Self {
            size,
            domain_min,
            domain_max,
            data,
            hash: h.finish(),
        }
    }

    /// Parse `.cube` text. A 1D LUT is expanded to a 3D one of the same size so
    /// one apply path serves both.
    pub fn parse_cube(text: &str) -> Result<Arc<Self>> {
        let mut size_3d = None;
        let mut size_1d = None;
        let mut domain_min = [0.0f32; 3];
        let mut domain_max = [1.0f32; 3];
        let mut rows: Vec<[f32; 3]> = Vec::new();
        let bad = |l: &str| Error::InvalidArgument(format!("bad .cube line: {l}"));
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with("TITLE") {
                continue;
            }
            let mut it = line.split_whitespace();
            match it.next() {
                Some("LUT_3D_SIZE") => {
                    size_3d = Some(
                        it.next()
                            .and_then(|v| v.parse().ok())
                            .ok_or_else(|| bad(line))?,
                    )
                }
                Some("LUT_1D_SIZE") => {
                    size_1d = Some(
                        it.next()
                            .and_then(|v| v.parse().ok())
                            .ok_or_else(|| bad(line))?,
                    )
                }
                Some("DOMAIN_MIN") | Some("DOMAIN_MAX") => {
                    let mut v = [0.0; 3];
                    for x in &mut v {
                        *x = it
                            .next()
                            .and_then(|s| s.parse().ok())
                            .ok_or_else(|| bad(line))?;
                    }
                    if line.starts_with("DOMAIN_MIN") {
                        domain_min = v
                    } else {
                        domain_max = v
                    }
                }
                Some(first) => {
                    let r: f32 = first.parse().map_err(|_| bad(line))?;
                    let g: f32 = it
                        .next()
                        .and_then(|s| s.parse().ok())
                        .ok_or_else(|| bad(line))?;
                    let b: f32 = it
                        .next()
                        .and_then(|s| s.parse().ok())
                        .ok_or_else(|| bad(line))?;
                    rows.push([r, g, b]);
                }
                None => {}
            }
        }
        if let Some(n) = size_3d {
            if n < 2 || rows.len() != n * n * n {
                return Err(Error::InvalidArgument(format!(
                    "3D LUT size {n} needs {} rows, found {}",
                    n * n * n,
                    rows.len()
                )));
            }
            return Ok(Arc::new(Self::new(n, domain_min, domain_max, rows)));
        }
        if let Some(n) = size_1d {
            if n < 2 || rows.len() != n {
                return Err(Error::InvalidArgument(format!(
                    "1D LUT size {n} needs {n} rows, found {}",
                    rows.len()
                )));
            }
            let mut data = Vec::with_capacity(n * n * n);
            for b in 0..n {
                for g in 0..n {
                    for r in 0..n {
                        data.push([rows[r][0], rows[g][1], rows[b][2]]);
                    }
                }
            }
            return Ok(Arc::new(Self::new(n, domain_min, domain_max, data)));
        }
        Err(Error::InvalidArgument(
            "no LUT_3D_SIZE or LUT_1D_SIZE".into(),
        ))
    }

    fn at(&self, r: usize, g: usize, b: usize) -> [f32; 3] {
        self.data[(b * self.size + g) * self.size + r]
    }

    /// Trilinear lookup; input outside the domain is clamped.
    pub fn apply_rgb(&self, c: [f32; 3]) -> [f32; 3] {
        let n = self.size;
        let s = (n - 1) as f32;
        let mut idx = [0usize; 3];
        let mut frac = [0f32; 3];
        for i in 0..3 {
            let u = ((c[i] - self.domain_min[i]) / (self.domain_max[i] - self.domain_min[i]))
                .clamp(0.0, 1.0)
                * s;
            let i0 = (u.floor() as usize).min(n - 2);
            idx[i] = i0;
            frac[i] = u - i0 as f32;
        }
        let [r0, g0, b0] = idx;
        let [fr, fg, fb] = frac;
        let lerp = |a: [f32; 3], b: [f32; 3], t: f32| {
            [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ]
        };
        let c00 = lerp(self.at(r0, g0, b0), self.at(r0 + 1, g0, b0), fr);
        let c10 = lerp(self.at(r0, g0 + 1, b0), self.at(r0 + 1, g0 + 1, b0), fr);
        let c01 = lerp(self.at(r0, g0, b0 + 1), self.at(r0 + 1, g0, b0 + 1), fr);
        let c11 = lerp(
            self.at(r0, g0 + 1, b0 + 1),
            self.at(r0 + 1, g0 + 1, b0 + 1),
            fr,
        );
        lerp(lerp(c00, c10, fg), lerp(c01, c11, fg), fb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_lut_is_identity_between_nodes() {
        let lut = Lut3d::identity(9);
        for c in [
            [0.0, 0.0, 0.0],
            [0.3, 0.6, 0.9],
            [1.0, 1.0, 1.0],
            [0.123, 0.456, 0.789],
        ] {
            let o = lut.apply_rgb(c);
            assert!(
                o.iter().zip(&c).all(|(a, b)| (a - b).abs() < 1e-6),
                "{c:?} -> {o:?}"
            );
        }
        assert_eq!(lut.apply_rgb([2.0, -1.0, 0.5]), [1.0, 0.0, 0.5]);
    }

    #[test]
    fn parses_3d_and_1d_cube_files() {
        let text = "TITLE \"swap\"\nLUT_3D_SIZE 2\n# r g b\n0 0 0\n0 0 1\n0 1 0\n0 1 1\n1 0 0\n1 0 1\n1 1 0\n1 1 1\n";
        let lut = Lut3d::parse_cube(text).unwrap();
        // This table maps (r,g,b) -> (b,g,r).
        let o = lut.apply_rgb([1.0, 0.25, 0.0]);
        assert!(
            (o[0] - 0.0).abs() < 1e-6 && (o[1] - 0.25).abs() < 1e-6 && (o[2] - 1.0).abs() < 1e-6,
            "{o:?}"
        );

        let text =
            "LUT_1D_SIZE 3\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\n0 0 0\n0.25 0.5 0.75\n1 1 1\n";
        let lut = Lut3d::parse_cube(text).unwrap();
        let o = lut.apply_rgb([0.5, 0.5, 0.5]);
        assert!(
            (o[0] - 0.25).abs() < 1e-6 && (o[1] - 0.5).abs() < 1e-6 && (o[2] - 0.75).abs() < 1e-6,
            "{o:?}"
        );

        assert!(Lut3d::parse_cube("LUT_3D_SIZE 2\n0 0 0\n").is_err());
        assert_ne!(Lut3d::identity(2).hash, Lut3d::identity(3).hash);
    }
}
