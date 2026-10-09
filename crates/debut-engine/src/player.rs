//! One sequence playing: transport + audio renderer + frame source, driven by two
//! threads the shell owns — the device callback (gets the [`RtSink`]) and the UI
//! refresh (calls [`Player::tick`]).

use crate::frames::FrameSource;
use crate::playback::{Stats, Transport};
use crate::samples::SampleCache;
use debut_audio::{AudioRenderer, Clock, RtSink, TrackMix};
use debut_core::{MediaId, Rational, Result, TrackId};
use debut_platform::Decoder;
use debut_project::Sequence;
use debut_render::compose::{compose, SourceInfo};
use debut_render::Graph;
use std::sync::Arc;

pub const SAMPLE_RATE: u32 = 48_000;
/// Keep a quarter second of audio rendered ahead of the device.
const LOOK_AHEAD_FRAMES: usize = SAMPLE_RATE as usize / 4;
const BLOCK_FRAMES: usize = 512;

pub struct Player {
    pub sequence: Sequence,
    pub transport: Transport,
    audio: AudioRenderer,
    pub frames: FrameSource,
    pub samples: SampleCache,
}

impl SourceInfo for FrameSource {
    fn dimensions(&self, media: MediaId) -> (u32, u32) {
        FrameSource::dimensions(self, media).unwrap_or((1, 1))
    }
}

impl Player {
    /// Returns the player and the sink to hand to the platform `AudioOut`.
    pub fn new(sequence: Sequence) -> (Self, RtSink) {
        let clock = Clock::new(SAMPLE_RATE);
        let (audio, sink) = AudioRenderer::new(Arc::clone(&clock), LOOK_AHEAD_FRAMES * 2);
        let transport = Transport::new(clock, sequence.frame_rate, sequence.duration());
        let player = Self {
            sequence,
            transport,
            audio,
            frames: FrameSource::new(8),
            samples: SampleCache::new(SAMPLE_RATE),
        };
        (player, sink)
    }

    /// Register media. Video-only or audio-only files are fine.
    pub fn add_media(
        &mut self,
        media: MediaId,
        video: Option<Box<dyn Decoder>>,
        audio: Option<Box<dyn Decoder>>,
    ) -> Result<()> {
        if let Some(d) = video {
            self.frames.add(media, d)?;
        }
        if let Some(d) = audio {
            self.samples.add(media, d)?;
        }
        Ok(())
    }

    pub fn set_track_mix(&mut self, track: TrackId, mix: TrackMix) {
        self.audio.set_track_mix(track, mix);
    }

    pub fn play(&mut self) {
        self.transport.play();
        self.audio.resync();
    }

    pub fn pause(&mut self) {
        self.transport.pause();
        self.audio.resync();
    }

    pub fn seek(&mut self, t: Rational) {
        self.transport.seek(t);
        self.audio.resync();
    }

    pub fn shuttle(&mut self, forward: bool) {
        self.transport.shuttle(forward);
        self.audio.resync();
    }

    pub fn step(&mut self, n: i64) {
        self.transport.step(n);
        self.audio.resync();
    }

    pub fn stats(&self) -> Stats {
        self.transport.stats()
    }

    /// Call once per display refresh. Keeps audio rendered ahead and returns the
    /// graph for the frame to present, or `None` if the displayed frame is current.
    pub fn tick(&mut self) -> Result<Option<Graph>> {
        if self.transport.is_playing() {
            self.audio.fill_ahead(
                &self.sequence,
                &mut self.samples,
                LOOK_AHEAD_FRAMES,
                BLOCK_FRAMES,
            )?;
        }
        let Some(frame) = self.transport.tick() else {
            return Ok(None);
        };
        let t = self.transport.frame_rate().frame_to_time(frame);
        Ok(Some(compose(&self.sequence, t, &self.frames)))
    }

    /// The graph for the current frame regardless of whether it changed (e.g. after
    /// an edit while paused).
    pub fn current_graph(&self) -> Graph {
        let t = self
            .transport
            .frame_rate()
            .frame_to_time(self.transport.current_frame());
        compose(&self.sequence, t, &self.frames)
    }
}
