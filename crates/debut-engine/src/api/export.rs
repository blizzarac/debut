//! The export queue, its worker thread and codec capabilities (EXP-01 .. EXP-03, NFR-09).

use super::*;
use debut_platform::{HdrSettings, HdrTransfer};

pub(crate) type ExportSpec = (
    Preset,
    Option<f32>,
    Vec<(MediaId, String)>,
    Vec<Sequence>,
    Option<String>,
    // Smart render: the media whose packets may be copied (no input colour
    // conversion), when asked for.
    Option<std::collections::HashSet<MediaId>>,
);

/// What an FCPXML import brought in.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct XmlImportDto {
    /// The sequence now open in the timeline.
    pub sequence: String,
    pub name: String,
    /// Projects imported (compound clips come along as nested sequences).
    pub sequences: usize,
    pub clips: usize,
    pub media_added: usize,
    /// Media files that could not be opened (offline until relinked).
    pub missing: Vec<String>,
    /// Elements not imported (multicam, auditions, …).
    pub skipped: Vec<String>,
}

#[derive(Serialize)]
pub struct PresetDto {
    pub name: String,
    pub loudness_lufs: f32,
    /// "pq" or "hlg" for HDR presets.
    pub hdr: Option<String>,
}

#[derive(Serialize)]
pub struct ExportStatusDto {
    pub id: u64,
    pub name: String,
    pub output: String,
    pub state: String,
    pub frames_done: u64,
    pub frames_total: u64,
    pub loudness_lufs: Option<f32>,
    pub true_peak_db: f32,
    /// Frames copied from a source by a smart render (EXP-05).
    pub frames_copied: u64,
    /// Measured MaxCLL / MaxFALL in nits (HDR exports).
    pub max_cll: Option<f32>,
    pub max_fall: Option<f32>,
    pub error: Option<String>,
}

#[derive(Serialize, Debug, Clone)]
pub struct HwEncoderDto {
    pub name: String,
    pub codec: String,
    pub api: String,
}

#[derive(Serialize, Debug, Clone)]
pub struct CodecCapabilitiesDto {
    pub hardware_encoders: Vec<HwEncoderDto>,
    pub hardware_decoders: Vec<String>,
}

impl Session {
    pub fn export_presets(&self) -> Vec<PresetDto> {
        Preset::all()
            .into_iter()
            .map(|p| PresetDto {
                name: p.name,
                loudness_lufs: p.loudness_lufs,
                hdr: p.hdr.map(|t| match t {
                    HdrTransfer::Pq => "pq".to_string(),
                    HdrTransfer::Hlg => "hlg".to_string(),
                }),
            })
            .collect()
    }

    /// Queue an export of the whole sequence and make sure a worker is running.
    /// What this machine can accelerate (NFR-09): hardware encoders that
    /// really open, and hardware decoders compiled into FFmpeg.
    pub fn codec_capabilities(&self) -> CodecCapabilitiesDto {
        CodecCapabilitiesDto {
            hardware_encoders: self
                .platform
                .hardware_encoders()
                .into_iter()
                .map(|e| HwEncoderDto {
                    name: e.name,
                    codec: e.codec.to_string(),
                    api: e.api.to_string(),
                })
                .collect(),
            hardware_decoders: self.platform.hardware_decoders(),
        }
    }

    pub fn export_start(
        &mut self,
        output: String,
        preset: &str,
        normalize: Option<f32>,
        caption_sidecar: bool,
        hardware: bool,
    ) -> Result<u64, String> {
        self.export_start_with(output, preset, normalize, caption_sidecar, hardware, false)
    }

