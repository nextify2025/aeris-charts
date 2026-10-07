//! Stable image keys and bounded invalidation for anything the adapter hands to GPUI's atlas.
//!
//! GPUI keys its own atlas on `RenderImageParams { id, frame_index }`, where `id` is an opaque
//! `u64` the caller owns. That makes the *caller* responsible for id stability: reuse an id for
//! different pixels and GPUI serves the stale tile; mint a fresh id every frame and the atlas grows
//! without bound. This module owns that contract for Aeris.
//!
//! It is reserved for a Aeris-owned cached text or image path and is available to any future
//! prim that needs raster data. It is not part of the current GPUI-native text path. The module is
//! GPUI-free, so the id policy and eviction are testable without a window.

use std::collections::HashMap;
use std::hash::Hash;

/// Default budget in bytes for retained raster data (~8 MB: a few thousand label runs at chart
/// label sizes, an order of magnitude above any realistic frame's working set).
pub const DEFAULT_BYTE_BUDGET: usize = 8 * 1024 * 1024;

/// A GPUI atlas image id plus the pixel payload's size, as handed back to the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageHandle {
    /// Stable id for `gpui::RenderImageParams::id`. Unique per distinct key for the lifetime of
    /// the entry, and never reused for different pixels.
    pub id: u64,
    pub bytes: usize,
}

/// Why an entry left the cache — surfaced so a host can drop the matching GPUI image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Evicted {
    /// Pushed out by the byte budget.
    Budget(u64),
    /// Dropped by an explicit [`ImageCache::invalidate`].
    Invalidated(u64),
}

/// A bounded, LRU, byte-budgeted map from a caller-defined key to a stable GPUI image id.
///
/// `K` is whatever fully determines the pixels — for text that is
/// [`crate::text::TextKey`], which already covers font, color, alignment and subpixel phase.
pub struct ImageCache<K: Eq + Hash + Clone> {
    entries: HashMap<K, (ImageHandle, u64)>,
    /// Ids retired since the host last drained them, so it can release GPUI-side resources.
    retired: Vec<Evicted>,
    budget: usize,
    used: usize,
    tick: u64,
    next_id: u64,
    generation: u64,
}

impl<K: Eq + Hash + Clone> Default for ImageCache<K> {
    fn default() -> Self {
        Self::with_budget(DEFAULT_BYTE_BUDGET)
    }
}

