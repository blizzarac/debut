//! Stable identifiers. IDs are 128-bit, random, and never reused, so command logs
//! from different machines merge without collisions (COL-01, COL-06).

use serde::{Deserialize, Serialize};
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

macro_rules! id_type {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        pub struct $name(pub u128);

        impl From<u128> for $name {
            fn from(v: u128) -> Self {
                $name(v)
            }
        }
    };
}

id_type!(ProjectId);
id_type!(SequenceId);
id_type!(TrackId);
id_type!(ClipId);
id_type!(MediaId);
id_type!(BinId);
id_type!(MarkerId);
id_type!(CaptionId);
id_type!(TemplateId);
id_type!(SnapshotId);

/// Deterministic ID generator (splitmix64 over a 128-bit state). Commands carry the
/// IDs they create, so replaying a log never calls this; only fresh edits do.
#[derive(Clone, Debug)]
pub struct IdGen {
    state: u64,
    salt: u64,
}

impl IdGen {
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed,
            salt: seed.rotate_left(32) ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    /// Seeded from process randomness. Works on native and wasm targets.
    pub fn random() -> Self {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(0x5EED);
        let a = h.finish();
        let mut h = RandomState::new().build_hasher();
        h.write_u64(0xB105);
        let b = h.finish();
        Self { state: a, salt: b }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state ^ self.salt;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn fresh<T: From<u128>>(&mut self) -> T {
        let hi = self.next_u64() as u128;
        let lo = self.next_u64() as u128;
        T::from((hi << 64) | lo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_generators_are_deterministic_and_distinct() {
        let mut a = IdGen::new(7);
        let mut b = IdGen::new(7);
        let x: ClipId = a.fresh();
        let y: ClipId = b.fresh();
        assert_eq!(x, y);
        let z: ClipId = a.fresh();
        assert_ne!(x, z);
    }
}
