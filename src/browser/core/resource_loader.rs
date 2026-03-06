//! Browser resource loading process, supports HTTP and resource:/// schemes.

use crate::platform::network::{NetworkCore, NetworkError};
use anyhow::{Result, anyhow};
use hyper::StatusCode;
use std::{fmt, rc::Rc};
use url::Url;

/// BrowserResourceLoader
///
/// High-level resource loading abstraction used by the browser core to obtain
/// content for tabs and internal resources.
///
/// Responsibilities:
/// - Resolve and fetch resources from `resource:///` scheme (bundled/local) and
///   from standard HTTP/HTTPS URLs.
/// - Provide a small synchronous/queuing abstraction over the platform network
///   core so callers in the engine/browser can request resources without dealing
///   with the network implementation details.
///
/// Processing flow (overview):
/// 1. Caller requests a URL (either `resource:///...` or `http(s)://...`).
/// 2. If the URL scheme is `resource`, loader resolves it to a local path or
///    embedded asset and returns the bytes immediately when available.
/// 3. For HTTP/HTTPS, loader forwards the request to `NetworkCore` and manages
///    request ids / pending responses. When the network reply is ready, the
///    loader hands the response back to the browser/tab via the expected
///    callback or message path.
///
/// Example usage:
/// ```no_run
/// use orinium_browser::browser::core::resource_loader::BrowserResourceLoader;
/// use std::rc::Rc;
/// use orinium_browser::platform::network::NetworkCore;
///
/// let network = Some(Rc::new(NetworkCore::new()));
/// let loader = BrowserResourceLoader::new(network);
///
/// // Typical call (pseudocode):
/// // let body = loader.fetch(&url)?;
/// // process body...
/// ```
///
/// Notes for contributors:
/// - Keep the loader focused on scheme resolution, simple caching/pooling,
///   and delegation to `NetworkCore`. Avoid adding heavy parsing logic here.
/// - Unit tests should validate `resource:///` resolution and HTTP request
///   delegation semantics (e.g. mapping of request IDs to responses).
pub struct BrowserResourceLoader {
    /// Optional platform network core used for HTTP/HTTPS requests.
    pub network: Option<Rc<NetworkCore>>,

    /// Immediate pool / internal queue for messages produced by the loader.
    /// The concrete type `BrowserNetworkMessage` represents internal network
    /// events; see the network module for details.
    pub immediate_pool: Vec<BrowserNetworkMessage>,
}

// NOTE: The actual fetch and handling methods are implemented below in this
// file. When adding methods, prefer small, testable units:
// - `resolve_resource_url(&self, url: &Url) -> ResourceLocation`
// - `fetch_http(&self, url: Url) -> Result<Vec<u8>>`
// - `fetch_resource_scheme(&self, url: Url) -> Result<Vec<u8>>`
//
// Keep the public API ergonomic for the engine (sync or async facade as
// appropriate for how NetworkCore exposes requests).

impl BrowserResourceLoader {
    /// Construct a new resource loader.
    ///
    /// `network` is optional to allow operating in environments where the
    /// network stack is not available (tests, limited examples, or when only
    /// `resource:///` is needed).
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
