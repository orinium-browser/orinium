//! Hit/miss counters shared by the engine and platform caches.
//!
//! Every cache in the engine (CSS selector match results) and in the platform
//! layer (HTTP responses, glyph atlas, glyph bearings, text layouts, text
//! shaping) owns a [`CacheStats`] counter set so hit rates and eviction
//! pressure can be observed at runtime and measured in `benches/cache_bench.rs`.
//!
//! This module lives in the engine layer so engine-side caches can count hits
//! without reaching down into the platform; the platform re-exports it under
//! its original path.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic counters describing how a cache behaves.
#[derive(Debug)]
pub struct CacheStats {
    name: &'static str,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    skips: AtomicU64,
}

/// Immutable view over a [`CacheStats`] counter set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStatsSnapshot {
    pub name: &'static str,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    /// Insertions rejected because a cache with admission limits was full.
    pub skips: u64,
}

impl CacheStatsSnapshot {
    /// Number of lookups that hit or missed.
    pub fn accesses(&self) -> u64 {
        self.hits + self.misses
    }

    /// Fraction of lookups that hit; `0.0` when nothing was looked up yet.
    pub fn hit_rate(&self) -> f64 {
        let accesses = self.accesses();
        if accesses == 0 {
            0.0
        } else {
            self.hits as f64 / accesses as f64
        }
    }
}

impl fmt::Display for CacheStatsSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} hits / {} misses ({:.1}%), {} evictions",
            self.name,
            self.hits,
            self.misses,
            self.hit_rate() * 100.0,
            self.evictions
        )?;
        if self.skips > 0 {
            write!(f, ", {} skips", self.skips)?;
        }
        Ok(())
    }
}

impl CacheStats {
    /// Creates a counter set labelled with `name` for log output.
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
            skips: AtomicU64::new(0),
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_eviction(&self) {
        self.evictions.fetch_add(1, Ordering::Relaxed);
    }

    /// Records an insertion that was rejected because an admission-limited
    /// cache was already full.
    pub fn record_skip(&self) {
        self.skips.fetch_add(1, Ordering::Relaxed);
    }

    /// Resets every counter to zero, for caches whose statistics describe one
    /// invalidation epoch rather than the whole process lifetime.
    pub fn reset(&self) {
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.evictions.store(0, Ordering::Relaxed);
        self.skips.store(0, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> CacheStatsSnapshot {
        CacheStatsSnapshot {
            name: self.name,
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            skips: self.skips.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_accumulate_and_compose_a_hit_rate() {
        let stats = CacheStats::new("test");

        assert_eq!(stats.snapshot().accesses(), 0);
        assert_eq!(stats.snapshot().hit_rate(), 0.0);

        stats.record_hit();
        stats.record_hit();
        stats.record_miss();
        stats.record_eviction();
        stats.record_skip();

        let snapshot = stats.snapshot();
        assert_eq!(snapshot.name, "test");
        assert_eq!(snapshot.hits, 2);
        assert_eq!(snapshot.misses, 1);
        assert_eq!(snapshot.evictions, 1);
        assert_eq!(snapshot.skips, 1);
        assert_eq!(snapshot.accesses(), 3);
        assert!((snapshot.hit_rate() - 2.0 / 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn reset_zeroes_every_counter() {
        let stats = CacheStats::new("test");
        stats.record_hit();
        stats.record_miss();
        stats.record_eviction();
        stats.record_skip();

        stats.reset();

        let snapshot = stats.snapshot();
        assert_eq!(snapshot.accesses(), 0);
        assert_eq!(snapshot.evictions, 0);
        assert_eq!(snapshot.skips, 0);
    }

    #[test]
    fn display_includes_the_label_and_rates() {
        let stats = CacheStats::new("glyph_atlas");
        stats.record_hit();
        stats.record_miss();

        assert_eq!(
            stats.snapshot().to_string(),
            "glyph_atlas: 1 hits / 1 misses (50.0%), 0 evictions"
        );
    }

    #[test]
    fn display_includes_skips_only_when_present() {
        let stats = CacheStats::new("selector_match");
        stats.record_miss();
        stats.record_skip();

        assert_eq!(
            stats.snapshot().to_string(),
            "selector_match: 0 hits / 1 misses (0.0%), 0 evictions, 1 skips"
        );
    }
}
