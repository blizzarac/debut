//! Native file system (MED-06, MED-09). Memory-mapped reads come with the media
//! cache work.

use debut_core::{Error, Result};
use debut_platform::FileStore;
use std::path::PathBuf;

pub struct NativeFileStore {
    root: PathBuf,
}

impl NativeFileStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn resolve(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }
}

fn io(e: std::io::Error) -> Error {
    Error::Other(e.to_string())
}

impl FileStore for NativeFileStore {
    fn read(&self, path: &str) -> Result<Vec<u8>> {
        std::fs::read(self.resolve(path)).map_err(io)
    }

    fn write(&self, path: &str, data: &[u8]) -> Result<()> {
        let full = self.resolve(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        // Write-then-rename so a crash never leaves a half-written project (NFR-05).
        let tmp = full.with_extension("tmp");
        std::fs::write(&tmp, data).map_err(io)?;
        std::fs::rename(&tmp, &full).map_err(io)
    }

    fn exists(&self, path: &str) -> bool {
        self.resolve(path).exists()
    }

    fn list(&self, dir: &str) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(self.resolve(dir)).map_err(io)? {
            let entry = entry.map_err(io)?;
            out.push(entry.file_name().to_string_lossy().into_owned());
        }
        out.sort();
        Ok(out)
    }

    fn is_dir(&self, path: &str) -> bool {
        self.resolve(path).is_dir()
    }

    fn copy(&self, from: &str, to: &str) -> Result<u64> {
        let dst = self.resolve(to);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        std::fs::copy(self.resolve(from), dst).map_err(io)
    }

    fn size(&self, path: &str) -> Result<u64> {
        std::fs::metadata(self.resolve(path))
            .map(|m| m.len())
            .map_err(io)
    }

    fn checksum(&self, path: &str) -> Result<u64> {
        use std::io::Read;
        let mut f = std::fs::File::open(self.resolve(path)).map_err(io)?;
        let mut h = debut_platform::file_store::Fnv64::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = f.read(&mut buf).map_err(io)?;
            if n == 0 {
                return Ok(h.finish());
            }
            h.update(&buf[..n]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_atomically_and_lists() {
        let dir = std::env::temp_dir().join(format!("debut-fs-{}", std::process::id()));
        let fs = NativeFileStore::new(&dir);
        fs.write("a/b.txt", b"hi").unwrap();
        assert!(fs.exists("a/b.txt"));
        assert!(!fs.exists("a/b.tmp"));
        assert_eq!(fs.read("a/b.txt").unwrap(), b"hi");
        assert_eq!(fs.list("a").unwrap(), vec!["b.txt".to_string()]);
        assert!(fs.is_dir("a") && !fs.is_dir("a/b.txt"));
        assert_eq!(fs.copy("a/b.txt", "c/d/e.txt").unwrap(), 2);
        assert_eq!(
            fs.checksum("c/d/e.txt").unwrap(),
            fs.checksum("a/b.txt").unwrap()
        );
        // The streaming checksum matches the default (whole-file) one.
        let mut h = debut_platform::file_store::Fnv64::new();
        h.update(b"hi");
        assert_eq!(fs.checksum("a/b.txt").unwrap(), h.finish());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
