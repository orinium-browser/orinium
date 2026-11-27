//! Epoch-scoped cache of CSS selector match results.
//!
//! Selector matching is the dominant per-element cost after the rule set has
//! been built: every layout run walks the candidate rules of every element and
//! runs `ComplexSelector::matches` again, even though the result only depends on
//! the (immutable) DOM snapshot and the (immutable) rule set. Repeated layout
//! runs over an unchanged document — scroll, resize, hover and animation frames
//! — therefore repeat exactly the same matches.
//!
//! [`SelectorMatchCache`] memoizes `(selector, element)` pairs. The key is the
//! identity of the [`ComplexSelector`] allocation ([`Arc::as_ptr`]) rather than
//! the selector text: rules sharing a selector group share one `Arc`, so the
//! pointer is stable for as long as the owning [`RuleSet`] is reused, and this
//! avoids hashing selector trees on every lookup.
//!
//! ## Invalidation contract
//!
//! The cache is scoped to one DOM-snapshot/rule-set/interaction epoch.
//! [`SelectorMatchCache::clear`] MUST be called whenever any input that can
//! affect selector matching changes:
//!
//! - The rule set is rebuilt (new selector allocations).
//! - The DOM snapshot changes (`dom_version` bumps): structural changes,
//!   attribute changes, pseudo-class state changes (:checked, :focus, etc.).
//! - Interaction state changes (`interaction_version` bumps): :hover, :active,
//!   :focus-within, etc.
//!
//! Entries carry the epoch they were created in. [`SelectorMatchCache::get_or_insert`]
//! returns a miss for entries from a previous epoch, so even if `clear` is
//! accidentally skipped, stale entries are never reused. `clear` also drops the
//! backing map to bound memory growth.
//!
//! [`SelectorMatchCache`] is single-threaded: it is owned by a layout worker
//! and accessed only from that worker's thread.

use std::collections::HashMap;
use std::sync::Arc;

use super::dom_snapshot::NodeId;
use crate::engine::cache_stats::{CacheStats, CacheStatsSnapshot};
use crate::engine::css::parser::ComplexSelector;

/// Default number of memoized `(selector, element)` results.
// Entries are a pointer, a node id, a boolean and an epoch, so this bounds
// the cache at a few megabytes even for very large documents.
pub const DEFAULT_CAPACITY: usize = 131_072;

/// `(selector allocation, element, stylesheet scope)` identity of one cached
/// match result. The scope is part of the key because one selector can be
/// declared both by the document and by a shadow tree's stylesheet, and those
/// rules match different elements.
type MatchKey = (usize, NodeId, Option<u64>);

/// Memoizes selector match results for one DOM/rule-set epoch.
// Uses an epoch-scoped `HashMap` with an admission cap rather than an LRU.
// Once the cache reaches `capacity`, new entries are not inserted and the
// lookup is recorded as a skip so we can distinguish capacity pressure from
// cache misses that could be served by the map.
// Each entry stores the epoch it was created in. [`clear`] bumps the epoch,
// making all existing entries stale. [`get_or_insert`] treats entries from a
// previous epoch as misses, so stale results are never reused even if
// [`clear`] is inadvertently skipped. [`clear`] also empties the map to bound
// memory growth from stale entries.
pub struct SelectorMatchCache {
    entries: HashMap<MatchKey, (bool, u64)>,
    epoch: u64,
    capacity: usize,
    stats: CacheStats,
}