impl<K: Eq + Hash + Clone> ImageCache<K> {
    pub fn with_budget(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            retired: Vec::new(),
            budget,
            used: 0,
            tick: 0,
            // Ids start at 1 so 0 is never a valid handle — a zeroed id is a bug, not a hit.
            next_id: 1,
            generation: 0,
        }
    }

    /// The handle for `key`, rasterizing through `rasterize` on a miss.
    ///
    /// `rasterize` returns the payload size in bytes; the caller uploads the pixels to GPUI under
    /// the returned id. On a hit the payload is already in GPUI's atlas and `rasterize` is not
    /// called.
    pub fn get_or_insert(&mut self, key: K, rasterize: impl FnOnce(u64) -> usize) -> ImageHandle {
        self.tick += 1;
        if let Some((handle, stamp)) = self.entries.get_mut(&key) {
            *stamp = self.tick;
            return *handle;
        }
        let id = self.next_id;
        self.next_id += 1;
        let bytes = rasterize(id);
        let handle = ImageHandle { id, bytes };
        self.used += bytes;
        self.entries.insert(key, (handle, self.tick));
        self.enforce_budget();
        handle
    }

    /// Evict least-recently-used entries until the byte budget is met.
    ///
    /// The most recently inserted entry is never evicted, so a single oversized payload cannot
    /// cause an infinite eviction loop — it is retained for this frame and reported through
    /// [`ImageCache::over_budget`].
    fn enforce_budget(&mut self) {
        while self.used > self.budget && self.entries.len() > 1 {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, (_, stamp))| *stamp)
                .map(|(k, (h, _))| (k.clone(), *h));
            let Some((key, handle)) = oldest else { break };
            self.entries.remove(&key);
            self.used = self.used.saturating_sub(handle.bytes);
            self.retired.push(Evicted::Budget(handle.id));
        }
    }

    /// Whether the cache is holding more than its budget (only possible with one oversized entry).
    pub fn over_budget(&self) -> bool {
        self.used > self.budget
    }

    /// Drop every entry, retiring all ids.
    ///
    /// Call on any change the key cannot observe: a font-set change, a DPR change, or a GPUI atlas
    /// reset. Ids are never reused after this, so a stale GPUI tile can never be resurrected.
    pub fn invalidate(&mut self) {
        for (handle, _) in self.entries.values() {
            self.retired.push(Evicted::Invalidated(handle.id));
        }
        self.entries.clear();
        self.used = 0;
        self.generation += 1;
    }

    /// Take the ids retired since the last drain, so the host can release them GPUI-side.
    pub fn drain_retired(&mut self) -> Vec<Evicted> {
        std::mem::take(&mut self.retired)
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Bytes currently retained.
    pub fn used_bytes(&self) -> usize {
        self.used
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_across_hits_and_never_zero() {
        let mut cache: ImageCache<&str> = ImageCache::default();
        let a = cache.get_or_insert("a", |_| 100);
        let again = cache.get_or_insert("a", |_| panic!("must not re-rasterize"));
        assert_eq!(a, again);
        assert_ne!(a.id, 0);
    }

    #[test]
    fn distinct_keys_get_distinct_ids() {
        let mut cache: ImageCache<&str> = ImageCache::default();
        let a = cache.get_or_insert("a", |_| 10);
        let b = cache.get_or_insert("b", |_| 10);
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn ids_are_not_reused_after_eviction() {
        let mut cache: ImageCache<u32> = ImageCache::with_budget(100);
        let first = cache.get_or_insert(1, |_| 60);
        let second = cache.get_or_insert(2, |_| 60);
        assert_eq!(cache.len(), 1, "the budget forced an eviction");
        assert_eq!(cache.drain_retired(), vec![Evicted::Budget(first.id)]);

        // Re-inserting the evicted key mints a NEW id, so GPUI can never serve the stale tile.
        let refetched = cache.get_or_insert(1, |_| 60);
        assert_ne!(refetched.id, first.id);
        assert_ne!(refetched.id, second.id);
    }

    #[test]
    fn budget_is_enforced_lru() {
        let mut cache: ImageCache<u32> = ImageCache::with_budget(250);
        for k in 0..3 {
            cache.get_or_insert(k, |_| 100);
        }
        // 300 bytes over a 250 budget -> the oldest (key 0) is dropped.
        assert_eq!(cache.len(), 2);
        assert!(!cache.over_budget());
        assert_eq!(cache.used_bytes(), 200);
        assert_eq!(cache.drain_retired().len(), 1);
    }

    #[test]
    fn touching_an_entry_protects_it_from_eviction() {
        let mut cache: ImageCache<u32> = ImageCache::with_budget(250);
        cache.get_or_insert(0, |_| 100);
        cache.get_or_insert(1, |_| 100);
        cache.get_or_insert(0, |_| panic!("hit"));
        cache.get_or_insert(2, |_| 100);
        assert!(cache.len() == 2);
        let mut rerasterized = false;
        cache.get_or_insert(0, |_| {
            rerasterized = true;
            100
        });
        assert!(!rerasterized, "key 0 was touched and must have survived");
    }

    #[test]
    fn one_oversized_entry_is_retained_and_flagged_not_looped_on() {
        let mut cache: ImageCache<u32> = ImageCache::with_budget(10);
        cache.get_or_insert(1, |_| 5_000);
        assert_eq!(cache.len(), 1);
        assert!(cache.over_budget());
    }

    #[test]
    fn invalidate_retires_every_id_and_bumps_the_generation() {
        let mut cache: ImageCache<u32> = ImageCache::default();
        let a = cache.get_or_insert(1, |_| 10);
        let b = cache.get_or_insert(2, |_| 10);
        let before = cache.generation();

        cache.invalidate();
        assert!(cache.is_empty());
        assert_eq!(cache.used_bytes(), 0);
        assert_eq!(cache.generation(), before + 1);

        let mut retired = cache.drain_retired();
        retired.sort_by_key(|e| match e {
            Evicted::Budget(id) | Evicted::Invalidated(id) => *id,
        });
        assert_eq!(
            retired,
            vec![Evicted::Invalidated(a.id), Evicted::Invalidated(b.id)]
        );
    }

    #[test]
    fn drain_retired_is_idempotent() {
        let mut cache: ImageCache<u32> = ImageCache::default();
        cache.get_or_insert(1, |_| 10);
        cache.invalidate();
        assert_eq!(cache.drain_retired().len(), 1);
        assert!(cache.drain_retired().is_empty());
    }

    #[test]
    fn rasterize_receives_the_id_it_must_upload_under() {
        let mut cache: ImageCache<u32> = ImageCache::default();
        let mut seen = 0;
        let handle = cache.get_or_insert(7, |id| {
            seen = id;
            42
        });
        assert_eq!(seen, handle.id);
        assert_eq!(handle.bytes, 42);
    }
}
