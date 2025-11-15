use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::RwLock;
use url::Url;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct CachedResponse {
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
    pub cached_at: SystemTime,
    pub expires_at: Option<SystemTime>,
}

#[derive(Debug, Clone)]
pub struct Cache {
    store: Arc<RwLock<HashMap<String, CachedResponse>>>,
}

#[allow(dead_code)]
impl Default for Cache {
    fn default() -> Self {
        Self::new()
    }
}

impl Cache {
    pub fn new() -> Self {
        Self {
            store: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get(&self, url: &Url) -> Option<CachedResponse> {
        let store = self.store.read().await;
        let key = url.as_str();
        if let Some(entry) = store.get(key) {
            if let Some(exp) = entry.expires_at
                && SystemTime::now() > exp
            {
                return None;
            }
            return Some(entry.clone());
        }
        None
    }

    pub async fn set(&self, url: &Url, body: Vec<u8>, headers: Vec<(String, String)>) {
        let mut store = self.store.write().await;
        let key = url.as_str().to_string();
        let mut expires = None;
        if let Some((_, cc)) = headers
            .iter()
            .find(|(n, _)| n.to_lowercase() == "cache-control")
            && let Some(pos) = cc.find("max-age=")
            && let Ok(max_age) = cc[pos + 8..]
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .unwrap_or("0")
                .parse::<u64>()
        {
            expires = Some(SystemTime::now() + Duration::from_secs(max_age));
        }
        store.insert(
            key,
            CachedResponse {
                body,
                headers,
                cached_at: SystemTime::now(),
                expires_at: expires,
            },
        );
    }

    pub async fn clear(&self) {
        let mut store = self.store.write().await;
        store.clear();
    }
}