impl Default for SelectorMatchCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectorMatchCache {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            entries: HashMap::with_capacity(capacity.min(4096)),
            epoch: 0,
            capacity,
            stats: CacheStats::new("selector_match"),
        }
    }

    /// Returns the cached match result for `selector` against `node_id`,
    /// computing and storing it with `compute` on a miss.
    ///
    /// Entries from a previous epoch are treated as misses: they are never
    /// returned, even if they happen to still be in the map.
    pub fn get_or_insert(
        &mut self,
        selector: &Arc<ComplexSelector>,
        node_id: NodeId,
        origin_shadow_root: Option<u64>,
        compute: impl FnOnce() -> bool,
    ) -> bool {
        let key = (Arc::as_ptr(selector) as usize, node_id, origin_shadow_root);
        if let Some(&(matched, entry_epoch)) = self.entries.get(&key)
            && entry_epoch == self.epoch
        {
            self.stats.record_hit();
            return matched;
        }
        // Stale entry from a previous epoch: fall through to recompute.

        self.stats.record_miss();
        let matched = compute();
        if self.entries.len() < self.capacity {
            self.entries.insert(key, (matched, self.epoch));
        } else {
            self.stats.record_skip();
        }
        matched
    }

    /// Drops every entry and bumps the epoch. Call when the DOM snapshot or
    /// the rule set changes.
    ///
    /// After this call, all previously cached results are stale: [`get_or_insert`]
    /// will treat them as misses because their epoch no longer matches.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.epoch = self.epoch.wrapping_add(1);
        self.stats.reset();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn stats(&self) -> CacheStatsSnapshot {
        self.stats.snapshot()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selector() -> Arc<ComplexSelector> {
        Arc::new(ComplexSelector { parts: Vec::new() })
    }

    #[test]
    fn a_second_lookup_reuses_the_stored_result() {
        let mut cache = SelectorMatchCache::new();
        let selector = selector();
        let mut computed = 0;

        let first = cache.get_or_insert(&selector, 7, None, || {
            computed += 1;
            true
        });
        let second = cache.get_or_insert(&selector, 7, None, || {
            computed += 1;
            false
        });

        assert!(first);
        assert!(second, "the cached result must win over the recomputation");
        assert_eq!(computed, 1);
        let stats = cache.stats();
        assert_eq!(
            (stats.hits, stats.misses, stats.evictions, stats.skips),
            (1, 1, 0, 0)
        );
    }

    #[test]
    fn elements_and_selectors_are_distinct_keys() {
        let mut cache = SelectorMatchCache::new();
        let first_selector = selector();
        let second_selector = selector();
        let mut computed = 0;

        cache.get_or_insert(&first_selector, 1, None, || {
            computed += 1;
            true
        });
        cache.get_or_insert(&first_selector, 2, None, || {
            computed += 1;
            false
        });
        cache.get_or_insert(&second_selector, 1, None, || {
            computed += 1;
            false
        });

        assert_eq!(computed, 3);
        let stats = cache.stats();
        assert_eq!(
            (stats.hits, stats.misses, stats.evictions, stats.skips),
            (0, 3, 0, 0)
        );
    }

    #[test]
    fn capacity_pressure_skips_without_eviction() {
        let mut cache = SelectorMatchCache::with_capacity(1);
        let first_selector = selector();
        let second_selector = selector();

        cache.get_or_insert(&first_selector, 1, None, || true);
        cache.get_or_insert(&second_selector, 1, None, || true);

        assert_eq!(cache.len(), 1);
        let stats = cache.stats();
        assert_eq!(stats.evictions, 0);
        assert_eq!(stats.skips, 1);
        assert_eq!(stats.hits, 0);

        // Existing entries remain cached.
        let mut computed = 0;
        let matched = cache.get_or_insert(&first_selector, 1, None, || {
            computed += 1;
            false
        });
        assert!(matched);
        assert_eq!(computed, 0);
        assert_eq!(cache.stats().hits, 1);
    }

    #[test]
    fn over_capacity_passes_hit_in_the_second_epoch_like_pass() {
        let mut cache = SelectorMatchCache::with_capacity(4);
        let selectors: Vec<Arc<ComplexSelector>> = (0..6)
            .map(|_| Arc::new(ComplexSelector { parts: Vec::new() }))
            .collect();
        let mut computed = 0;

        // First pass fills the cache and skips beyond capacity.
        for sel in &selectors {
            cache.get_or_insert(sel, 1, None, || {
                computed += 1;
                true
            });
        }
        assert_eq!(cache.len(), 4);
        let first = cache.stats();
        assert_eq!(first.misses, 6);
        assert_eq!(first.skips, 2);
        assert_eq!(first.evictions, 0);
        assert_eq!(computed, 6);

        // Second pass over the same keys: entries inserted in pass 1 must hit.
        // Keys beyond capacity were never stored, so they miss again.
        for sel in &selectors {
            cache.get_or_insert(sel, 1, None, || {
                computed += 1;
                false
            });
        }
        let second = cache.stats();
        assert_eq!(second.misses, 8);
        assert_eq!(second.hits, 4);
        assert_eq!(second.skips, 4);
        assert_eq!(computed, 8);
    }

    #[test]
    fn clear_drops_entries_and_resets_counters() {
        let mut cache = SelectorMatchCache::new();
        let selector = selector();
        cache.get_or_insert(&selector, 1, None, || true);
        cache.clear();

        assert!(cache.is_empty());
        let stats = cache.stats();
        assert_eq!(stats.misses, 0);
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.evictions, 0);
        assert_eq!(stats.skips, 0);
    }

    #[test]
    fn clear_bumps_epoch_so_stale_entries_are_not_reused() {
        let mut cache = SelectorMatchCache::new();
        let selector = selector();

        // First epoch: cache stores `true`.
        let first = cache.get_or_insert(&selector, 1, None, || true);
        assert!(first);

        // Bump the epoch without clearing the map (simulates a missed clear).
        cache.epoch = cache.epoch.wrapping_add(1);

        // The old entry is still in the map but must be treated as a miss.
        let second = cache.get_or_insert(&selector, 1, None, || false);
        assert!(!second, "stale epoch entry must not be reused");

        // Stats should show two misses (both lookups were misses due to epoch mismatch).
        let stats = cache.stats();
        assert_eq!(stats.misses, 2);
        assert_eq!(stats.hits, 0);
    }

    #[test]
    fn epoch_rollover_does_not_break_cache() {
        let mut cache = SelectorMatchCache::new();
        let selector = selector();

        // Bump epoch many times to test wraparound behavior.
        for _ in 0..10_000 {
            cache.epoch = cache.epoch.wrapping_add(1);
            cache.get_or_insert(&selector, 1, None, || true);
            cache.clear();
        }

        assert!(cache.is_empty());
    }
}
