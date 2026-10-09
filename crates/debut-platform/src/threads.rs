//! Thread pool abstraction. Desktop: native threads. Browser: Web Workers + SharedArrayBuffer.

pub trait Threads: Send + Sync {
    fn spawn(&self, job: Box<dyn FnOnce() + Send>);
    fn parallelism(&self) -> usize;
}
