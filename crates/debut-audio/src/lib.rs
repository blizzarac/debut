//! Audio engine (AUD-01 .. AUD-11, PB-02). Internally 48 kHz / 32-bit float.
//! The real-time callback never allocates or locks; everything crosses into it via
//! lock-free queues. Audio is the master clock video is presented against.

pub mod clock; // PB-02 sample position -> timeline time
pub mod ducking;
pub mod effects; // AUD-05 EQ, compressor, limiter, de-esser, gate, reverb
pub mod graph; // AUD-02, AUD-03 tracks, buses, channel layouts
pub mod loudness; // AUD-06 LUFS / true peak, normalization targets
pub mod waveform; // AUD-04 multi-resolution peak cache // AUD-08
