//! Stable identifiers. IDs are 128-bit, random, and never reused, so command logs
//! from different machines merge without collisions (COL-01, COL-06).

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub struct $name(pub u128);
    };
}

id_type!(ProjectId);
id_type!(SequenceId);
id_type!(TrackId);
id_type!(ClipId);
id_type!(MediaId);
id_type!(BinId);
id_type!(MarkerId);
