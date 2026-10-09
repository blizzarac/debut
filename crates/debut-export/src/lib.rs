//! Export and delivery (EXP-01 .. EXP-10, NFR-04).
//! Drives `debut-render` offline at full quality through a background queue.

pub mod presets;  // EXP-02 YouTube, Vimeo, Instagram, TikTok, broadcast, master
pub mod queue;    // EXP-03 pause/resume/priority/notifications
pub mod job;      // EXP-04, EXP-07, EXP-08 ranges, batches, stems, stills, GIF
pub mod smart;    // EXP-05 pass-through of unmodified segments
pub mod hdr;      // EXP-06 HDR10 / HLG metadata
pub mod upload;   // EXP-09
