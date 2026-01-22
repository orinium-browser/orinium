use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use url::Url;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
}

#[derive(Debug, Clone)]
pub struct CookieStore {
    store: Arc<RwLock<HashMap<String, Vec<Cookie>>>>, // domain -> cookies
}

impl Default for CookieStore {
    fn default() -> Self {
        Self::new()
    }
}

impl CookieStore {
    pub fn new() -> Self {
        Self {
            store: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn set_cookies(&self, url: &Url, cookie_headers: &[String]) {
        let mut store = self.store.write().expect("RwLock poisoned");
        let domain = url.host_str().unwrap_or_default().to_string();
        let entry = store.entry(domain.clone()).or_default();

        for hdr in cookie_headers {
            if let Some((name, value)) = hdr.split_once('=') {
                entry.push(Cookie {
                    name: name.trim().to_string(),
                    value: value.split(';').next().unwrap_or("").trim().to_string(),
                    domain: domain.clone(),
                    path: "/".to_string(),
                    secure: url.scheme() == "https",
                });
            }
        }
    }

    pub fn get_cookie_header(&self, url: &Url) -> Option<String> {
        let store = self.store.read().ok()?;
        let domain = url.host_str().unwrap_or_default();

        store.get(domain).and_then(|cookies| {
            let s = cookies
                .iter()
                .map(|c| format!("{}={}", c.name, c.value))
                .collect::<Vec<_>>()
                .join("; ");

            if s.is_empty() { None } else { Some(s) }
        })
    }
}
