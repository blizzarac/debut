//! Multicam clips, audio sync and angle switching (MED-11, TL-08).

use super::*;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct MulticamSyncDto {
    /// Per-angle head offsets in seconds (angle order).
    pub offsets: Vec<f64>,
    /// Sync confidence per angle after the first, 0..1; empty without sync.
    pub confidences: Vec<f32>,
}

/// How a multicam clip lines its angles up (MED-11).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncBy {
    /// Every angle from its first frame.
    Start,
    /// Cross-correlate each angle's audio with the first's.
    Audio,
    /// Start timecodes: angles meet where their timecodes overlap.
    Timecode,
}

impl Session {
    /// Insert a multicam clip of `media` (two or more angles, angle 0 active) on
    /// the first video track and, when every angle has audio, on the first
    /// audio track, at `at` seconds. Its length is the shortest angle (MED-11).
    pub fn add_multicam(&mut self, at: f64, media: Vec<String>) -> Result<(), String> {
        self.add_multicam_synced(at, media, false).map(|_| ())
    }

    /// Mono audio of the first `seconds` of `media` at 48 kHz, for sync.
    pub(crate) fn audio_head(&self, media: MediaId, seconds: f64) -> Result<Vec<f32>, String> {
        let path = self
            .project()
            .and_then(|p| p.media.iter().find(|m| m.id == media))
            .map(|m| m.path.clone())
            .ok_or("unknown media")?;
        let dec = self
            .platform
            .open_decoder(&path)
            .map_err(|e| e.to_string())?;
        if dec.audio_info().is_none() {
            return Err(format!("{path} has no audio to sync on"));
        }
        let sr = 48_000u32;
        let mut cache = crate::SampleCache::new(sr);
        cache.add(media, dec).map_err(|e| e.to_string())?;
        let frames = (seconds * sr as f64) as usize;
        let mut buf = Vec::new();
        let ch =
            debut_audio::SampleSource::read(&mut cache, media, Rational::ZERO, frames, &mut buf)
                .map_err(|e| e.to_string())? as usize;
        Ok(buf
            .chunks(ch.max(1))
            .map(|f| f.iter().sum::<f32>() / ch.max(1) as f32)
            .collect())
    }

    /// `add_multicam`, optionally aligning every angle to the first by audio
    /// (MED-11). Returns the per-angle offsets in seconds and the sync
    /// confidences (empty when not syncing).
    pub fn add_multicam_synced(
        &mut self,
        at: f64,
        media: Vec<String>,
        sync: bool,
    ) -> Result<MulticamSyncDto, String> {
        let by = if sync { SyncBy::Audio } else { SyncBy::Start };
        self.add_multicam_by(at, media, by)
    }