    /// `export_start`, optionally as a smart render (EXP-05): stretches that
    /// show one untouched source frame for frame are copied from it, in its
    /// codec, and the rest is rendered and encoded to match where the codec
    /// allows. Falls back to a normal render (and says why in the job's
    /// name) when the sequence or its sources do not allow it.
    pub fn export_start_with(
        &mut self,
        output: String,
        preset: &str,
        normalize: Option<f32>,
        caption_sidecar: bool,
        hardware: bool,
        smart: bool,
    ) -> Result<u64, String> {
        let preset = Preset::all()
            .into_iter()
            .find(|p| p.name == preset)
            .ok_or_else(|| format!("unknown preset {preset}"))?;
        self.note_feature(&format!("export:{}", preset.name));
        if smart {
            self.note_feature("smart_render");
        }
        // Pick a hardware encoder for the preset's codec family when asked and
        // one is available; the encoder falls back to software if it cannot open.
        let encoder: Option<String> = if hardware {
            let family = match preset.codec {
                debut_export::VideoCodec::Hevc => "hevc",
                _ => "h264",
            };
            self.platform
                .hardware_encoders()
                .into_iter()
                .find(|e| e.codec == family)
                .map(|e| e.name)
        } else {
            None
        };
        let seq = self.first_sequence()?.clone();
        if seq.duration() <= Rational::ZERO {
            return Err("the sequence is empty".into());
        }
        // Captions as a sidecar next to the movie (GFX-06), written up front:
        // they do not depend on the render.
        if caption_sidecar && !seq.captions.is_empty() {
            let stem = match output.rfind('.') {
                Some(i) if !output[i..].contains('/') => &output[..i],
                _ => output.as_str(),
            };
            self.export_srt(&format!("{stem}.srt"))?;
        }
        let job = ExportJob {
            sequence: seq,
            range: (Rational::ZERO, self.first_sequence()?.duration()),
            sample_rate: 48_000,
            gain_db: 0.0,
            hdr: preset.hdr,
            plan: Vec::new(),
        };
        let media: Vec<(MediaId, String)> = self
            .project()
            .map(|p| p.media.iter().map(|m| (m.id, m.path.clone())).collect())
            .unwrap_or_default();
        let id = self
            .exports
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .submit(
                match &encoder {
                    Some(e) => format!("{} · {e}", preset.name),
                    None => preset.name.clone(),
                },
                job,
                output,
                0,
            );
        let sequences = self
            .project()
            .map(|p| p.sequences.clone())
            .unwrap_or_default();
        let smart = smart.then(|| {
            self.project()
                .map(|p| {
                    p.media
                        .iter()
                        .filter(|m| {
                            matches!(
                                m.metadata.color_space,
                                None | Some(debut_core::color::ColorSpace::Rec709)
                            )
                        })
                        .map(|m| m.id)
                        .collect()
                })
                .unwrap_or_default()
        });
        let spec = (preset, normalize, media, sequences, encoder, smart);
        if self
            .export_worker
            .load(std::sync::atomic::Ordering::Acquire)
        {
            if let Some(shared) = &self.export_specs_shared {
                shared
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id, spec);
            }
        } else {
            self.export_specs.insert(id, spec);
            self.spawn_export_worker();
        }
        Ok(id.0)
    }

    pub(crate) fn spawn_export_worker(&mut self) {
        if self
            .export_worker
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return;
        }
        let queue = Arc::clone(&self.exports);
        let running = Arc::clone(&self.export_worker);
        let platform = Arc::clone(&self.platform);
        let plugins = self.plugin_host();
        let specs = std::mem::take(&mut self.export_specs);
        let specs = Arc::new(Mutex::new(specs));
        self.export_specs_shared = Some(Arc::clone(&specs));
        let spawner = Arc::clone(&self.platform);
        let spawned = spawner.spawn(
            "debut-export",
            Box::new(move || {
                loop {
                    let next = queue.lock().unwrap_or_else(|e| e.into_inner()).take_next();
                    let Some((id, mut job, output, control)) = next else {
                        break;
                    };
                    let spec = specs.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                    let result = (|| -> Result<debut_export::Progress, String> {
                        let (preset, normalize, media, sequences, encoder, smart) =
                            spec.ok_or("missing export spec")?;
                        let mut frames = crate::FrameSource::new(4);
                        frames.set_platform(Arc::clone(&platform));
                        frames.set_plugins(plugins.clone());
                        let mut samples = crate::SampleCache::new(48_000);
                        samples.set_plugins(plugins.clone());
                        frames.set_sequences(&sequences);
                        samples.set_sequences(&sequences);
                        for (mid, path) in &media {
                            // Offline media export as the slate, like in the viewer.
                            let Ok(dec) = platform.open_decoder(path) else {
                                continue;
                            };
                            let has_audio = dec.audio_info().is_some();
                            if dec.video_info().is_some() {
                                frames.add(*mid, dec).map_err(|e| e.to_string())?;
                            }
                            if has_audio {
                                let audio =
                                    platform.open_decoder(path).map_err(|e| e.to_string())?;
                                samples.add(*mid, audio).map_err(|e| e.to_string())?;
                            }
                        }
                        if let Some(target) = normalize {
                            let (lufs, tp) =
                                measure_loudness(&job, &mut samples).map_err(|e| e.to_string())?;
                            if let Some(lufs) = lufs {
                                job.gain_db = 20.0 * normalize_gain(lufs, target, tp, -1.0).log10();
                            }
                        }
                        let mut template = None;
                        if let Some(allowed) = smart {
                            let note = match smart_setup(
                                platform.as_ref(),
                                &job,
                                &media,
                                &allowed,
                                &output,
                                preset.hdr.is_some(),
                            ) {
                                Ok((plan, path)) => {
                                    let note = format!(
                                        "smart: {:.0}% copied",
                                        debut_export::smart::copied_fraction(&plan) * 100.0
                                    );
                                    job.plan = plan;
                                    template = Some(path);
                                    note
                                }
                                Err(why) => format!("smart off: {why}"),
                            };
                            queue
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .rename(id, |n| format!("{n} · {note}"));
                        }
                        let (w, h) = (job.sequence.width, job.sequence.height);
                        let mut encoder = platform
                            .create_encoder(
                                &output,
                                EncodeSettings {
                                    width: w,
                                    height: h,
                                    frame_rate: job.sequence.frame_rate,
                                    crf: preset.quality.max(1),
                                    audio: Some(AudioEncodeSettings {
                                        channels: 2,
                                        sample_rate: 48_000,
                                        bitrate: preset.audio_bitrate.max(96_000),
                                    }),
                                    encoder,
                                    hdr: preset.hdr.map(|t| match t {
                                        HdrTransfer::Pq => HdrSettings::hdr10(),
                                        HdrTransfer::Hlg => HdrSettings::hlg(),
                                    }),
                                    smart: template,
                                },
                            )
                            .map_err(|e| e.to_string())?;
                        if encoder.used_fallback() {
                            queue
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .rename(id, |n| {
                                    format!("{n} · fell back to {}", encoder.encoder_name())
                                });
                        }
                        let mut backend = AnyBackend::detect();
                        let q = Arc::clone(&queue);
                        let progress = match &mut backend {
                            AnyBackend::Cpu(b) => export(
                                &job,
                                b,
                                &mut frames,
                                &mut samples,
                                encoder.as_mut(),
                                &control,
                                |p| {
                                    q.lock()
                                        .unwrap_or_else(|e| e.into_inner())
                                        .report_progress(id, p)
                                },
                            ),
                            AnyBackend::Gpu(b) => export(
                                &job,
                                b.as_mut(),
                                &mut frames,
                                &mut samples,
                                encoder.as_mut(),
                                &control,
                                |p| {
                                    q.lock()
                                        .unwrap_or_else(|e| e.into_inner())
                                        .report_progress(id, p)
                                },
                            ),
                        }
                        .map_err(|e| e.to_string())?;
                        encoder.finish().map_err(|e| e.to_string())?;
                        Ok(progress)
                    })();
                    queue
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .finish(id, result);
                }
                running.store(false, std::sync::atomic::Ordering::Release);
            }),
        );
        if spawned.is_err() {
            // No worker: let the next export try again; queued jobs stay queued.
            self.export_worker
                .store(false, std::sync::atomic::Ordering::Release);
        }
    }

    pub fn export_status(&self) -> Vec<ExportStatusDto> {
        let q = self.exports.lock().unwrap_or_else(|e| e.into_inner());
        q.entries()
            .map(|e| ExportStatusDto {
                id: e.id.0,
                name: e.name.clone(),
                output: e.output.clone(),
                state: match e.state {
                    JobState::Queued => "queued",
                    JobState::Running => "running",
                    JobState::Paused => "paused",
                    JobState::Done => "done",
                    JobState::Failed => "failed",
                    JobState::Cancelled => "cancelled",
                }
                .into(),
                frames_done: e.progress.frames_done,
                frames_total: e.progress.frames_total,
                loudness_lufs: e.progress.loudness_lufs,
                true_peak_db: e.progress.true_peak_db,
                frames_copied: e.progress.frames_copied,
                max_cll: e.progress.light.map(|l| l.max_cll),
                max_fall: e.progress.light.map(|l| l.max_fall),
                error: e.error.clone(),
            })
            .collect()
    }

    pub fn export_control(&self, id: u64, action: &str) {
        let mut q = self.exports.lock().unwrap_or_else(|e| e.into_inner());
        match action {
            "pause" => q.pause(JobId(id)),
            "resume" => q.resume(JobId(id)),
            _ => q.cancel(JobId(id)),
        }
    }

    /// Read a Final Cut Pro XML file (MED-12): its projects become sequences
    /// (compound clips nested ones) and its assets media, matched to the
    /// project's by path, all in one undo step. The first imported sequence
    /// opens in the timeline; files that cannot be opened show as offline.
    pub fn import_fcpxml(&mut self, path: &str) -> Result<XmlImportDto, String> {
        let bytes = self.store.read(path).map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&bytes);
        let existing = self.project().ok_or("no project open")?.media.clone();
        let imported = debut_media::fcpxml_import::import(&text, &mut self.ids, &existing)
            .map_err(|e| e.to_string())?;
        let first = imported.sequences.first().ok_or("nothing to import")?;
        let report = XmlImportDto {
            sequence: id_str(first.id.0),
            name: first.name.clone(),
            sequences: imported.projects,
            clips: imported
                .sequences
                .iter()
                .flat_map(|s| &s.tracks)
                .map(|t| t.clips.len())
                .sum(),
            media_added: imported.media.len(),
            missing: Vec::new(),
            skipped: imported.skipped.clone(),
        };
        let first_id = first.id;
        let mut cmds: Vec<Command> = imported
            .media
            .iter()
            .cloned()
            .map(Command::AddMedia)
            .collect();
        cmds.extend(imported.sequences.into_iter().map(Command::AddSequence));
        self.exec(Command::Group(cmds))?;
        let mut report = report;
        for m in &imported.media {
            match self.platform.open_decoder(&m.path) {
                Ok(dec) => {
                    if let Some(info) = media_info(dec.as_ref()) {
                        self.probed.insert(m.id, info);
                    }
                    self.offline.remove(&m.id);
                }
                Err(_) => {
                    self.offline.insert(m.id);
                    report.missing.push(m.path.clone());
                }
            }
        }
        self.active = Some(first_id);
        self.player = None;
        self.sync_player()?;
        Ok(report)
    }

    /// Write the active sequence for another application (MED-12): `"edl"`
    /// is CMX3600 for the first video and audio track, `"otio"`
    /// OpenTimelineIO JSON, `"fcpxml"` Final Cut Pro XML and `"aaf"` an AAF
    /// file (Avid, Pro Tools, Resolve) for the whole sequence.
    pub fn export_interchange(&self, path: &str, format: &str) -> Result<(), String> {
        let project = self.project().ok_or("no project open")?;
        let seq = self.first_sequence()?;
        let bytes = match format {
            "edl" => debut_media::interchange::edl(seq, &project.media).into_bytes(),
            "otio" => debut_media::interchange::otio(seq, project).into_bytes(),
            "fcpxml" => {
                let durations = self.probed.iter().map(|(id, p)| (*id, p.2)).collect();
                debut_media::fcpxml::fcpxml(seq, project, &durations).into_bytes()
            }
            "aaf" => {
                // A 0-pixel picture means sound only.
                let media = self
                    .probed
                    .iter()
                    .map(|(id, &(width, height, duration, has_audio))| {
                        let info = debut_media::aaf::AafMedia {
                            width,
                            height,
                            duration,
                            has_video: width > 0,
                            has_audio,
                        };
                        (*id, info)
                    })
                    .collect();
                debut_media::aaf::aaf(seq, project, &media, Default::default())?
            }
            other => return Err(format!("unknown interchange format {other}")),
        };
        self.store.write(path, &bytes).map_err(|e| e.to_string())
    }
}

