//! One sequence playing: transport + audio renderer + frame source, driven by two
//! threads the shell owns — the device callback (gets the [`RtSink`]) and the UI
//! refresh (calls [`Player::tick`]).

use crate::frames::FrameSource;
use crate::playback::{Stats, Transport};
use crate::samples::SampleCache;
use debut_audio::{AudioRenderer, Clock, RtSink};
use debut_core::{MediaId, Rational, Result};
use debut_platform::Decoder;
use debut_project::Sequence;
use debut_render::compose::{compose_at, SourceInfo};
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
    /// Preview resolution divisor: 1 = full, 2 = half, 4 = quarter (PB-03).
    preview_divisor: u32,
}

impl SourceInfo for FrameSource {
    fn dimensions(&self, media: MediaId) -> (u32, u32) {
        FrameSource::dimensions(self, media).unwrap_or(crate::frames::OFFLINE_SIZE)
    }
    fn title(&self, title: &debut_project::Title) -> Option<Arc<debut_render::Image8>> {
        FrameSource::title(self, title)
    }
    fn sequence(&self, id: debut_core::SequenceId) -> Option<Arc<Sequence>> {
        FrameSource::sequence(self, id)
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
            preview_divisor: 1,
        };
        (player, sink)
    }

    /// Tell both frame and sample sources about every sequence in the project so
    /// compound clips resolve (TL-07). Call after any edit that touches sequences.
    pub fn set_sequences(&mut self, all: &[Sequence]) {
        self.frames.set_sequences(all);
        self.samples.set_sequences(all);
    }

    /// Forget a media's decoders so the next `add_media` reloads it (relink).
    pub fn forget_media(&mut self, media: MediaId) {
        self.frames.remove(media);
        self.samples.remove(media);
    }

    /// Whether `media` has decoders here already (picture or sound).
    pub fn has_media(&self, media: MediaId) -> bool {
        self.frames.has(media) || self.samples.has(media)
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

    pub fn preview_divisor(&self) -> u32 {
        self.preview_divisor
    }

    pub fn set_preview_divisor(&mut self, d: u32) {
        self.preview_divisor = d.clamp(1, 8);
    }

    /// Canvas the viewer renders at.
    pub fn preview_canvas(&self) -> (u32, u32) {
        let d = self.preview_divisor;
        (
            (self.sequence.width / d).max(16) & !1,
            (self.sequence.height / d).max(16) & !1,
        )
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
        Ok(Some(compose_at(
            &self.sequence,
            t,
            &self.frames,
            self.preview_canvas(),
        )))
    }

    /// The graph for the current frame regardless of whether it changed (e.g. after
    /// an edit while paused).
    /// The current frame composed at `canvas` (a program window's size).
    pub fn graph_at_size(&self, canvas: (u32, u32)) -> Graph {
        let t = self
            .transport
            .frame_rate()
            .frame_to_time(self.transport.current_frame());
        compose_at(&self.sequence, t, &self.frames, canvas)
    }

    pub fn sequence_size(&self) -> (u32, u32) {
        (self.sequence.width, self.sequence.height)
    }

    pub fn current_graph(&self) -> Graph {
        let t = self
            .transport
            .frame_rate()
            .frame_to_time(self.transport.current_frame());
        compose_at(&self.sequence, t, &self.frames, self.preview_canvas())
    }
}
