//! Per-clip effect stack (FX-01, FX-02, FX-09, FX-11). Every numeric parameter
//! is a keyframe [`Curve`] in clip-local time (seconds from the clip's head), so
//! keys travel with the clip when it is moved or trimmed.

use debut_core::{Curve, Interp, Rational};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    Transform(TransformFx),
    Grade(GradeFx),
    /// A 3D LUT by content hash; the engine's LUT library resolves it.
    Lut {
        hash: u64,
        name: String,
    },
    /// Shape mask on the layer (FX-04).
    Mask(MaskFx),
    /// Chroma key (FX-05).
    ChromaKey(KeyFx),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskShape {
    Rectangle,
    Ellipse,
    /// A closed polygon given by `MaskFx::points`.
    Polygon,
}

/// A soft shape in sequence pixels: centre offset from the frame centre,
/// full width/height, feather width. `invert` keeps the outside instead.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskFx {
    pub shape: MaskShape,
    pub invert: bool,
    pub x: Curve,
    pub y: Curve,
    pub width: Curve,
    pub height: Curve,
    pub feather: Curve,
    /// Polygon vertices in sequence pixels from the frame centre, moved by
    /// `x` / `y` like the other shapes (FX-04).
    #[serde(default)]
    pub points: Vec<[f32; 2]>,
    /// Bézier handles per point, `[in_x, in_y, out_x, out_y]` relative to it;
    /// missing or zero makes a corner (FX-04).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub handles: Vec<[f32; 4]>,
    /// Planar track (FX-06): per-frame homographies, in clip-local time,
    /// mapping the polygon as drawn (sequence pixels from the frame centre)
    /// to where the tracked surface is. Held before the first and after the
    /// last key, blended between keys.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub planar: Vec<PlanarKey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanarKey {
    pub at: Rational,
    pub h: [f64; 9],
}

impl MaskFx {
    /// The planar-track homography at clip-local `t`, if the mask has one.
    pub fn planar_at(&self, t: Rational) -> Option<[f64; 9]> {
        let keys = &self.planar;
        let first = keys.first()?;
        if t <= first.at {
            return Some(first.h);
        }
        let i = keys.partition_point(|k| k.at <= t);
        if i >= keys.len() {
            return Some(keys[keys.len() - 1].h);
        }
        let (a, b) = (keys[i - 1], keys[i]);
        let f = ((t - a.at).as_f64() / (b.at - a.at).as_f64()).clamp(0.0, 1.0);
        let mut h = [0.0; 9];
        for (j, v) in h.iter_mut().enumerate() {
            *v = a.h[j] + (b.h[j] - a.h[j]) * f;
        }
        Some(h)
    }
}

impl Default for MaskFx {
    fn default() -> Self {
        Self {
            shape: MaskShape::Rectangle,
            invert: false,
            x: Curve::constant(0.0),
            y: Curve::constant(0.0),
            width: Curve::constant(960.0),
            height: Curve::constant(540.0),
            feather: Curve::constant(20.0),
            points: Vec::new(),
            handles: Vec::new(),
            planar: Vec::new(),
        }
    }
}

/// Chroma key: the colour to remove (straight sRGB bytes), how far in chroma
/// counts as that colour, the soft band past it, and how much spill to pull.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeyFx {
    pub color: [u8; 3],
    pub tolerance: Curve,
    pub softness: Curve,
    pub spill: Curve,
}

impl Default for KeyFx {
    fn default() -> Self {
        Self {
            color: [0, 255, 0],
            tolerance: Curve::constant(0.25),
            softness: Curve::constant(0.1),
            spill: Curve::constant(0.5),
        }
    }
}

/// Scale (1 = fit), rotation (degrees), position offset (sequence pixels), opacity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransformFx {
    pub scale: Curve,
    pub rotation: Curve,
    pub x: Curve,
    pub y: Curve,
    pub opacity: Curve,
}

