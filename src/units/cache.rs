//! Pictures drawn on demand and kept while they are in use.

use std::collections::HashMap;
use std::hash::Hash;

/// Frames a picture is kept after it was last asked for.
const KEPT_FRAMES: u64 = 3600;

/// Pictures by what they show, each with the frame it was last asked for. One
/// not asked for in a while is let go; a sprite still showing it holds its own
/// handle, so it stays until that sprite changes.
pub(super) struct ImageCache<K, V> {
    entries: HashMap<K, (Option<V>, u64)>,
    frame: u64,
}

impl<K, V> Default for ImageCache<K, V> {
    fn default() -> Self {
        ImageCache { entries: HashMap::new(), frame: 0 }
    }
}

pub(super) struct CacheEntry<'a, K, V> {
    cache: &'a mut ImageCache<K, V>,
    key: K,
}

impl<'a, K: Hash + Eq, V> CacheEntry<'a, K, V> {
    pub(super) fn or_insert_with(self, make: impl FnOnce() -> Option<V>) -> &'a mut Option<V> {
        self.or_insert_with_key(|_| make())
    }

    pub(super) fn or_insert_with_key(self, make: impl FnOnce(&K) -> Option<V>) -> &'a mut Option<V> {
        let frame = self.cache.frame;
        let slot = self.cache.entries.entry(self.key).or_insert_with_key(|key| (make(key), frame));
        slot.1 = frame;
        &mut slot.0
    }
}

impl<K: Hash + Eq, V> ImageCache<K, V> {
    pub(super) fn entry(&mut self, key: K) -> CacheEntry<'_, K, V> {
        CacheEntry { cache: self, key }
    }

    /// Moves on a frame, now and then letting go of what has not been used.
    pub(super) fn step(&mut self) {
        self.frame += 1;
        if self.frame.is_multiple_of(600) {
            let frame = self.frame;
            self.entries.retain(|_, (_, used)| frame - *used <= KEPT_FRAMES);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ImageCache;

    #[test]
    fn pictures_not_asked_for_are_let_go() {
        let mut cache: ImageCache<u8, u8> = ImageCache::default();
        assert_eq!(*cache.entry(1).or_insert_with(|| Some(10)), Some(10));
        // Made once only.
        assert_eq!(*cache.entry(1).or_insert_with(|| Some(11)), Some(10));
        for _ in 0..3000 {
            cache.step();
        }
        cache.entry(2).or_insert_with(|| Some(20));
        for _ in 0..1800 {
            cache.step();
        }
        assert!(!cache.entries.contains_key(&1));
        assert!(cache.entries.contains_key(&2));
    }
}
