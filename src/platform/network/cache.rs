//! In-memory HTTP response cache with TTL and LRU eviction.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use lru::LruCache;

use super::core::Response;

/// Maximum number of responses kept before the oldest ones are evicted.
const DEFAULT_MAX_ENTRIES: usize = 512;

#[derive(Debug)]
pub struct Cache {
    enabled: AtomicBool,
    store: Arc<Mutex<LruCache<String, CachedResponse>>>,
}

#[derive(Debug)]
struct CachedResponse {
    response: Response,
    expires_at: Option<SystemTime>,
}

impl Default for Cache {
    fn default() -> Self {
        Self::new()
    }
}

impl Cache {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_MAX_ENTRIES)
    }

    /// Creates a cache holding at most `max_entries` responses.
    pub fn with_capacity(max_entries: usize) -> Self {
        let capacity = NonZeroUsize::new(max_entries.max(1)).unwrap();
        Self {
            enabled: AtomicBool::new(true),
            store: Arc::new(Mutex::new(LruCache::new(capacity))),
        }
    }

    /// Enables or disables caching. When disabled, `get` always misses and
    /// `set` is a no-op.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Returns a cached response for `url` if present and not expired.
    pub fn get(&self, url: &str) -> Option<Response> {
        if !self.is_enabled() {
            return None;
        }
        let mut store = self.store.lock().ok()?;
        let entry = store.get(url)?;
        if let Some(exp) = entry.expires_at
            && SystemTime::now() > exp
        {
            store.pop(url);
            return None;
        }
        Some(entry.response.clone())
    }

    /// Caches a response unless its headers forbid it.
    pub fn set(&self, url: &str, response: &Response) {
        if !self.is_enabled() {
            return;
        }
        if forbids_caching(&response.headers) {
            return;
        }
        if let Ok(mut store) = self.store.lock() {
            store.put(
                url.to_string(),
                CachedResponse {
                    response: response.clone(),
                    expires_at: expiry_from_headers(&response.headers),
                },
            );
        }
    }

    /// Number of responses currently held.
    pub fn len(&self) -> usize {
        self.store.lock().map(|store| store.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&self) {
        if let Ok(mut store) = self.store.lock() {
            store.clear();
        }
    }
}

/// Returns whether `Cache-Control` forbids reusing the response.
fn forbids_caching(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("cache-control")
            && (value.contains("no-store") || value.contains("no-cache"))
    })
}

/// Derives the expiry time from `Cache-Control: max-age`.
///
/// Returns `None` when no freshness lifetime is given (the response stays in
/// the cache until LRU eviction).
fn expiry_from_headers(headers: &[(String, String)]) -> Option<SystemTime> {
    for (name, value) in headers {
        if !name.eq_ignore_ascii_case("cache-control") {
            continue;
        }
        if let Some(pos) = value.find("max-age=") {
            let digits = value[pos + 8..]
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .unwrap_or("0");
            if let Ok(max_age) = digits.parse::<u64>() {
                return Some(SystemTime::now() + Duration::from_secs(max_age));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::network::core::{Response, StatusCode};

    fn response(body: &[u8], headers: Vec<(String, String)>) -> Response {
        Response {
            url: "https://example.test/".to_string(),
            status: StatusCode::from(hyper::StatusCode::OK),
            reason_phrase: "OK".to_string(),
            headers,
            body: body.to_vec(),
        }
    }

    fn cache_control(value: &str) -> Vec<(String, String)> {
        vec![("cache-control".to_string(), value.to_string())]
    }

    #[test]
    fn cached_response_is_returned_until_expiry() {
        let cache = Cache::with_capacity(16);
        let url = "https://example.test/page";

        cache.set(url, &response(b"hello", cache_control("max-age=100")));

        let cached = cache.get(url).expect("response should be cached");
        assert_eq!(cached.body, b"hello");
    }

    #[test]
    fn expired_entry_is_treated_as_a_miss() {
        let cache = Cache::with_capacity(16);
        let url = "https://example.test/page";

        cache.set(url, &response(b"hello", cache_control("max-age=0")));

        assert!(
            cache.get(url).is_none(),
            "max-age=0 must expire immediately"
        );
    }

    #[test]
    fn no_store_responses_are_not_cached() {
        let cache = Cache::with_capacity(16);
        let url = "https://example.test/page";

        cache.set(url, &response(b"hello", cache_control("no-store")));
        cache.set(
            "https://example.test/no-cache",
            &response(b"hello", cache_control("no-cache")),
        );

        assert!(cache.get(url).is_none());
        assert!(cache.get("https://example.test/no-cache").is_none());
    }

    #[test]
    fn lru_evicts_the_oldest_entry() {
        let cache = Cache::with_capacity(2);

        cache.set("https://example.test/a", &response(b"a", Vec::new()));
        cache.set("https://example.test/b", &response(b"b", Vec::new()));
        cache.set("https://example.test/c", &response(b"c", Vec::new()));

        assert!(cache.get("https://example.test/a").is_none());
        assert!(cache.get("https://example.test/b").is_some());
        assert!(cache.get("https://example.test/c").is_some());
    }

    #[test]
    fn disabled_cache_ignores_sets_and_misses() {
        let cache = Cache::with_capacity(16);
        let url = "https://example.test/page";

        cache.set_enabled(false);
        cache.set(url, &response(b"hello", cache_control("max-age=100")));
        assert!(!cache.is_enabled());
        assert!(cache.get(url).is_none());

        cache.set_enabled(true);
        assert!(cache.is_enabled());
        assert!(
            cache.get(url).is_none(),
            "set while disabled must be ignored"
        );

        cache.set(url, &response(b"hello", cache_control("max-age=100")));
        assert_eq!(cache.get(url).unwrap().body, b"hello");
    }

    #[test]
    fn len_tracks_entries_and_clear_empties_it() {
        let cache = Cache::with_capacity(16);

        assert!(cache.is_empty());
        cache.set("https://example.test/a", &response(b"a", Vec::new()));
        cache.set("https://example.test/b", &response(b"b", Vec::new()));
        assert_eq!(cache.len(), 2);

        cache.clear();
        assert!(cache.is_empty());
        assert!(cache.get("https://example.test/a").is_none());
    }
}
