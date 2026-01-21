use crate::network::{NetworkCore, NetworkError};
use anyhow::{Result, anyhow};
use hyper::StatusCode;
use std::sync::Arc;

/// Unified resource loader for `resource:///` and HTTP/HTTPS URLs
pub struct BrowserResourceLoader {
    network: Option<Arc<NetworkCore>>,
}

impl BrowserResourceLoader {
    pub fn new(network: Option<Arc<NetworkCore>>) -> Self {
        Self { network }
    }

    /// 非同期 fetch: URL と Tab ID を送信するだけ
    pub fn fetch_async(&self, url: String, tab_id: usize) {
        if let Some(net) = &self.network {
            net.fetch_async(url, tab_id);
        }
    }

    /// UIスレッドから呼ぶ: 受信済みネットワーク結果を取り込む
    pub fn try_receive(&self) -> Vec<BrowserNetworkMessage> {
        if let Some(net) = &self.network {
            net.try_receive()
                .into_iter()
                .map(|msg| BrowserNetworkMessage {
                    tab_id: msg.tab_id,
                    response: msg.response.map(|resp| BrowserResponse {
                        url: resp.url,
                        status: resp.status,
                        body: resp.body,
                        headers: resp.headers,
                    }),
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    /// resource:/// のみ同期ロード
    pub fn load_resource_sync(&self, url: &str) -> Result<BrowserResponse> {
        if url.starts_with("resource:///") {
            let data = ResourceURI::load(url)?;
            Ok(BrowserResponse {
                url: url.to_string(),
                status: StatusCode::OK,
                body: data,
                headers: vec![],
            })
        } else {
            Err(anyhow!("Cannot synchronously fetch network URL: {}", url))
        }
    }
}

/// 統一レスポンス
pub struct BrowserResponse {
    pub url: String,
    pub status: StatusCode,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

/// ネットワーク結果を UI スレッドで受け取るためのラッパー
pub struct BrowserNetworkMessage {
    pub tab_id: usize,
    pub response: Result<BrowserResponse, NetworkError>,
}

/// resource:/// 専用
pub struct ResourceURI;

impl ResourceURI {
    pub fn load(url: &str) -> Result<Vec<u8>, anyhow::Error> {
        use crate::platform::io;
        if let Some(path) = url.strip_prefix("resource:///") {
            io::load_resource(path)
        } else {
            Err(anyhow!("Unsupported scheme: {}", url))
        }
    }
}
