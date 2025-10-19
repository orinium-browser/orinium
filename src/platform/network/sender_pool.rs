use http_body_util::Empty;
use hyper::body::Bytes;
use hyper::client::conn::{http1, http2};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct HostKey {
    pub scheme: hyper::http::uri::Scheme,
    pub host: String,
    pub port: u16,
}

/// HTTP/1 と HTTP/2 の Sender を統一的に扱う型
pub enum HttpSender {
    Http1(http1::SendRequest<Empty<Bytes>>),
    Http2(http2::SendRequest<Empty<Bytes>>),
}

pub struct SenderPool {
    pool: Arc<RwLock<HashMap<HostKey, Vec<HttpSender>>>>,
    pub max_connections_per_host: usize,
}

impl Default for SenderPool {
    fn default() -> Self {
        Self::new()
    }
}

impl SenderPool {
    pub fn new() -> Self {
        Self {
            pool: Arc::new(RwLock::new(HashMap::new())),
            max_connections_per_host: 6,
        }
    }

    pub async fn get_connection(&self, key: &HostKey) -> Option<HttpSender> {
        let mut pool = self.pool.write().await;
        pool.get_mut(key).and_then(|vec| vec.pop())
    }

    pub async fn add_connection(&self, key: HostKey, conn: HttpSender) {
        let mut pool = self.pool.write().await;
        let entry = pool.entry(key).or_insert_with(Vec::new);
        if entry.len() < self.max_connections_per_host {
            entry.push(conn);
        }
    }

    pub async fn remove_connection(&self, key: &HostKey) {
        let mut pool = self.pool.write().await;
        if let Some(conns) = pool.get_mut(key) {
            if !conns.is_empty() {
                conns.pop();
            }
            if conns.is_empty() {
                pool.remove(key);
            }
        }
    }

    pub async fn close_all(&self) {
        let mut pool = self.pool.write().await;
        pool.clear();
    }
}
