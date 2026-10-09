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
}

impl Effect {
    pub fn kind(&self) -> &'static str {
        match self {
            Effect::Transform(_) => "transform",
            Effect::Grade(_) => "grade",
            Effect::Lut { .. } => "lut",
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