impl Default for TransformFx {
    fn default() -> Self {
        Self {
            scale: Curve::constant(1.0),
            rotation: Curve::constant(0.0),
            x: Curve::constant(0.0),
            y: Curve::constant(0.0),
            opacity: Curve::constant(1.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradeFx {
    pub exposure: Curve,
    pub contrast: Curve,
    pub saturation: Curve,
    pub temperature: Curve,
    pub tint: Curve,
}

impl Default for GradeFx {
    fn default() -> Self {
        Self {
            exposure: Curve::constant(0.0),
            contrast: Curve::constant(1.0),
            saturation: Curve::constant(1.0),
            temperature: Curve::constant(0.0),
            tint: Curve::constant(0.0),
        }
    }
}

/// Addressable parameters, for commands and the inspector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Param {
    Scale,
    Rotation,
    X,
    Y,
    Opacity,
    Exposure,
    Contrast,
    Saturation,
    Temperature,
    Tint,
    MaskX,
    MaskY,
    MaskWidth,
    MaskHeight,
    Feather,
    Tolerance,
    Softness,
    Spill,
}

impl Effect {
    pub fn kind(&self) -> &'static str {
        match self {
            Effect::Transform(_) => "transform",
            Effect::Grade(_) => "grade",
            Effect::Lut { .. } => "lut",
            Effect::Mask(_) => "mask",
            Effect::ChromaKey(_) => "key",
        }
    }

    pub fn curve(&self, p: Param) -> Option<&Curve> {
        match (self, p) {
            (Effect::Transform(t), Param::Scale) => Some(&t.scale),
            (Effect::Transform(t), Param::Rotation) => Some(&t.rotation),
            (Effect::Transform(t), Param::X) => Some(&t.x),
            (Effect::Transform(t), Param::Y) => Some(&t.y),
            (Effect::Transform(t), Param::Opacity) => Some(&t.opacity),
            (Effect::Grade(g), Param::Exposure) => Some(&g.exposure),
            (Effect::Grade(g), Param::Contrast) => Some(&g.contrast),
            (Effect::Grade(g), Param::Saturation) => Some(&g.saturation),
            (Effect::Grade(g), Param::Temperature) => Some(&g.temperature),
            (Effect::Grade(g), Param::Tint) => Some(&g.tint),
            (Effect::Mask(m), Param::MaskX) => Some(&m.x),
            (Effect::Mask(m), Param::MaskY) => Some(&m.y),
            (Effect::Mask(m), Param::MaskWidth) => Some(&m.width),
            (Effect::Mask(m), Param::MaskHeight) => Some(&m.height),
            (Effect::Mask(m), Param::Feather) => Some(&m.feather),
            (Effect::ChromaKey(k), Param::Tolerance) => Some(&k.tolerance),
            (Effect::ChromaKey(k), Param::Softness) => Some(&k.softness),
            (Effect::ChromaKey(k), Param::Spill) => Some(&k.spill),
            _ => None,
        }
    }

    pub fn curve_mut(&mut self, p: Param) -> Option<&mut Curve> {
        match (self, p) {
            (Effect::Transform(t), Param::Scale) => Some(&mut t.scale),
            (Effect::Transform(t), Param::Rotation) => Some(&mut t.rotation),
            (Effect::Transform(t), Param::X) => Some(&mut t.x),
            (Effect::Transform(t), Param::Y) => Some(&mut t.y),
            (Effect::Transform(t), Param::Opacity) => Some(&mut t.opacity),
            (Effect::Grade(g), Param::Exposure) => Some(&mut g.exposure),
            (Effect::Grade(g), Param::Contrast) => Some(&mut g.contrast),
            (Effect::Grade(g), Param::Saturation) => Some(&mut g.saturation),
            (Effect::Grade(g), Param::Temperature) => Some(&mut g.temperature),
            (Effect::Grade(g), Param::Tint) => Some(&mut g.tint),
            (Effect::Mask(m), Param::MaskX) => Some(&mut m.x),
            (Effect::Mask(m), Param::MaskY) => Some(&mut m.y),
            (Effect::Mask(m), Param::MaskWidth) => Some(&mut m.width),
            (Effect::Mask(m), Param::MaskHeight) => Some(&mut m.height),
            (Effect::Mask(m), Param::Feather) => Some(&mut m.feather),
            (Effect::ChromaKey(k), Param::Tolerance) => Some(&mut k.tolerance),
            (Effect::ChromaKey(k), Param::Softness) => Some(&mut k.softness),
            (Effect::ChromaKey(k), Param::Spill) => Some(&mut k.spill),
            _ => None,
        }
    }

    /// The parameters this effect has, in inspector order.
    pub fn params(&self) -> &'static [Param] {
        match self {
            Effect::Transform(_) => &[
                Param::Scale,
                Param::Rotation,
                Param::X,
                Param::Y,
                Param::Opacity,
            ],
            Effect::Grade(_) => &[
                Param::Exposure,
                Param::Contrast,
                Param::Saturation,
                Param::Temperature,
                Param::Tint,
            ],
            Effect::Lut { .. } => &[],
            Effect::Mask(_) => &[
                Param::MaskX,
                Param::MaskY,
                Param::MaskWidth,
                Param::MaskHeight,
                Param::Feather,
            ],
            Effect::ChromaKey(_) => &[Param::Tolerance, Param::Softness, Param::Spill],
        }
    }

    /// Value of `p` at clip-local time `t`.
    pub fn value(&self, p: Param, t: Rational) -> Option<f64> {
        self.curve(p).map(|c| c.eval(t))
    }

    /// Set `p` to `value`: at a keyframe `t` (adding one), or as a constant.
    pub fn set(&mut self, p: Param, at: Option<Rational>, value: f64) -> bool {
        match self.curve_mut(p) {
            Some(c) => {
                match at {
                    Some(t) => {
                        // A constant becomes animated: its single held key starts interpolating.
                        if let [k] = c.keys() {
                            if k.interp == Interp::Hold {
                                let (kt, kv) = (k.t, k.value);
                                c.set(kt, kv, Interp::Linear);
                            }
                        }
                        c.set(t, value, Interp::Linear)
                    }
                    None => *c = Curve::constant(value),
                }
                true
            }
            None => false,
        }
    }
}
