//use anyhow::Result;
use http_body_util::Empty;
use hyper::body::Bytes;
use hyper::client::conn::http1::SendRequest;
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::sync::Arc;
//use tokio::net::TcpStream;
use tokio::sync::RwLock;

#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct HostKey {
    pub scheme: hyper::http::uri::Scheme,
    pub host: String,
    pub port: u16,
}

pub enum Connection {
    http(SendRequest<Empty<Bytes>>),
}

pub struct ConnectionPool {
    pool: Arc<RwLock<HashMap<HostKey, Vec<Connection>>>>,
    pub max_connections_per_host: usize,
}

impl ConnectionPool {
    pub fn new() -> Self {
        Self {
            pool: Arc::new(RwLock::new(HashMap::new())),
            max_connections_per_host: 6,
        }
    }

    pub async fn get_connection(&self, key: &HostKey) -> Option<Connection> {
        let mut pool = self.pool.write().await;
        pool.get_mut(key).and_then(|vec| vec.pop())
    }

    pub async fn add_connection(&self, key: HostKey, conn: Connection) {
        let mut pool = self.pool.write().await;
        let entry = pool.entry(key).or_insert_with(Vec::new);
        if entry.len() < self.max_connections_per_host {
            entry.push(conn);
        }
    }

    #[allow(dead_code)]
    pub async fn close_all(&self) {
        let mut pool = self.pool.write().await;
        pool.clear();
    }
}
