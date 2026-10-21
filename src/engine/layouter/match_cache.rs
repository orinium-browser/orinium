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
//!
//! [`RuleSet`]: super::css_resolver::RuleSet

use std::num::NonZeroUsize;
use std::sync::Arc;

use lru::LruCache;

use super::dom_snapshot::NodeId;
use crate::engine::cache_stats::{CacheStats, CacheStatsSnapshot};
use crate::engine::css::parser::ComplexSelector;

/// Default number of memoized `(selector, element)` results.
///
/// Entries are a pointer, a node id and a boolean, so this bounds the cache at
/// a few megabytes even for very large documents.
pub const DEFAULT_CAPACITY: usize = 65_536;

/// `(selector allocation, element)` identity of one cached match result.
type MatchKey = (usize, NodeId);

/// Memoizes selector match results for one DOM/rule-set epoch.
pub struct SelectorMatchCache {
    entries: LruCache<MatchKey, bool>,
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
        let capacity = NonZeroUsize::new(capacity.max(1)).expect("capacity is at least one");
        Self {
            entries: LruCache::new(capacity),
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
        // `push` reports the evicted entry (unlike `put`), which lets us count
        // capacity pressure without double counting an existing key.
        let evicted = self
            .entries
            .push(key, matched)
            .is_some_and(|(evicted_key, _)| evicted_key != key);
        if evicted {
            self.stats.record_eviction();
        }
        matched
    }

    /// Drops every entry. Call when the DOM snapshot or the rule set changes.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
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
        assert_eq!((stats.hits, stats.misses), (1, 1));
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
        assert_eq!((stats.hits, stats.misses), (0, 3));
    }

    #[test]
    fn capacity_pressure_is_counted_as_an_eviction() {
        let mut cache = SelectorMatchCache::with_capacity(1);
        let first_selector = selector();
        let second_selector = selector();

        cache.get_or_insert(&first_selector, 1, || true);
        cache.get_or_insert(&second_selector, 1, || true);

        assert_eq!(cache.len(), 1);
        let stats = cache.stats();
        assert_eq!(stats.evictions, 1);
    }

    #[test]
    fn clear_drops_entries_and_keeps_counters() {
        let mut cache = SelectorMatchCache::new();
        let selector = selector();
        cache.get_or_insert(&selector, 1, || true);
        cache.clear();

        assert!(cache.is_empty());
        let stats = cache.stats();
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.hits, 0);
    }
}
