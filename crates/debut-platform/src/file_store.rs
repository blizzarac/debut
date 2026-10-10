//! Project and media storage (MED-06, MED-09, NFR-13).
//! Desktop: native FS with memory-mapped reads. Browser: OPFS + File System Access API.

use debut_core::Result;

pub trait FileStore: Send + Sync {
    fn read(&self, path: &str) -> Result<Vec<u8>>;
    fn write(&self, path: &str, data: &[u8]) -> Result<()>;
    fn exists(&self, path: &str) -> bool;
    fn list(&self, dir: &str) -> Result<Vec<String>>;

    /// Whether `path` is a directory (for walking folders).
    fn is_dir(&self, _path: &str) -> bool {
        false
    }

    /// Copy a file, creating the destination's folders; returns the bytes
    /// copied. Stores override this to stream instead of loading the file.
    fn copy(&self, from: &str, to: &str) -> Result<u64> {
        let data = self.read(from)?;
        self.write(to, &data)?;
        Ok(data.len() as u64)
    }

    /// A 64-bit content checksum (FNV-1a), to verify copies (MED-02).
    fn checksum(&self, path: &str) -> Result<u64> {
        let mut h = Fnv64::new();
        h.update(&self.read(path)?);
        Ok(h.finish())
    }
}

/// FNV-1a, 64-bit: a streaming checksum every store computes the same way.
pub struct Fnv64(u64);

impl Fnv64 {
    pub fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub fn update(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

impl Default for Fnv64 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv64_matches_the_reference_values() {
        let hash = |b: &[u8]| {
            let mut h = Fnv64::new();
            h.update(b);
            h.finish()
        };
        assert_eq!(hash(b""), 0xcbf29ce484222325);
        assert_eq!(hash(b"a"), 0xaf63dc4c8601ec8c);
        // Streaming in pieces gives the same value.
        let mut h = Fnv64::new();
        h.update(b"foo");
        h.update(b"bar");
        assert_eq!(h.finish(), hash(b"foobar"));
    }
}
