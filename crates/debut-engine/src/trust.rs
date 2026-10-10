//! Plugin approval (NFR-13). A project names plugin binaries by path, so
//! opening someone else's project could otherwise run any native code it
//! points at. Every plugin call goes through [`TrustedHost`], which runs a
//! binary only when its SHA-256 matches one the user approved; a changed
//! file needs approving again. Adding a plugin from the scan list counts as
//! approval. Scanning itself (the user's own plugin folders) is not gated.

use debut_core::{Error, Result};
use debut_platform::plugin_host::{AudioJob, PluginHost, ScanResult, VideoJob};
use debut_platform::FileStore;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, RwLock};

/// Approved binaries: path -> SHA-256 (hex).
pub type Trusted = Arc<RwLock<BTreeMap<String, String>>>;

/// SHA-256 of a file, hex.
pub fn sha256(store: &dyn FileStore, path: &str) -> Result<String> {
    let bytes = store.read(path)?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

pub struct TrustedHost {
    inner: Arc<dyn PluginHost>,
    store: Arc<dyn FileStore>,
    trusted: Trusted,
    /// Hashes taken this session, so a binary is read once, not per frame.
    hashes: Mutex<HashMap<String, String>>,
}

impl TrustedHost {
    pub fn new(inner: Arc<dyn PluginHost>, store: Arc<dyn FileStore>, trusted: Trusted) -> Self {
        Self {
            inner,
            store,
            trusted,
            hashes: Mutex::new(HashMap::new()),
        }
    }

    /// Forget cached hashes (after approvals change or files may have).
    pub fn rehash(&self) {
        self.hashes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    fn hash(&self, path: &str) -> Result<String> {
        let mut hashes = self.hashes.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(h) = hashes.get(path) {
            return Ok(h.clone());
        }
        let h = sha256(self.store.as_ref(), path)?;
        hashes.insert(path.to_string(), h.clone());
        Ok(h)
    }

    /// Ok when `path` is approved and unchanged since.
    pub fn check(&self, path: &str) -> Result<()> {
        let pinned = self
            .trusted
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(path)
            .cloned();
        let Some(pinned) = pinned else {
            return Err(Error::Unsupported(format!(
                "plugin not approved: {path} (approve it to run it)"
            )));
        };
        if self.hash(path)? != pinned {
            return Err(Error::Unsupported(format!(
                "plugin changed since it was approved: {path} (approve it again to run it)"
            )));
        }
        Ok(())
    }
}

impl PluginHost for TrustedHost {
    fn scan(&self) -> Result<ScanResult> {
        self.inner.scan()
    }

    fn process_video(&self, job: &VideoJob, rgba: &mut [f32]) -> Result<()> {
        self.check(&job.plugin.path)?;
        self.inner.process_video(job, rgba)
    }

    fn process_audio(&self, job: &AudioJob, samples: &mut [f32]) -> Result<()> {
        self.check(&job.plugin.path)?;
        self.inner.process_audio(job, samples)
    }

    fn release(&self, instance: u64) {
        self.inner.release(instance)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods, clippy::disallowed_types)] // tests use the OS directly
    use super::*;
    use debut_platform::plugin_host::{PluginKind, PluginRef};

    struct Counting(Mutex<u32>);

    impl PluginHost for Counting {
        fn scan(&self) -> Result<ScanResult> {
            Ok(ScanResult::default())
        }
        fn process_video(&self, _: &VideoJob, _: &mut [f32]) -> Result<()> {
            *self.0.lock().unwrap() += 1;
            Ok(())
        }
        fn process_audio(&self, _: &AudioJob, _: &mut [f32]) -> Result<()> {
            *self.0.lock().unwrap() += 1;
            Ok(())
        }
    }

    #[test]
    fn runs_only_approved_unchanged_binaries() {
        let dir = std::env::temp_dir().join(format!("debut-trust-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("fx.ofx");
        std::fs::write(&bin, b"plugin v1").unwrap();
        let path = bin.to_string_lossy().into_owned();
        let store: Arc<dyn FileStore> =
            Arc::new(debut_platform_native::file_store::NativeFileStore::new("/"));
        let inner = Arc::new(Counting(Mutex::new(0)));
        let trusted: Trusted = Default::default();
        let host = TrustedHost::new(inner.clone(), Arc::clone(&store), Arc::clone(&trusted));
        let job = VideoJob {
            plugin: PluginRef {
                kind: PluginKind::OpenFx,
                path: path.clone(),
                index: 0,
            },
            width: 1,
            height: 1,
            frame: 0.0,
            fps: 25.0,
            params: Vec::new(),
        };
        let mut px = [0f32; 4];
        let err = host.process_video(&job, &mut px).unwrap_err().to_string();
        assert!(err.contains("not approved"), "{err}");
        assert_eq!(*inner.0.lock().unwrap(), 0, "the plugin never ran");

        let hash = sha256(store.as_ref(), &path).unwrap();
        // SHA-256("plugin v1"), as sha256sum prints it.
        assert_eq!(
            hash,
            "9a9eb439cf6401aa6b0b6735c22d7c1580efb375a770ebf7a75011896439490a"
        );
        trusted.write().unwrap().insert(path.clone(), hash);
        host.process_video(&job, &mut px).unwrap();
        assert_eq!(*inner.0.lock().unwrap(), 1);

        // The file is swapped: refused until approved again.
        std::fs::write(&bin, b"plugin v2 (tampered)").unwrap();
        host.rehash();
        let err = host.process_video(&job, &mut px).unwrap_err().to_string();
        assert!(err.contains("changed since it was approved"), "{err}");
        assert_eq!(*inner.0.lock().unwrap(), 1);
        std::fs::remove_dir_all(dir).ok();
    }
}
