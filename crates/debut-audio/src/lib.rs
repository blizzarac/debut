//! Audio engine (AUD-01 .. AUD-11, PB-02). Internally 48 kHz / 32-bit float.
//! The real-time callback never allocates or locks; everything crosses into it via
//! a lock-free ring. Audio is the master clock video is presented against.

pub mod clock; // PB-02 sample position -> timeline time
pub mod engine; // renderer (producer) + RtSink (real-time consumer)
pub mod graph; // AUD-02, AUD-03 gain, pan, mute, solo, stereo bus
pub mod ring; // SPSC lock-free sample ring

pub mod ducking; // AUD-08 (pending)
pub mod effects; // AUD-05 EQ, compressor, limiter, de-esser, gate, reverb (pending)
pub mod loudness; // AUD-06 LUFS / true peak, normalization targets (pending)
pub mod waveform; // AUD-04 multi-resolution peak cache (pending)

pub use clock::Clock;
pub use effects::{AudioEffect, EqBand, EqKind, Processor};
pub use engine::{
    render_span, AudioRenderer, Inserts, RtSink, SampleSource, CHANNELS, MAX_NESTING,
};
pub use graph::TrackMix;
pub use loudness::{normalize_gain, LoudnessMeter};
