//! CLAP track inserts (AUD-09): a [`Processor`] that sends each block to the
//! out-of-process plugin host. Each built insert is its own plugin instance,
//! released when the insert is dropped. A plugin that fails (or crashes the
//! host) is bypassed until the next `reset` (a stop or seek), so a broken
//! plugin costs one error, not one helper restart per block.

use crate::effects::{AudioEffect, Processor, CHANNELS};
use debut_platform::plugin_host::{AudioJob, PluginKind, PluginRef};
use debut_platform::PluginHost;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

struct PluginInsert {
    host: Arc<dyn PluginHost>,
    job: AudioJob,
    failed: bool,
}

/// Leaves audio untouched (a plugin insert with no host to run it).
struct Bypass;

impl Processor for Bypass {
    fn process(&mut self, _buf: &mut [f32]) {}
    fn reset(&mut self) {}
}

/// The processor for an `AudioEffect::Plugin`; anything else, or no host, bypasses.
pub fn build(
    effect: &AudioEffect,
    sample_rate: u32,
    host: Option<Arc<dyn PluginHost>>,
) -> Box<dyn Processor> {
    let (
        AudioEffect::Plugin {
            path,
            index,
            params,
            ..
        },
        Some(host),
    ) = (effect, host)
    else {
        return Box::new(Bypass);
    };
    Box::new(PluginInsert {
        host,
        job: AudioJob {
            plugin: PluginRef {
                kind: PluginKind::Clap,
                path: path.clone(),
                index: *index,
            },
            instance: NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed),
            sample_rate,
            channels: CHANNELS as u32,
            params: params.iter().map(|p| (p.name.clone(), p.value)).collect(),
        },
        failed: false,
    })
}

impl Processor for PluginInsert {
    fn process(&mut self, buf: &mut [f32]) {
        if self.failed || buf.is_empty() {
            return;
        }
        let before = buf.to_vec();
        if self.host.process_audio(&self.job, buf).is_err() {
            buf.copy_from_slice(&before);
            self.failed = true;
        }
    }

    fn reset(&mut self) {
        self.failed = false;
    }
}

impl Drop for PluginInsert {
    fn drop(&mut self) {
        self.host.release(self.job.instance);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::{Error, Result};
    use debut_platform::plugin_host::{ScanResult, VideoJob};
    use debut_project::AudioPluginParam;
    use std::sync::Mutex;

    /// A host whose "plugin" multiplies by its Gain parameter, or fails.
    #[derive(Default)]
    struct FakeHost {
        calls: Mutex<u32>,
        released: Mutex<Vec<u64>>,
        fail: bool,
    }

    impl PluginHost for FakeHost {
        fn scan(&self) -> Result<ScanResult> {
            Ok(ScanResult::default())
        }
        fn process_video(&self, _: &VideoJob, _: &mut [f32]) -> Result<()> {
            Ok(())
        }
        fn process_audio(&self, job: &AudioJob, samples: &mut [f32]) -> Result<()> {
            *self.calls.lock().unwrap() += 1;
            if self.fail {
                samples.fill(9.0); // garbage the insert must undo
                return Err(Error::Other("crashed".into()));
            }
            let g = job.params[0].1 as f32;
            samples.iter_mut().for_each(|s| *s *= g);
            Ok(())
        }
        fn release(&self, instance: u64) {
            self.released.lock().unwrap().push(instance);
        }
    }

    fn gain(v: f64) -> AudioEffect {
        AudioEffect::Plugin {
            path: "/x.clap".into(),
            index: 0,
            id: "x".into(),
            name: "X".into(),
            params: vec![AudioPluginParam {
                name: "Gain".into(),
                min: 0.0,
                max: 4.0,
                value: v,
            }],
        }
    }

    #[test]
    fn runs_through_the_host_and_releases_its_instance() {
        let host = Arc::new(FakeHost::default());
        let mut p = build(&gain(2.0), 48_000, Some(host.clone()));
        let mut buf = vec![0.25f32; 8];
        p.process(&mut buf);
        assert_eq!(buf, vec![0.5; 8]);
        drop(p);
        assert_eq!(host.released.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_failing_plugin_is_bypassed_until_reset() {
        let host = Arc::new(FakeHost {
            fail: true,
            ..Default::default()
        });
        let mut p = build(&gain(2.0), 48_000, Some(host.clone()));
        let mut buf = vec![0.25f32; 8];
        p.process(&mut buf);
        assert_eq!(buf, vec![0.25; 8], "the block comes back unprocessed");
        p.process(&mut buf);
        assert_eq!(*host.calls.lock().unwrap(), 1, "not retried every block");
        p.reset();
        p.process(&mut buf);
        assert_eq!(*host.calls.lock().unwrap(), 2);
    }

    #[test]
    fn without_a_host_audio_passes_through() {
        let mut p = build(&gain(2.0), 48_000, None);
        let mut buf = vec![0.25f32; 4];
        p.process(&mut buf);
        assert_eq!(buf, vec![0.25; 4]);
    }
}
