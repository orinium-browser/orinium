//! Browser resource loading process.
//!
//! Supports the `http(s)://` scheme (via the platform `NetworkCore`), and the
//! network-free `resource:///`, `data:` and `file://` schemes.

use crate::engine::origin::Origin;
use crate::engine::port::{InProcess, Mailbox, Outbox, Transport};
use crate::platform::network::{
    NetworkConfig, NetworkCore, NetworkError, NetworkRequest, StatusCode,
};
use anyhow::{Context, Result, anyhow};
use base64::Engine;
use std::fmt;
use url::Url;

/// Whether a document with `initiator` may load a resource addressed by `url`.
///
/// Web (network) origins and unprivileged opaque origins (like `data:`) may only
/// reach external schemes and `data:`; internal schemes (`resource:`, `file:`)
/// require an internal origin.
///
/// TODO(security): Review `file:` access; local files could read the entire home directory.
fn scheme_allowed(initiator: &Origin, url: &Url) -> bool {
    match url.scheme() {
        "http" | "https" | "data" => true,
        "resource" | "file" => initiator.is_internal(),
        _ => false,
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
/// 2. Network-free schemes (`resource`, `data`) are resolved locally and posted
///    to the internal channel as `BrowserNetworkMessage`s.
/// 3. For HTTP/HTTPS, loader forwards the request to `NetworkCore` and manages
///    request ids / pending responses. When the network reply is ready, the
///    loader hands the response back to the browser/tab via the expected
///    callback or message path.
///
/// Example usage:
/// ```no_run
/// use orinium_browser::browser::core::resource_loader::BrowserResourceLoader;
/// use orinium_browser::platform::network::NetworkCore;
///
/// let loader = BrowserResourceLoader::new(NetworkCore::new().ok());
/// ```
///
/// The loader is `Send`: it owns its [`NetworkCore`] outright rather than
/// sharing it through a single-threaded handle. `BrowserApp` is not `Send`
/// yet, because it still holds per-window `BrowserUi` state; the loader is
/// free of that so it can move once the page boundary does.
///
/// Notes for contributors:
/// - Keep the loader focused on scheme resolution, simple caching/pooling,
///   and delegation to `NetworkCore`. Avoid adding heavy parsing logic here.
/// - Do not reintroduce `Rc` (or any other `!Send` handle) around
///   `NetworkCore`; it would make this type impossible to move.
/// - Unit tests should validate `resource:///` and `data:` resolution and HTTP
///   request delegation semantics (e.g. mapping of request IDs to responses).
pub struct BrowserResourceLoader {
    /// Optional platform network core used for HTTP/HTTPS requests.
    ///
    /// Owned directly: `NetworkCore` is already a handle to a separate
    /// network process and every operation takes `&self`, so there is nothing
    /// to share.
    network: Option<NetworkCore>,

    /// Where results of the network-free schemes are posted. A channel rather
    /// than a `Vec` so a transport that carries these elsewhere keeps the same
    /// call sites.
    immediate_outbox: Outbox<BrowserNetworkMessage>,

    /// The reading half of the same channel.
    immediate_inbox: Mailbox<BrowserNetworkMessage>,
}

impl BrowserResourceLoader {
    /// Construct a new resource loader.
    ///
    /// `network` is optional to allow operating in environments where the
    /// network stack is not available (tests, limited examples, or when only
    /// `resource:///` is needed).
    pub fn new(network: Option<NetworkCore>) -> Self {
        Self::with_transport(network, &InProcess)
    }

    /// Constructs a loader whose internal channel is carried by `transport`.
    ///
    /// The default [`InProcess`] hands messages over directly. Generic rather
    /// than `&dyn Transport` because pairing the ends is generic over the
    /// message type, which costs object safety — and the transport is only
    /// needed here, so it does not have to be stored either.
    pub fn with_transport<T: Transport>(network: Option<NetworkCore>, transport: &T) -> Self {
        let (immediate_outbox, immediate_inbox) = transport.channel();
        Self {
            network,
            immediate_outbox,
            immediate_inbox,
        }
    }

    /// Constructs a loader backed by `network`.
    pub fn with_network(network: NetworkCore) -> Self {
        Self::new(Some(network))
    }

    /// Constructs a loader that performs no network I/O.
    ///
    /// `resource:///`, `data:` and `file://` still resolve; anything else is
    /// dropped.
    pub fn offline() -> Self {
        Self::new(None)
    }

    /// Applies a new configuration to the network process, if present.
    pub fn set_network_config(&self, config: NetworkConfig) {
        if let Some(net) = &self.network {
            net.set_network_config(config);
        }
    }

    /// Drops every cached response held by the network process, if present.
    pub fn clear_cache(&self) {
        if let Some(net) = &self.network {
            net.clear_cache();
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
            self.post_immediate(BrowserNetworkMessage {
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
            self.post_immediate(BrowserNetworkMessage {
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
        self.post_immediate(msg);
    }

    /// Queues a message for the next `try_receive`.
    ///
    /// A send can only fail if the reading half is gone, and this type owns
    /// both halves, so the `expect` documents that invariant.
    fn post_immediate(&self, message: BrowserNetworkMessage) {
        self.immediate_outbox
            .send(message)
            .expect("the loader owns the reading end of its own channel");
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
        msgs.extend(std::iter::from_fn(|| self.immediate_inbox.try_recv()));

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
            let decoded = percent_decode(payload);
            let cleaned: Vec<u8> = decoded
                .into_iter()
                .filter(|b| !b.is_ascii_whitespace())
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
    use crate::engine::port::{Consume, Disconnected, Produce, Undeliverable};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The loader owns its `NetworkCore`, so it must be movable across
    /// threads. This is the invariant that a `!Send` handle around
    /// `NetworkCore` would silently break, so it is asserted directly.
    #[test]
    fn loader_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<BrowserResourceLoader>();
        assert_send::<BrowserNetworkMessage>();
        assert_send::<BrowserResponse>();
        assert_send::<BrowserNetworkError>();
    }

    /// A loader can be built and dropped without ever opening a device or a
    /// socket, which is what makes the offline mode usable in tests.
    #[test]
    fn offline_loader_needs_no_network_stack() {
        let loader = BrowserResourceLoader::offline();
        assert!(loader.network.is_none());
    }

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
        let loader = BrowserResourceLoader::offline();
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"png-bytes");
        let url = Url::parse(&format!("data:image/png;base64,{encoded}")).unwrap();

        let resp = loader.fetch_blocking(url).unwrap();
        assert!(resp.status.is_success());
        assert_eq!(resp.body, b"png-bytes");
    }

    #[test]
    fn fetch_async_pushes_data_url_into_immediate_pool() {
        let mut loader = BrowserResourceLoader::offline();
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
        let mut loader = BrowserResourceLoader::offline();
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
        let mut loader = BrowserResourceLoader::offline();
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
        let mut loader = BrowserResourceLoader::offline();
        let internal = Origin::internal();

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
        let mut loader = BrowserResourceLoader::offline();
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
        let loader = BrowserResourceLoader::offline();
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
        let mut loader = BrowserResourceLoader::offline();

        loader.fetch_async(url, 12, &Origin::internal());
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
        let mut loader = BrowserResourceLoader::offline();

        loader.fetch_async(url, 13, &Origin::internal());
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
        let mut loader = BrowserResourceLoader::offline();

        loader.fetch_async(url, 14, &web);
        let msgs = loader.try_receive();
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].response.is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn data_origin_cannot_reach_resource_scheme() {
        let mut loader = BrowserResourceLoader::offline();
        let data_origin = Origin::from_url(&Url::parse("data:text/html,evil").unwrap());

        loader.fetch_async(
            Url::parse("resource:///devtools/index.html").unwrap(),
            20,
            &data_origin,
        );

        let messages = loader.try_receive();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, 20);
        assert!(messages[0].response.is_err());
    }

    #[test]
    fn data_origin_cannot_reach_file_scheme() {
        let path = temp_file(b"secret");
        let url = Url::from_file_path(&path).unwrap();
        let data_origin = Origin::from_url(&Url::parse("data:text/html,evil").unwrap());
        let mut loader = BrowserResourceLoader::offline();

        loader.fetch_async(url, 21, &data_origin);
        let msgs = loader.try_receive();
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].response.is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn opaque_origin_cannot_reach_resource_or_file() {
        let opaque = Origin::opaque();
        let mut loader = BrowserResourceLoader::offline();

        loader.fetch_async(
            Url::parse("resource:///devtools/index.html").unwrap(),
            22,
            &opaque,
        );
        let messages = loader.try_receive();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].response.is_err());

        let path = temp_file(b"secret");
        let file_url = Url::from_file_path(&path).unwrap();
        loader.fetch_async(file_url, 23, &opaque);
        let messages = loader.try_receive();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].response.is_err());
        let _ = std::fs::remove_file(&path);
    }

    /// A loader must behave the same over a transport that is not the default.
    ///
    /// [`Buffered`] is a different implementation, not the in-process channel
    /// wearing another name, so this fails the moment the loader reaches past
    /// the port and depends on `InProcess` specifically.
    #[test]
    fn results_arrive_over_a_transport_other_than_the_default() {
        let mut loader = BrowserResourceLoader::with_transport(None, &Buffered);
        let url = Url::parse("data:text/plain,relayed").unwrap();

        loader.fetch_async(url, 15, &Origin::opaque());
        let msgs = loader.try_receive();

        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].id, 15);
        assert_eq!(msgs[0].response.as_ref().unwrap().body, b"relayed");
    }

    #[test]
    fn draining_an_idle_loader_reports_nothing_without_blocking() {
        let mut loader = BrowserResourceLoader::with_transport(None, &Buffered);
        assert!(loader.try_receive().is_empty());
    }

    /// The loader's channel must not stop the loader being movable, which is
    /// the invariant the page boundary depends on. `loader_is_send` above
    /// asserts the bound; this moves one to check the claim.
    #[test]
    fn loader_can_move_between_threads() {
        let loader = BrowserResourceLoader::offline();
        let url = Url::parse("data:text/plain,moved").unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let mut loader = loader;
            loader.fetch_async(url, 16, &Origin::opaque());
            for message in loader.try_receive() {
                sender.send(message.id).expect("receiver is alive");
            }
        })
        .join()
        .expect("the loader thread finished");

        assert_eq!(receiver.recv().expect("a message was posted"), 16);
    }

    /// A second [`Transport`] implementation, so the tests above cannot pass by
    /// relying on [`InProcess`] specifically.
    #[derive(Debug, Default, Clone, Copy)]
    struct Buffered;

    impl Transport for Buffered {
        fn channel<T: Send + 'static>(&self) -> (Outbox<T>, Mailbox<T>) {
            struct Queue<T> {
                messages: std::sync::Mutex<std::collections::VecDeque<T>>,
                arrived: std::sync::Condvar,
            }

            impl<T> Queue<T> {
                fn push(&self, message: T) {
                    self.messages
                        .lock()
                        .expect("lock poisoned")
                        .push_back(message);
                    self.arrived.notify_all();
                }

                fn pop(&self) -> Option<T> {
                    self.messages.lock().expect("lock poisoned").pop_front()
                }

                fn pop_waiting(&self) -> Result<T, Disconnected> {
                    let mut messages = self.messages.lock().expect("lock poisoned");
                    loop {
                        if let Some(message) = messages.pop_front() {
                            return Ok(message);
                        }
                        messages = self.arrived.wait(messages).expect("lock poisoned");
                    }
                }
            }

            struct Enqueuer<T>(std::sync::Arc<Queue<T>>);
            struct Dequeuer<T>(std::sync::Arc<Queue<T>>);

            impl<T: Send + 'static> Produce<T> for Enqueuer<T> {
                fn send(&self, message: T) -> Result<(), Undeliverable<T>> {
                    self.0.push(message);
                    Ok(())
                }
            }

            impl<T: Send + 'static> Consume<T> for Dequeuer<T> {
                fn recv(&self) -> Result<T, Disconnected> {
                    self.0.pop_waiting()
                }

                fn try_recv(&self) -> Option<T> {
                    self.0.pop()
                }
            }

            let queue = std::sync::Arc::new(Queue {
                messages: std::sync::Mutex::new(std::collections::VecDeque::new()),
                arrived: std::sync::Condvar::new(),
            });
            (
                Outbox::of(Enqueuer(std::sync::Arc::clone(&queue))),
                Mailbox::of(Dequeuer(queue)),
            )
        }
    }
}
