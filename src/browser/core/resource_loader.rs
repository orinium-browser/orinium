use crate::network::{NetworkCore, NetworkError};
use anyhow::{Result, anyhow};
use hyper::StatusCode;
use std::{fmt, rc::Rc};
use url::Url;

/// Unified resource loader for `resource:///` and HTTP/HTTPS URLs
pub struct BrowserResourceLoader {
    network: Option<Rc<NetworkCore>>,
    immediate_pool: Vec<BrowserNetworkMessage>,
}

impl BrowserResourceLoader {
    pub fn new(network: Option<Rc<NetworkCore>>) -> Self {
        Self {
            network,
            immediate_pool: vec![],
        }
    }

    /// 非同期 fetch: URL と ID を送信するだけ
    pub fn fetch_async(&mut self, url: Url, id: usize) {
        if url.scheme() == ("resource") {
            let data = ResourceURI::load(url.as_ref());
            let msg = BrowserNetworkMessage {
                id,
                response: data
                    .map(|data| BrowserResponse {
                        url: url.to_string(),
                        status: StatusCode::OK,
                        body: data,
                        headers: vec![],
                    })
                    .map_err(BrowserNetworkError::AnyhowError),
            };
            self.immediate_pool.push(msg);
        } else if let Some(net) = &self.network {
            net.fetch_async(url.to_string(), id);
        }
    }

    pub fn fetch_blocking(&self, url: Url) -> Result<BrowserResponse> {
        if url.scheme() == ("resource") {
            let data = ResourceURI::load(url.as_ref());
            data.map(|data| BrowserResponse {
                url: url.to_string(),
                status: StatusCode::OK,
                body: data,
                headers: vec![],
            })
        } else if let Some(net) = &self.network {
            net.fetch_blocking(url.as_str())
                .map(|resp| BrowserResponse {
                    url: resp.url,
                    status: resp.status,
                    body: resp.body,
                    headers: resp.headers,
                })
                .map_err(|e| anyhow!("NetworkError: {}", e))
        } else {
            Err(anyhow!("NetworkCore not available"))
        }
    }

    /// UIスレッドから呼ぶ: 受信済みネットワーク結果を取り込む
    pub fn try_receive(&mut self) -> Vec<BrowserNetworkMessage> {
        let mut msgs = if let Some(net) = &self.network {
            net.try_receive()
                .into_iter()
                .map(|msg| BrowserNetworkMessage {
                    id: msg.msg_id,
                    response: msg
                        .response
                        .map(|resp| BrowserResponse {
                            url: resp.url,
                            status: resp.status,
                            body: resp.body,
                            headers: resp.headers,
                        })
                        .map_err(BrowserNetworkError::NetworkError),
                })
                .collect()
        } else {
            Vec::new()
        };
        msgs.extend(std::mem::take(&mut self.immediate_pool));

        msgs
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
    pub id: usize,
    pub response: Result<BrowserResponse, BrowserNetworkError>,
}

#[derive(Debug)]
pub enum BrowserNetworkError {
    NetworkError(NetworkError),
    AnyhowError(anyhow::Error),
}

impl fmt::Display for BrowserNetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NetworkError(ne) => write!(f, "{ne}"),
            Self::AnyhowError(ae) => write!(f, "{ae}"),
        }
    }
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
