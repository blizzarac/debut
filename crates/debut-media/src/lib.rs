//! Media ingest and management (MED-01 .. MED-13).
//! Decoding itself is a platform concern; this crate schedules and tracks it.

pub mod conform; // MED-03 VFR -> CFR (pending)
pub mod fcpxml; // MED-12 Final Cut Pro XML export
pub mod fcpxml_import; // MED-12 Final Cut Pro XML import
pub mod ingest; // MED-01, MED-02, MED-04, MED-13 (pending; single imports: `Session::import_media`)
pub mod interchange; // MED-12 EDL and OpenTimelineIO export (AAF pending)
pub mod multicam; // MED-11 (pending here; timecode sync in `Session::add_multicam_by`, audio in `debut_audio::sync`)
pub mod proxy; // MED-05 proxy paths, downscale and constant-rate transcode
pub mod relink; // MED-06 batch relink by name (pending; single files: `Session::relink_media`)
