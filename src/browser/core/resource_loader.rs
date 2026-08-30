//! Browser resource loading process.
//!
//! Supports the `http(s)://` scheme (via the platform `NetworkCore`), and the
//! network-free `resource:///`, `data:` and `file://` schemes.

use crate::engine::origin::Origin;
use crate::platform::network::{NetworkCore, NetworkError, NetworkRequest, StatusCode};
use anyhow::{Context, Result, anyhow};
use base64::Engine;
use std::{fmt, rc::Rc};
use url::Url;

/// Whether a document with `initiator` may load a resource addressed by `url`.
///
/// Web (network) origins may only reach external schemes and `data:`; every
/// other custom scheme (`resource:`, `file:`, `about:`, unknown schemes)
/// requires an opaque origin, i.e. a page that is itself internal.
fn scheme_allowed(initiator: &Origin, url: &Url) -> bool {
    match url.scheme() {
        "http" | "https" | "data" => true,
        _ => initiator.is_opaque(),
    }
}

/// BrowserResourceLoader
///
/// High-level resource loading abstraction used by the browser core to obtain
/// content for tabs and internal resources.
///
/// Responsibilities:
/// - Resolve and fetch resources from `resource:///` scheme (bundled/local) and
///   from standard HTTP/HTTPS URLs.
/// - Decode `data:` URLs (base64 or percent-encoded payloads) without touching
///   the network stack.
/// - Provide a small synchronous/queuing abstraction over the platform network
///   core so callers in the engine/browser can request resources without dealing
///   with the network implementation details.
///
/// Processing flow (overview):
/// 1. Caller requests a URL (`resource:///...`, `data:...` or `http(s)://...`).
/// 2. Network-free schemes (`resource`, `data`) are resolved locally and pushed
///    to `immediate_pool` as `BrowserNetworkMessage`s.
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
/// let network = Some(Rc::new(NetworkCore::new().unwrap()));
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
/// - Unit tests should validate `resource:///` and `data:` resolution and HTTP
///   request delegation semantics (e.g. mapping of request IDs to responses).
pub struct BrowserResourceLoader {
    /// Optional platform network core used for HTTP/HTTPS requests.
    pub network: Option<Rc<NetworkCore>>,

    /// Immediate pool / internal queue for messages produced by the loader.
    /// The concrete type `BrowserNetworkMessage` represents internal network
    /// events; see the network module for details.
    pub immediate_pool: Vec<BrowserNetworkMessage>,
}

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

    /// Async fetch: resolve immediate schemes (`resource` / `data`) in place and
    /// push the result to `immediate_pool`; delegate all other schemes to `NetworkCore`.
    ///
    /// `initiator` is the origin of the requesting document. Its scheme access
    /// is enforced here: web (network) origins can never reach internal
    /// `resource:`/custom scheme content.
    pub fn fetch_async(&mut self, url: Url, id: usize, initiator: &Origin) {
        self.fetch_request_async(NetworkRequest::get(url.to_string()), id, initiator);
    }

    /// Fetches a request while preserving method, headers, and body for HTTP(S).
    pub fn fetch_request_async(&mut self, request: NetworkRequest, id: usize, initiator: &Origin) {
        let Ok(url) = Url::parse(&request.url) else {
            self.immediate_pool.push(BrowserNetworkMessage {
                id,
                response: Err(BrowserNetworkError::AnyhowError(anyhow!(
                    "Invalid request URL: {}",
                    request.url
                ))),
            });
            return;
        };
        if !scheme_allowed(initiator, &url) {
            log::warn!(
                "Blocked {} from {} (internal scheme access denied)",
                url,
                initiator.ascii_serialization()
            );
            self.immediate_pool.push(BrowserNetworkMessage {
                id,
                response: Err(BrowserNetworkError::AnyhowError(anyhow!(
                    "Blocked request for {url}: the requesting page is not allowed to access this scheme"
                ))),
            });
            return;
        }
        let Some(body) = load_immediate(&url) else {
            if let Some(net) = &self.network {
                net.fetch_request_async(request, id);
            }
            return;
        };
        let msg = BrowserNetworkMessage {
            id,
            response: if request.method == "GET" && request.body.is_empty() {
                body.map(|body| make_response(&url, body))
                    .map_err(BrowserNetworkError::AnyhowError)
            } else {
                Err(BrowserNetworkError::AnyhowError(anyhow!(
                    "{} is not supported for {} URLs",
                    request.method,
                    url.scheme()
                )))
            },
        };
        self.immediate_pool.push(msg);
    }

    pub fn fetch_blocking(&self, url: Url) -> Result<BrowserResponse> {
        if let Some(body) = load_immediate(&url) {
            return body.map(|body| make_response(&url, body));
        }
        let Some(net) = &self.network else {
            return Err(anyhow!("NetworkCore not available"));
        };
        net.fetch_blocking(url.as_str())
            .map(|resp| BrowserResponse {
                url: resp.url,
                status: resp.status,
                status_text: resp.reason_phrase,
                body: resp.body,
                headers: resp.headers,
            })
            .map_err(|e| anyhow!("NetworkError: {}", e))
    }

    /// Called from the UI thread: collect received network and immediate-scheme results.
    pub fn try_receive(&mut self) -> Vec<BrowserNetworkMessage> {
        let mut msgs: Vec<BrowserNetworkMessage> = self
            .network
            .as_ref()
            .map(|net| {
                net.try_receive()
                    .into_iter()
                    .map(|msg| BrowserNetworkMessage {
                        id: msg.msg_id,
                        response: msg
                            .response
                            .map(|resp| BrowserResponse {
                                url: resp.url,
                                status: resp.status,
                                status_text: resp.reason_phrase,
                                body: resp.body,
                                headers: resp.headers,
                            })
                            .map_err(BrowserNetworkError::NetworkError),
                    })
                    .collect()
            })
            .unwrap_or_default();
        msgs.extend(std::mem::take(&mut self.immediate_pool));

        msgs
    }
}

