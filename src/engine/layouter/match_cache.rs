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
//! The caller owns invalidation. Entries are only valid for one
//! DOM-snapshot/rule-set epoch, so [`SelectorMatchCache::clear`] must run
//! whenever either input changes.

use std::collections::HashMap;
use std::sync::Arc;

use super::dom_snapshot::NodeId;
use crate::engine::cache_stats::{CacheStats, CacheStatsSnapshot};
use crate::engine::css::parser::ComplexSelector;

/// Default number of memoized `(selector, element)` results.
///
/// Entries are a pointer, a node id and a boolean, so this bounds the cache at
/// a few megabytes even for very large documents.
pub const DEFAULT_CAPACITY: usize = 131_072;

/// `(selector allocation, element)` identity of one cached match result.
type MatchKey = (usize, NodeId);

/// Memoizes selector match results for one DOM/rule-set epoch.
///
/// Uses an epoch-scoped `HashMap` with an admission cap rather than an LRU.
/// Once the cache reaches `capacity`, new entries are not inserted and the
/// lookup is recorded as a skip so we can distinguish capacity pressure from
/// cache misses that could be served by the map.
pub struct SelectorMatchCache {
    entries: HashMap<MatchKey, bool>,
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
            capacity,
            stats: CacheStats::new("selector_match"),
        }
    }

    /// Returns the cached match result for `selector` against `node_id`,
    /// computing and storing it with `compute` on a miss.
    pub fn get_or_insert(
        &mut self,
        selector: &Arc<ComplexSelector>,
        node_id: NodeId,
        compute: impl FnOnce() -> bool,
    ) -> bool {
        let key = (Arc::as_ptr(selector) as usize, node_id);
        if let Some(&matched) = self.entries.get(&key) {
            self.stats.record_hit();
            return matched;
        }

        self.stats.record_miss();
        let matched = compute();
        if self.entries.len() < self.capacity {
            self.entries.insert(key, matched);
        } else {
            self.stats.record_skip();
        }
        matched
    }

    /// Drops every entry. Call when the DOM snapshot or the rule set changes.
    pub fn clear(&mut self) {
        self.entries.clear();
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

        let first = cache.get_or_insert(&selector, 7, || {
            computed += 1;
            true
        });
        let second = cache.get_or_insert(&selector, 7, || {
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

        cache.get_or_insert(&first_selector, 1, || {
            computed += 1;
            true
        });
        cache.get_or_insert(&first_selector, 2, || {
            computed += 1;
            false
        });
        cache.get_or_insert(&second_selector, 1, || {
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

        cache.get_or_insert(&first_selector, 1, || true);
        cache.get_or_insert(&second_selector, 1, || true);

        assert_eq!(cache.len(), 1);
        let stats = cache.stats();
        assert_eq!(stats.evictions, 0);
        assert_eq!(stats.skips, 1);
        assert_eq!(stats.hits, 0);

        // Existing entries remain cached.
        let mut computed = 0;
        let matched = cache.get_or_insert(&first_selector, 1, || {
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
            cache.get_or_insert(sel, 1, || {
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
            cache.get_or_insert(sel, 1, || {
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
        cache.get_or_insert(&selector, 1, || true);
        cache.clear();

        assert!(cache.is_empty());
        let stats = cache.stats();
        assert_eq!(stats.misses, 0);
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.evictions, 0);
        assert_eq!(stats.skips, 0);
    }
}