    /// Insert a multicam clip with its angles lined up `by` start, audio or
    /// timecode. Returns the per-angle offsets in seconds and, for audio, the
    /// sync confidences.
    pub fn add_multicam_by(
        &mut self,
        at: f64,
        media: Vec<String>,
        by: SyncBy,
    ) -> Result<MulticamSyncDto, String> {
        if media.len() < 2 {
            return Err("a multicam clip needs at least two angles".into());
        }
        let angles = media
            .iter()
            .map(|m| Ok(MediaId(parse_id(m)?)))
            .collect::<Result<Vec<_>, String>>()?;
        let (seq_id, fr, video, audio) = {
            let seq = self.first_sequence()?;
            let first = |kind| seq.tracks.iter().find(|t| t.kind == kind).map(|t| t.id);
            (
                seq.id,
                seq.frame_rate,
                first(TrackKind::Video).ok_or("no video track")?,
                first(TrackKind::Audio),
            )
        };
        let mut duration = Rational::from_int(i64::MAX / 4);
        let mut all_audio = true;
        for m in &angles {
            let p = self.probed.get(m).ok_or("unknown media")?;
            duration = duration.min(p.2);
            all_audio &= p.3;
        }
        // Audio sync: offsets relative to angle 0, each angle's usable length
        // shrinks by what it must skip at the head.
        let mut offsets = vec![Rational::ZERO; angles.len()];
        let mut confidences = Vec::new();
        if by == SyncBy::Timecode {
            // Start of each take in seconds of the day; the latest start is
            // where every angle has material.
            let mut starts = Vec::with_capacity(angles.len());
            for m in &angles {
                let r = self
                    .project()
                    .and_then(|p| p.media.iter().find(|r| r.id == *m))
                    .ok_or("unknown media")?;
                let tc = r.metadata.start_timecode.ok_or_else(|| {
                    format!(
                        "{} has no timecode",
                        r.path.rsplit('/').next().unwrap_or(&r.path)
                    )
                })?;
                let rate = r.metadata.frame_rate.unwrap_or(fr);
                starts.push(rate.frame_to_time(tc.to_frames(rate)));
            }
            let latest = starts.iter().copied().fold(Rational::ZERO, Rational::max);
            for (i, start) in starts.iter().enumerate() {
                offsets[i] = fr.snap(latest - *start);
            }
            for (i, m) in angles.iter().enumerate() {
                let p = self.probed.get(m).ok_or("unknown media")?;
                duration = duration.min(p.2 - offsets[i]);
            }
            if duration <= Rational::ZERO {
                return Err("the takes' timecodes do not overlap".into());
            }
        }
        if by == SyncBy::Audio {
            const HEAD_SECONDS: f64 = 60.0;
            const MAX_OFFSET_S: f32 = 30.0;
            // Read at most a minute, and never past the end of a short take.
            let head = |s: &Self, m: &MediaId| {
                let len = s
                    .probed
                    .get(m)
                    .map(|p| p.2.as_f64())
                    .unwrap_or(HEAD_SECONDS);
                s.audio_head(*m, len.min(HEAD_SECONDS))
            };
            let reference = head(self, &angles[0])?;
            for (i, m) in angles.iter().enumerate().skip(1) {
                let other = head(self, m)?;
                let al = debut_audio::align(&reference, &other, 48_000, MAX_OFFSET_S)
                    .ok_or("recordings are too short to sync")?;
                // `other` lags by `offset`: it must start that far in.
                offsets[i] = fr.snap(Rational::new(al.offset_samples, 48_000));
                confidences.push(al.confidence);
            }
            // A negative offset means the reference lags: shift everything so the
            // smallest offset is zero and no angle starts before its material.
            let min = offsets.iter().copied().fold(Rational::ZERO, Rational::min);
            for o in &mut offsets {
                *o -= min;
            }
            for (i, m) in angles.iter().enumerate() {
                let p = self.probed.get(m).ok_or("unknown media")?;
                duration = duration.min(p.2 - offsets[i]);
            }
        }
        let duration = fr.snap(duration).max(fr.frame_duration());
        let result = MulticamSyncDto {
            offsets: offsets.iter().map(|o| secs(*o)).collect(),
            confidences,
        };
        let source = ClipSource::Multicam {
            angles,
            active: 0,
            offsets,
        };
        let mut cmds = Vec::new();
        let mut targets = vec![video];
        if let (Some(a), true) = (audio, all_audio) {
            targets.push(a);
        }
        for track in targets {
            let clip = Clip::new(
                self.ids.fresh(),
                source.clone(),
                Rational::ZERO,
                duration,
                Rational::ZERO,
            );
            let target = Target {
                sequence: seq_id,
                track,
            };
            cmds.push(Command::insert(
                target,
                frames_of(at, fr),
                vec![clip],
                &mut self.ids,
            ));
        }
        self.exec(Command::Group(cmds))?;
        Ok(result)
    }

    /// Switch a multicam clip to `angle`. With `cut` and the playhead strictly
    /// inside the clip, blade there first so only the part after the playhead
    /// switches (the live-switch gesture, TL-08). Returns the clip that switched.
    pub fn switch_angle(
        &mut self,
        track: &str,
        clip: &str,
        angle: usize,
        cut: bool,
    ) -> Result<String, String> {
        let (target, clip_id, c) = self.clip_ref(track, clip)?;
        let ClipSource::Multicam {
            angles, offsets, ..
        } = &c.source
        else {
            return Err("not a multicam clip".into());
        };
        if angle >= angles.len() {
            return Err(format!("no angle {angle}"));
        }
        let source = ClipSource::Multicam {
            angles: angles.clone(),
            active: angle,
            offsets: offsets.clone(),
        };
        let at = self.playhead();
        let mut cmds = Vec::new();
        let mut switched = clip_id;
        if cut && c.timeline_in < at && at < c.timeline_out() {
            let tail_id: ClipId = self.ids.fresh();
            cmds.push(Command::Blade {
                target,
                at,
                tail_id,
            });
            switched = tail_id;
        }
        cmds.push(Command::SetClipSource {
            target,
            clip: switched,
            source,
        });
        self.exec(Command::Group(cmds))?;
        Ok(id_str(switched.0))
    }
}
