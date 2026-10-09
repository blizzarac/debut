//! Media ingest and management (MED-01 .. MED-13).
//! Decoding itself is a platform concern; this crate schedules and tracks it.

pub mod conform; // MED-03 VFR -> CFR (pending)
pub mod ingest; // MED-01, MED-02, MED-04, MED-13 (pending; single imports: `Session::import_media`)
pub mod interchange; // MED-12 FCPXML, EDL, AAF, OTIO (pending)
pub mod multicam; // MED-11 timecode / in-point sync (pending; audio sync: `debut_audio::sync`)
pub mod proxy; // MED-05 background proxy queue (pending)
pub mod relink; // MED-06 batch relink by name (pending; single files: `Session::relink_media`)
