//! Native thread pool: a fixed set of workers over a shared queue.

use debut_platform::Threads;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

type Job = Box<dyn FnOnce() + Send>;

pub struct NativeThreads {
    sender: mpsc::Sender<Job>,
    workers: usize,
}

impl NativeThreads {
    pub fn new(workers: usize) -> Self {
        let workers = workers.max(1);
        let (sender, receiver) = mpsc::channel::<Job>();
        let receiver = Arc::new(Mutex::new(receiver));
        for i in 0..workers {
            let rx = Arc::clone(&receiver);
            thread::Builder::new()
                .name(format!("debut-worker-{i}"))
                .spawn(move || loop {
                    let job = rx.lock().unwrap().recv();
                    match job {
                        Ok(job) => job(),
                        Err(_) => break,
                    }
                })
                .expect("spawn worker");
        }
        Self { sender, workers }
    }
}

impl Default for NativeThreads {
    fn default() -> Self {
        Self::new(
            thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
        )
    }
}

impl Threads for NativeThreads {
    fn spawn(&self, job: Job) {
        self.sender.send(job).expect("worker pool alive");
    }

    fn parallelism(&self) -> usize {
        self.workers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn runs_jobs_on_workers() {
        let pool = NativeThreads::new(3);
        let done = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel();
        for _ in 0..30 {
            let done = Arc::clone(&done);
            let tx = tx.clone();
            pool.spawn(Box::new(move || {
                done.fetch_add(1, Ordering::SeqCst);
                tx.send(()).unwrap();
            }));
        }
        for _ in 0..30 {
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        }
        assert_eq!(done.load(Ordering::SeqCst), 30);
    }
}
