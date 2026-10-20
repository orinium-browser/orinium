//! Bounded cache for shaped text results keyed by text and style.

use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use lru::LruCache;
use smol_str::SmolStr;

use orinium_text::ShapedText;

use crate::platform::cache_stats::{CacheStats, CacheStatsSnapshot};

/// Maximum number of shaped texts kept before the least recently used entry is
/// evicted. The cache used to be an unbounded `HashMap`, which let long-lived
/// pages with dynamic content grow without limit.
const DEFAULT_CAPACITY: usize = 4096;

#[derive(Clone)]
pub struct TextShapeCache {
    inner: Arc<Mutex<LruCache<TextShapeCacheKey, ShapedText>>>,
    stats: Arc<CacheStats>,
}

impl Default for TextShapeCache {
    fn default() -> Self {
        Self::new()
    }
}

impl TextShapeCache {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// Creates a cache holding at most `capacity` shaped texts.
    pub fn with_capacity(capacity: usize) -> Self {
        let capacity = NonZeroUsize::new(capacity.max(1)).unwrap();
        Self {
            inner: Arc::new(Mutex::new(LruCache::new(capacity))),
            stats: Arc::new(CacheStats::new("text_shape")),
        }
    }

    pub fn get(&self, text: &str, style: &orinium_text::TextStyle) -> Option<ShapedText> {
        let key = TextShapeCacheKey::new(text, style);

        let Ok(mut cache) = self.inner.lock() else {
            self.stats.record_miss();
            return None;
        };
        match cache.get(&key) {
            Some(shaped) => {
                let shaped = shaped.clone();
                self.stats.record_hit();
                Some(shaped)
            }
            None => {
                self.stats.record_miss();
                None
            }
        }
    }

    pub fn insert(&self, text: &str, style: &orinium_text::TextStyle, shaped: ShapedText) {
        let key = TextShapeCacheKey::new(text, style);

        if let Ok(mut cache) = self.inner.lock() {
            // `push` returns the replaced entry when the key already exists
            // and the least recently used entry when the cache overflows.
            let unchanged_key = key.clone();
            if let Some((evicted_key, _)) = cache.push(key, shaped)
                && evicted_key != unchanged_key
            {
                self.stats.record_eviction();
            }
        }
    }

    /// Drops every entry; the counters keep accumulating.
    pub fn clear(&self) {
        if let Ok(mut cache) = self.inner.lock() {
            cache.clear();
        }
    }

    /// Number of entries currently held.
    pub fn len(&self) -> usize {
        self.inner.lock().map(|cache| cache.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Snapshot of this cache's hit/miss/eviction counters.
    pub fn stats(&self) -> CacheStatsSnapshot {
        self.stats.snapshot()
    }
}

#[derive(Clone, Eq)]
struct TextShapeCacheKey {
    text: SmolStr,
    style_hash: u64,
}

impl TextShapeCacheKey {
    fn new(text: &str, style: &orinium_text::TextStyle) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hash_style(style, &mut hasher);

        Self {
            text: SmolStr::new(text),
            style_hash: hasher.finish(),
        }
    }
}

impl PartialEq for TextShapeCacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.style_hash == other.style_hash
    }
}

impl std::hash::Hash for TextShapeCacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
        self.style_hash.hash(state);
    }
}

fn hash_style(style: &orinium_text::TextStyle, hasher: &mut impl Hasher) {
    style.font_size.to_bits().hash(hasher);
    style.line_height.to_bits().hash(hasher);

    style.color.0.hash(hasher);
    style.color.1.hash(hasher);
    style.color.2.hash(hasher);
    style.color.3.hash(hasher);

    style.font_weight.0.hash(hasher);
    style.font_style.hash(hasher);

    style.bidi_mode.hash(hasher);
    style.variant.hash(hasher);

    for family in &style.font_families {
        family.hash(hasher);
    }

    for font in &style.exact_fonts {
        font.hash(hasher);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orinium_text::{FontSystem, PlatformFallback, TextLayouter, TextStyle};

    /// Produces a real `ShapedText` without touching system fonts: the font
    /// system starts from an empty database, and shaping an empty database
    /// returns an empty (but valid) result.
    fn sample_shape() -> ShapedText {
        let db = orinium_text::fontdb::Database::new();
        let mut fs = FontSystem::new_with_locale_and_db_and_fallback(db, PlatformFallback);
        TextLayouter::new().shape_text(&mut fs, "sample", &TextStyle::default())
    }

    #[test]
    fn stores_and_returns_shaped_text() {
        let cache = TextShapeCache::new();

        assert!(cache.is_empty());
        assert!(cache.get("hello", &TextStyle::default()).is_none());

        cache.insert("hello", &TextStyle::default(), sample_shape());
        assert!(
            cache.get("hello", &TextStyle::default()).is_some(),
            "entry should hit"
        );

        let stats = cache.stats();
        assert_eq!(stats.name, "text_shape");
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.evictions, 0);
    }

    #[test]
    fn different_styles_are_cached_separately() {
        let cache = TextShapeCache::new();
        let other = TextStyle {
            font_size: 20.0,
            ..TextStyle::default()
        };

        cache.insert("hello", &TextStyle::default(), sample_shape());

        assert!(cache.get("hello", &other).is_none());
        assert!(cache.get("hello", &TextStyle::default()).is_some());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn evicts_the_least_recently_used_entry() {
        let cache = TextShapeCache::with_capacity(2);

        cache.insert("a", &TextStyle::default(), sample_shape());
        cache.insert("b", &TextStyle::default(), sample_shape());
        assert!(cache.get("a", &TextStyle::default()).is_some());
        // "b" is now the least recently used entry.
        cache.insert("c", &TextStyle::default(), sample_shape());

        assert!(cache.get("b", &TextStyle::default()).is_none());
        assert!(cache.get("a", &TextStyle::default()).is_some());
        assert!(cache.get("c", &TextStyle::default()).is_some());

        let stats = cache.stats();
        assert_eq!(stats.evictions, 1);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn clear_empties_the_cache_and_keeps_counters() {
        let cache = TextShapeCache::new();

        cache.insert("a", &TextStyle::default(), sample_shape());
        cache.clear();

        assert!(cache.is_empty());
        assert_eq!(cache.stats().evictions, 0);
    }

    #[test]
    fn stays_bounded_under_unbounded_insertion() {
        let cache = TextShapeCache::with_capacity(64);

        for i in 0..10_000 {
            cache.insert(
                &format!("dynamic content {i}"),
                &TextStyle::default(),
                sample_shape(),
            );
        }

        assert_eq!(cache.len(), 64, "the cache must stay within its capacity");
        assert_eq!(cache.stats().evictions, 10_000 - 64);
    }
}