/// Plan a smart render for `job` (EXP-05): which stretches to copy, and the
/// source whose codec the output takes. `Err` says why it falls back to a
/// normal render.
fn smart_setup(
    platform: &dyn Platform,
    job: &ExportJob,
    media: &[(MediaId, String)],
    allowed: &std::collections::HashSet<MediaId>,
    output: &str,
    hdr: bool,
) -> Result<(Vec<debut_export::smart::Span>, String), String> {
    use debut_export::smart::{plan, SourceFacts};
    if hdr {
        return Err("HDR output is always rendered".into());
    }
    let path_of = |m: MediaId| media.iter().find(|(id, _)| *id == m).map(|(_, p)| p);
    // Facts for the media on the video tracks only.
    let mut facts = std::collections::HashMap::new();
    for track in &job.sequence.tracks {
        for clip in &track.clips {
            if let ClipSource::Media(m) = clip.source {
                if facts.contains_key(&m) || !allowed.contains(&m) {
                    continue;
                }
                let info = path_of(m)
                    .and_then(|p| platform.open_decoder(p).ok())
                    .and_then(|d| d.video_info().cloned());
                facts.insert(
                    m,
                    info.map(|v| SourceFacts {
                        width: v.width,
                        height: v.height,
                        frame_rate: v.frame_rate,
                        variable_frame_rate: v.variable_frame_rate,
                    }),
                );
            }
        }
    }
    let mut spans = plan(&job.sequence, job.range, |m| {
        facts.get(&m).copied().flatten()
    });
    let mut infos = std::collections::HashMap::new();
    for span in &mut spans {
        if let Some(copy) = &mut span.copy {
            let path = path_of(copy.media).cloned().unwrap_or_default();
            infos
                .entry(copy.media)
                .or_insert_with(|| platform.stream_copy_info(&path).ok());
            copy.path = path;
        }
    }
    // The first copied source sets the output codec; others join it only
    // with identical stream parameters.
    let first = spans
        .iter()
        .find_map(|s| s.copy.as_ref())
        .ok_or("nothing in the sequence can be copied")?;
    let template = infos
        .get(&first.media)
        .cloned()
        .flatten()
        .ok_or("the source's stream cannot be read for copying")?;
    let template_path = first.path.clone();
    for span in &mut spans {
        let same = span.copy.as_ref().is_some_and(|c| {
            infos
                .get(&c.media)
                .cloned()
                .flatten()
                .is_some_and(|i| i.fingerprint == template.fingerprint)
        });
        if !same {
            span.copy = None;
        }
    }
    let ext = output.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if template.intra_only && !matches!(ext.as_str(), "mov" | "mkv") {
        return Err(format!(
            "{} copies into .mov or .mkv, not .{ext}",
            template.codec
        ));
    }
    if !template.can_match {
        // Long-GOP: everything must be copied, cut on keyframes.
        let near = |t: Rational| {
            template
                .keyframes
                .iter()
                .any(|k| (*k - t).as_f64().abs() < 1e-3)
        };
        for span in &spans {
            let Some((from, to)) = span.source_range() else {
                return Err(format!(
                    "{} frames cannot be rendered to match; every stretch must be copied",
                    template.codec
                ));
            };
            if !near(from) || !(near(to) || to >= template.duration) {
                return Err(format!(
                    "{} cuts must fall on keyframes ({:.2}s to {:.2}s does not)",
                    template.codec,
                    from.as_f64(),
                    to.as_f64()
                ));
            }
        }
    }
    Ok((spans, template_path))
}
