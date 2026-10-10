//! Request handling: describes plugins, keeps one OpenFX instance per filter
//! and one CLAP instance per audio insert, and runs them.

use crate::ffi::{self, ClapInstance, OfxDescriptor, OfxInstance};
use debut_platform::plugin_host::{PluginInfo, PluginKind, PluginRef, Reply, Request};
use std::collections::HashMap;

/// Largest audio block handed to a CLAP plugin at once.
const MAX_FRAMES: u32 = 4096;

#[derive(Default)]
pub struct Host {
    descriptors: HashMap<(String, u32), OfxDescriptor>,
    /// One instance per filter: the app sends every parameter with every
    /// frame, so instances hold no state worth keeping apart.
    filters: HashMap<(String, u32), (f64, OfxInstance)>,
    inserts: HashMap<u64, (PluginRef, u32, ClapInstance)>,
}

/// Bytes of samples that follow a request's header line.
pub fn payload_len(r: &Request) -> Option<usize> {
    match r {
        Request::Video { bytes, .. } | Request::Audio { bytes, .. } => Some(*bytes),
        _ => None,
    }
}

fn to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn to_bytes(samples: &[f32]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_ne_bytes()).collect()
}

impl Host {
    pub fn handle(&mut self, request: Request, payload: Vec<u8>) -> (Reply, Option<Vec<u8>>) {
        let result = match request {
            Request::List { dirs } => Ok((
                Reply::Listed {
                    files: crate::scan::list(&dirs),
                },
                None,
            )),
            Request::Describe { kind, path } => self
                .describe(kind, &path)
                .map(|plugins| (Reply::Described { plugins }, None)),
            Request::Video { job, .. } => {
                let mut px = to_f32(&payload);
                self.video(&job, &mut px).map(|()| {
                    let out = to_bytes(&px);
                    (Reply::Processed { bytes: out.len() }, Some(out))
                })
            }
            Request::Audio { job, .. } => {
                let mut samples = to_f32(&payload);
                self.audio(&job, &mut samples).map(|()| {
                    let out = to_bytes(&samples);
                    (Reply::Processed { bytes: out.len() }, Some(out))
                })
            }
            Request::Release { instance } => {
                self.inserts.remove(&instance);
                Ok((Reply::Done, None))
            }
        };
        result.unwrap_or_else(|error| (Reply::Failed { error }, None))
    }

    fn describe(&mut self, kind: PluginKind, path: &str) -> Result<Vec<PluginInfo>, String> {
        let count = match kind {
            PluginKind::OpenFx => ffi::ofx_count(path)?,
            PluginKind::Clap => ffi::clap_count(path)?,
        };
        let mut plugins = Vec::new();
        let mut last_error = None;
        for index in 0..count {
            let described = match kind {
                PluginKind::OpenFx => self.descriptor(path, index).map(|d| (d.0, d.1, d.2)),
                PluginKind::Clap => {
                    ffi::clap_open(path, index, 0.0, 0).map(|d| (d.id, d.name, d.params))
                }
            };
            match described {
                Ok((id, name, params)) => plugins.push(PluginInfo {
                    plugin: PluginRef {
                        kind,
                        path: path.to_string(),
                        index,
                    },
                    id,
                    name,
                    params,
                }),
                Err(e) => last_error = Some(e),
            }
        }
        match (plugins.is_empty(), last_error) {
            (true, Some(e)) => Err(e),
            (true, None) => Err(format!("{path} contains no plugins")),
            _ => Ok(plugins),
        }
    }

    /// Describe (once) and remember an OpenFX plugin.
    fn descriptor(
        &mut self,
        path: &str,
        index: u32,
    ) -> Result<
        (
            String,
            String,
            Vec<debut_platform::plugin_host::PluginParamInfo>,
        ),
        String,
    > {
        let d = ffi::ofx_describe(path, index)?;
        self.descriptors.insert((path.to_string(), index), d.handle);
        Ok((d.id, d.name, d.params))
    }

    fn video(
        &mut self,
        job: &debut_platform::plugin_host::VideoJob,
        px: &mut [f32],
    ) -> Result<(), String> {
        let key = (job.plugin.path.clone(), job.plugin.index);
        if !self.descriptors.contains_key(&key) {
            self.descriptor(&job.plugin.path, job.plugin.index)?;
        }
        let stale = self
            .filters
            .get(&key)
            .is_none_or(|(fps, _)| *fps != job.fps);
        if stale {
            let instance = self.descriptors[&key].instance(job.fps)?;
            self.filters.insert(key.clone(), (job.fps, instance));
        }
        let (_, instance) = self.filters.get_mut(&key).expect("just made");
        for (name, v) in &job.params {
            if !instance.set(name, *v) {
                return Err(format!("the plugin has no parameter {name:?}"));
            }
        }
        instance.render(job.frame, job.fps, job.width, job.height, px)
    }

    fn audio(
        &mut self,
        job: &debut_platform::plugin_host::AudioJob,
        samples: &mut [f32],
    ) -> Result<(), String> {
        let stale = self
            .inserts
            .get(&job.instance)
            .is_none_or(|(p, rate, _)| *p != job.plugin || *rate != job.sample_rate);
        if stale {
            self.inserts.remove(&job.instance);
            let d = ffi::clap_open(
                &job.plugin.path,
                job.plugin.index,
                job.sample_rate as f64,
                MAX_FRAMES,
            )?;
            self.inserts.insert(
                job.instance,
                (job.plugin.clone(), job.sample_rate, d.handle),
            );
        }
        let (_, _, instance) = self.inserts.get_mut(&job.instance).expect("just made");
        for (name, v) in &job.params {
            if !instance.set(name, *v) {
                return Err(format!("the plugin has no parameter {name:?}"));
            }
        }
        instance.process(samples, job.channels)
    }
}
