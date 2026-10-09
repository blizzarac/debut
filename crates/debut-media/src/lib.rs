//! Media ingest and management (MED-01 .. MED-13).
//! Decoding itself is a platform concern; this crate schedules and tracks it.

pub mod conform;
pub mod ingest; // MED-01, MED-02, MED-04, MED-13
pub mod interchange; // MED-12 FCPXML, EDL, AAF, OTIO
pub mod multicam; // MED-11 sync by timecode / waveform / in-point
pub mod proxy; // MED-05 background proxy queue
pub mod relink; // MED-06 // MED-03 VFR -> CFR
