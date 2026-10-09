//! Single-producer / single-consumer lock-free ring of `f32` samples. The consumer
//! is the real-time callback: `pop` never blocks, locks or allocates.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct Inner {
    buf: Box<[UnsafeCell<f32>]>,
    /// Next index to write (producer-owned, consumer reads).
    head: AtomicUsize,
    /// Next index to read (consumer-owned, producer reads).
    tail: AtomicUsize,
}

// Each slot is written only by the producer before `head` is published and read
// only by the consumer before `tail` is published, so no slot is shared at once.
unsafe impl Send for Inner {}
unsafe impl Sync for Inner {}

pub struct Producer(Arc<Inner>);
pub struct Consumer(Arc<Inner>);

/// Create a ring holding up to `capacity` samples.
pub fn ring(capacity: usize) -> (Producer, Consumer) {
    let cap = capacity.max(1) + 1; // one slot kept empty to tell full from empty
    let buf = (0..cap)
        .map(|_| UnsafeCell::new(0.0))
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let inner = Arc::new(Inner {
        buf,
        head: AtomicUsize::new(0),
        tail: AtomicUsize::new(0),
    });
    (Producer(Arc::clone(&inner)), Consumer(inner))
}

impl Inner {
    fn len(&self, head: usize, tail: usize) -> usize {
        (head + self.buf.len() - tail) % self.buf.len()
    }
}

impl Producer {
    pub fn capacity(&self) -> usize {
        self.0.buf.len() - 1
    }

    /// Samples currently queued.
    pub fn len(&self) -> usize {
        self.0.len(
            self.0.head.load(Ordering::Acquire),
            self.0.tail.load(Ordering::Acquire),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn free(&self) -> usize {
        self.capacity() - self.len()
    }

    /// Append as many of `data` as fit; returns how many were written.
    pub fn push(&mut self, data: &[f32]) -> usize {
        let cap = self.0.buf.len();
        let head = self.0.head.load(Ordering::Relaxed);
        let tail = self.0.tail.load(Ordering::Acquire);
        let free = cap - 1 - self.0.len(head, tail);
        let n = data.len().min(free);
        for (i, &s) in data[..n].iter().enumerate() {
            // SAFETY: slots in (head, head + n) are free (see Inner).
            unsafe { *self.0.buf[(head + i) % cap].get() = s };
        }
        self.0.head.store((head + n) % cap, Ordering::Release);
        n
    }

    /// Drop everything queued. Only the producer may call this, and only while the
    /// consumer is treating the ring as empty-able (e.g. around a seek).
    pub fn clear(&mut self) {
        let head = self.0.head.load(Ordering::Acquire);
        self.0.tail.store(head, Ordering::Release);
    }
}

impl Consumer {
    pub fn len(&self) -> usize {
        self.0.len(
            self.0.head.load(Ordering::Acquire),
            self.0.tail.load(Ordering::Acquire),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Fill `out` from the ring; returns how many samples were available. The rest
    /// of `out` is untouched.
    pub fn pop(&mut self, out: &mut [f32]) -> usize {
        let cap = self.0.buf.len();
        let head = self.0.head.load(Ordering::Acquire);
        let tail = self.0.tail.load(Ordering::Relaxed);
        let n = out.len().min(self.0.len(head, tail));
        for (i, o) in out[..n].iter_mut().enumerate() {
            // SAFETY: slots in (tail, tail + n) were published by the producer.
            *o = unsafe { *self.0.buf[(tail + i) % cap].get() };
        }
        self.0.tail.store((tail + n) % cap, Ordering::Release);
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_pop_wraps_and_reports_counts() {
        let (mut p, mut c) = ring(4);
        assert_eq!(p.push(&[1.0, 2.0, 3.0, 4.0, 5.0]), 4);
        assert_eq!(p.free(), 0);
        let mut out = [0.0; 2];
        assert_eq!(c.pop(&mut out), 2);
        assert_eq!(out, [1.0, 2.0]);
        assert_eq!(p.push(&[5.0, 6.0]), 2);
        let mut out = [0.0; 8];
        assert_eq!(c.pop(&mut out), 4);
        assert_eq!(&out[..4], &[3.0, 4.0, 5.0, 6.0]);
        assert!(c.is_empty());
    }

    #[test]
    fn streams_in_order_across_threads() {
        let (mut p, mut c) = ring(64);
        let total = 100_000usize;
        let producer = std::thread::spawn(move || {
            let mut i = 0usize;
            while i < total {
                let chunk: Vec<f32> = (i..(i + 7).min(total)).map(|v| v as f32).collect();
                let n = p.push(&chunk);
                i += n;
                if n == 0 {
                    std::thread::yield_now();
                }
            }
        });
        let mut expect = 0usize;
        let mut buf = [0.0; 13];
        while expect < total {
            let n = c.pop(&mut buf);
            for &s in &buf[..n] {
                assert_eq!(s, expect as f32);
                expect += 1;
            }
            if n == 0 {
                std::thread::yield_now();
            }
        }
        producer.join().unwrap();
    }
}
