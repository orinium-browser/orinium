use http_body_util::Empty;
use hyper::{
    body::Bytes,
    client::conn::{http1, http2},
};
use std::collections::HashMap;

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
    pool: HashMap<HostKey, Vec<HttpSender>>,
    max_connections_per_host: usize,
}

impl Default for SenderPool {
    fn default() -> Self {
        Self::new()
    }
}

impl SenderPool {
    pub fn new() -> Self {
        Self {
            pool: HashMap::new(),
            max_connections_per_host: 6,
        }
    }

    pub fn get_connection(&mut self, key: &HostKey) -> Option<HttpSender> {
        self.pool.get_mut(key).and_then(|v| v.pop())
    }

    pub fn add_connection(&mut self, key: HostKey, conn: HttpSender) {
        let entry = self.pool.entry(key).or_insert_with(Vec::new);
        if entry.len() < self.max_connections_per_host {
            entry.push(conn);
        }
    }

    pub fn remove_connection(&mut self, key: &HostKey) {
        if let Some(conns) = self.pool.get_mut(key) {
            conns.pop();
            if conns.is_empty() {
                self.pool.remove(key);
            }
        }
    }

    pub fn clear(&mut self) {
        self.pool.clear();
    }
}
