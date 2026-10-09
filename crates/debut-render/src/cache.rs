//! Render cache keyed by graph content hash (PB-06). A rendered frame stays valid
//! for as long as the graph that produced it hashes the same, whatever the user
//! did elsewhere on the timeline. Eviction is least-recently-used by frame count.

use std::collections::HashMap;

pub struct RenderCache<I> {
    entries: HashMap<u64, (u64, I)>,
    capacity: usize,
    clock: u64,
}

impl<I: Clone> RenderCache<I> {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            capacity: capacity.max(1),
            clock: 0,
        }
    }

    pub fn get(&mut self, key: u64) -> Option<I> {
        self.clock += 1;
        let clock = self.clock;
        self.entries.get_mut(&key).map(|(used, img)| {
            *used = clock;
            img.clone()
        })
    }

    pub fn insert(&mut self, key: u64, img: I) {
        self.clock += 1;
        if self.entries.len() >= self.capacity && !self.entries.contains_key(&key) {
            if let Some(&oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (used, _))| *used)
                .map(|(k, _)| k)
            {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(key, (self.clock, img));
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_least_recently_used() {
        let mut c = RenderCache::new(2);
        c.insert(1, "a");
        c.insert(2, "b");
        assert_eq!(c.get(1), Some("a")); // touch 1 so 2 is the oldest
        c.insert(3, "c");
        assert_eq!(c.get(2), None);
        assert_eq!(c.get(1), Some("a"));
        assert_eq!(c.get(3), Some("c"));
    }
}
