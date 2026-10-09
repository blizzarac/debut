//! Export and delivery (EXP-01 .. EXP-10, NFR-04).
//! Drives `debut-render` offline at full quality through a background queue, so
//! output matches the viewer exactly.

pub mod job; // EXP-04 ranges; the frame/audio walk into an Encoder
pub mod presets; // EXP-02 YouTube, Vimeo, Instagram, TikTok, broadcast, master
pub mod queue; // EXP-03 pause/resume/priority/cancel

pub mod hdr; // EXP-06 HDR10 / HLG metadata (pending)
pub mod smart; // EXP-05 pass-through of unmodified segments (pending)
pub mod upload; // EXP-09 (pending)

pub use job::{export, measure_loudness, Control, ExportJob, Progress};
pub use presets::{Preset, VideoCodec};
pub use queue::{ExportQueue, JobId, JobState};