/// Loads the body of schemes that are resolved without the network.
///
/// Returns `None` for schemes that must be delegated to `NetworkCore`.
fn load_immediate(url: &Url) -> Option<Result<Vec<u8>>> {
    match url.scheme() {
        "resource" => Some(ResourceURI::load(url.as_str())),
        "data" => Some(DataURI::decode(url.as_str())),
        "file" => Some(FileURI::load(url)),
        _ => None,
    }
}

/// Builds a 200 OK response from the body of an immediate scheme.
fn make_response(url: &Url, body: Vec<u8>) -> BrowserResponse {
    BrowserResponse {
        url: url.to_string(),
        status: hyper::StatusCode::OK.into(),
        status_text: "OK".to_string(),
        body,
        headers: vec![],
    }
}

/// 統一レスポンス
pub struct BrowserResponse {
    pub url: String,
    pub status: StatusCode,
    pub status_text: String,
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

/// `data:` URL decoder (RFC 2397)
///
/// Format: `data:[<mediatype>][;base64],<payload>`
/// - With the `;base64` flag: base64-decode the payload (ignoring whitespace).
/// - Otherwise: percent-decode the payload into bytes.
pub struct DataURI;

impl DataURI {
    pub fn decode(url: &str) -> Result<Vec<u8>> {
        let rest = url
            .strip_prefix("data:")
            .with_context(|| format!("Not a data: URL: {url}"))?;
        let (metadata, payload) = rest
            .split_once(',')
            .context("data: URL is missing the ',' delimiter")?;

        if metadata.to_ascii_lowercase().contains(";base64") {
            let cleaned: String = payload
                .chars()
                .filter(|c| !c.is_ascii_whitespace())
                .collect();
            base64::engine::general_purpose::STANDARD
                .decode(cleaned)
                .context("failed to decode base64 data: URL")
        } else {
            Ok(percent_decode(payload))
        }
    }
}

/// `file://` 用ローダー。
///
/// URL をローカルファイルシステムのパスに変換して読み込む。`file://host/...`
/// のように空でも `localhost` でもないホストを伴う URL は拒否し、ローカルの
/// ファイル URL 以外は解決しない。
pub struct FileURI;

impl FileURI {
    pub fn load(url: &Url) -> Result<Vec<u8>> {
        let path = url
            .to_file_path()
            .map_err(|()| anyhow!("Unsupported file URL (non-local host): {url}"))?;
        crate::platform::io::load_local_file(&path.to_string_lossy())
    }
}

/// Converts `%XX` sequences into their byte values. Invalid `%` sequences are kept as-is.
fn percent_decode(input: &str) -> Vec<u8> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2]))
        {
            out.push(hi * 16 + lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Creates a unique temporary file with `contents` and returns its path.
    /// The caller is responsible for removing the file.
    fn temp_file(contents: &[u8]) -> std::path::PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let name = format!(
            "orinium-file-uri-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn data_uri_decodes_base64_payload() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"hello");
        let url = format!("data:image/png;base64,{encoded}");
        assert_eq!(DataURI::decode(&url).unwrap(), b"hello");
    }

    #[test]
    fn data_uri_decodes_plain_payload() {
        let url = "data:text/plain,hello%20world";
        assert_eq!(DataURI::decode(url).unwrap(), b"hello world");
    }

    #[test]
    fn data_uri_ignores_whitespace_in_base64_payload() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"line1line2");
        let url = format!("data:text/plain;base64,{encoded}\n\r\t ");
        assert_eq!(DataURI::decode(&url).unwrap(), b"line1line2");
    }

    #[test]
    fn data_uri_rejects_missing_delimiter() {
        assert!(DataURI::decode("data:text/plain").is_err());
    }

    #[test]
    fn data_uri_rejects_invalid_base64() {
        assert!(DataURI::decode("data:text/plain;base64,%%%").is_err());
    }

    #[test]
    fn data_uri_flag_is_case_insensitive() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"ok");
        let url = format!("data:text/plain;BASE64,{encoded}");
        assert_eq!(DataURI::decode(&url).unwrap(), b"ok");
    }

    #[test]
    fn url_parse_preserves_data_url() {
        let url = Url::parse("data:image/png;base64,AAAA").unwrap();
        assert_eq!(url.scheme(), "data");
        assert_eq!(url.as_str(), "data:image/png;base64,AAAA");
    }

    #[test]
    fn fetch_blocking_decodes_data_url_without_network() {
        let loader = BrowserResourceLoader::new(None);
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"png-bytes");
        let url = Url::parse(&format!("data:image/png;base64,{encoded}")).unwrap();

        let resp = loader.fetch_blocking(url).unwrap();
        assert!(resp.status.is_success());
        assert_eq!(resp.body, b"png-bytes");
    }

    #[test]
    fn fetch_async_pushes_data_url_into_immediate_pool() {
        let mut loader = BrowserResourceLoader::new(None);
        let url = Url::parse("data:text/plain,hi").unwrap();

        loader.fetch_async(url, 7, &Origin::opaque());
        let msgs = loader.try_receive();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].id, 7);
        let resp = msgs[0].response.as_ref().unwrap();
        assert_eq!(resp.body, b"hi");
    }

    #[test]
    fn immediate_urls_reject_non_get_requests() {
        let mut loader = BrowserResourceLoader::new(None);
        loader.fetch_request_async(
            NetworkRequest {
                url: "data:text/plain,hi".to_string(),
                method: "POST".to_string(),
                headers: Vec::new(),
                body: b"request body".to_vec(),
            },
            8,
            &Origin::opaque(),
        );

        let messages = loader.try_receive();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, 8);
        assert!(messages[0].response.is_err());
    }

    #[test]
    fn network_origin_cannot_reach_resource_scheme() {
        let mut loader = BrowserResourceLoader::new(None);
        let web = Origin::from_url(&Url::parse("https://example.test/").unwrap());

        loader.fetch_async(
            Url::parse("resource:///devtools/index.html").unwrap(),
            9,
            &web,
        );

        let messages = loader.try_receive();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, 9);
        assert!(messages[0].response.is_err());
    }

    #[test]
    fn internal_origin_can_reach_resource_scheme() {
        let mut loader = BrowserResourceLoader::new(None);
        let internal = Origin::opaque();

        loader.fetch_async(
            Url::parse("resource:///devtools/index.html").unwrap(),
            10,
            &internal,
        );

        let messages = loader.try_receive();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, 10);
        assert!(messages[0].response.as_ref().is_ok());
    }

    #[test]
    fn any_origin_can_reach_data_scheme() {
        let mut loader = BrowserResourceLoader::new(None);
        let web = Origin::from_url(&Url::parse("https://example.test/").unwrap());

        loader.fetch_async(Url::parse("data:text/plain,hi").unwrap(), 11, &web);

        let messages = loader.try_receive();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, 11);
        let resp = messages[0].response.as_ref().unwrap();
        assert_eq!(resp.body, b"hi");
    }

    #[test]
    fn fetch_blocking_rejects_data_url_without_network() {
        let loader = BrowserResourceLoader::new(None);
        let url = Url::parse("data:image/png;base64,@@@not-base64@@@").unwrap();
        assert!(loader.fetch_blocking(url).is_err());
    }

    #[test]
    fn file_uri_loads_local_file_bytes() {
        let path = temp_file(b"file content");
        let url = Url::from_file_path(&path).unwrap();

        assert_eq!(FileURI::load(&url).unwrap(), b"file content");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_url_with_nonlocal_host_is_rejected() {
        let url = Url::parse("file://evil.example/etc/passwd").unwrap();
        assert!(FileURI::load(&url).is_err());
    }

    #[test]
    fn fetch_async_resolves_file_url_into_immediate_pool() {
        let path = temp_file(b"file bytes");
        let url = Url::from_file_path(&path).unwrap();
        let mut loader = BrowserResourceLoader::new(None);

        loader.fetch_async(url, 12, &Origin::opaque());
        let msgs = loader.try_receive();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].id, 12);
        let resp = msgs[0].response.as_ref().unwrap();
        assert_eq!(resp.body, b"file bytes");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn internal_origin_can_read_local_file() {
        let path = temp_file(b"local data");
        let url = Url::from_file_path(&path).unwrap();
        let mut loader = BrowserResourceLoader::new(None);

        loader.fetch_async(url, 13, &Origin::opaque());
        let msgs = loader.try_receive();
        assert_eq!(msgs.len(), 1);
        let resp = msgs[0].response.as_ref().unwrap();
        assert_eq!(resp.body, b"local data");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn network_origin_cannot_read_local_file() {
        let path = temp_file(b"secret");
        let url = Url::from_file_path(&path).unwrap();
        let web = Origin::from_url(&Url::parse("https://example.test/").unwrap());
        let mut loader = BrowserResourceLoader::new(None);

        loader.fetch_async(url, 14, &web);
        let msgs = loader.try_receive();
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].response.is_err());
        let _ = std::fs::remove_file(&path);
    }
}
