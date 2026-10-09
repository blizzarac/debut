//! Project and media storage (MED-06, MED-09, NFR-13).
//! Desktop: native FS with memory-mapped reads. Browser: OPFS + File System Access API.

use debut_core::Result;

pub trait FileStore: Send + Sync {
    fn read(&self, path: &str) -> Result<Vec<u8>>;
    fn write(&self, path: &str, data: &[u8]) -> Result<()>;
    fn exists(&self, path: &str) -> bool;
    fn list(&self, dir: &str) -> Result<Vec<String>>;
}
