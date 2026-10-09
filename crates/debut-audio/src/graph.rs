//! Mixing (AUD-02, AUD-03): per-track gain, pan, mute and solo summed to a stereo
//! bus. Buses, channel layouts beyond stereo and inserts build on this.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackMix {
    /// Linear gain; 1.0 = unity.
    pub gain: f32,
    /// -1 = hard left, 0 = centre, +1 = hard right (constant power).
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
}

impl Default for TrackMix {
    fn default() -> Self {
        Self {
            gain: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
        }
    }
}

impl TrackMix {
    /// Left/right multipliers for a mono signal: equal-power pan law.
    pub fn pan_gains(&self) -> (f32, f32) {
        let p = (self.pan.clamp(-1.0, 1.0) + 1.0) * 0.25 * std::f32::consts::PI;
        (p.cos() * self.gain, p.sin() * self.gain)
    }

    /// Whether this track sounds given whether any track in the mix is soloed.
    pub fn audible(&self, any_solo: bool) -> bool {
        !self.mute && (!any_solo || self.solo)
    }
}

/// Add `src` (interleaved, `channels` wide) into the stereo `bus`, applying `mix`.
/// Mono is panned; stereo keeps its image and pan balances it; wider sources use
/// their first two channels.
pub fn mix_into(bus: &mut [f32], src: &[f32], channels: u16, mix: &TrackMix) {
    let frames = bus.len() / 2;
    let ch = channels.max(1) as usize;
    debug_assert!(src.len() >= frames * ch);
    let (l, r) = mix.pan_gains();
    match ch {
        1 => {
            for i in 0..frames {
                let s = src[i];
                bus[2 * i] += s * l;
                bus[2 * i + 1] += s * r;
            }
        }
        _ => {
            // Balance: scale so centre pan is unity on both sides.
            let c = std::f32::consts::FRAC_1_SQRT_2;
            let (bl, br) = (l / c, r / c);
            for i in 0..frames {
                bus[2 * i] += src[i * ch] * bl;
                bus[2 * i + 1] += src[i * ch + 1] * br;
            }
        }
    }
}

/// dBFS to linear gain.
pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centre_pan_is_equal_power() {
        let (l, r) = TrackMix::default().pan_gains();
        assert!((l - r).abs() < 1e-6);
        assert!((l * l + r * r - 1.0).abs() < 1e-6);
        let (l, r) = TrackMix {
            pan: -1.0,
            ..Default::default()
        }
        .pan_gains();
        assert!((l - 1.0).abs() < 1e-6 && r.abs() < 1e-6);
    }

    #[test]
    fn mono_is_panned_and_stereo_is_balanced() {
        let mut bus = [0.0; 4];
        mix_into(
            &mut bus,
            &[1.0, 1.0],
            1,
            &TrackMix {
                pan: 1.0,
                ..Default::default()
            },
        );
        assert!(bus[0].abs() < 1e-6 && (bus[1] - 1.0).abs() < 1e-6);

        let mut bus = [0.0; 4];
        mix_into(
            &mut bus,
            &[0.5, 0.25, 0.5, 0.25],
            2,
            &TrackMix {
                gain: 2.0,
                ..Default::default()
            },
        );
        assert!((bus[0] - 1.0).abs() < 1e-6 && (bus[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn solo_and_mute() {
        let t = TrackMix::default();
        assert!(t.audible(false));
        assert!(!t.audible(true));
        assert!(TrackMix { solo: true, ..t }.audible(true));
        assert!(!TrackMix {
            mute: true,
            solo: true,
            ..t
        }
        .audible(true));
        assert!((db_to_gain(-6.0206) - 0.5).abs() < 1e-4);
    }
}
