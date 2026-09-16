//! ネットワークコア
//! HTTP通信とレスポンス処理を担当する。

use super::{HostKey, HttpSender, NetworkConfig, NetworkError, NetworkRequest, SenderPool};

use brotli_decompressor::Decompressor as BrotliDecompressor;
use flate2::read::{DeflateDecoder as RawDeflateDecoder, GzDecoder, ZlibDecoder};
use http_body_util::{BodyExt, Full};
use hyper::{
    Method, Request, Uri,
    body::{Bytes, Incoming},
    client::conn,
    http::uri::Scheme,
};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::{
    ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::CryptoProvider,
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use rustls_native_certs::load_native_certs;
use serde::{Deserialize, Serialize};
use std::io::Read as _;
use std::sync::{Arc, RwLock};
use tokio::{net::TcpStream, runtime::Runtime, task::LocalSet};
use tokio_rustls::TlsConnector;

/// Per-thread driver for the shared network state.
///
/// The tokio runtime and its [`LocalSet`] are `!Send`, so each pool worker
/// owns one of these; the expensive state they operate on lives in the
/// [`SharedNetState`] behind an `Arc` and is reused across workers.
///
/// The [`SenderPool`] deliberately lives *here*, not in the shared state: a
/// pooled connection's driver task is spawned onto the creating worker's
/// local set and is only polled while that worker runs a fetch. Sharing
/// senders across runtimes would let one worker check out a connection whose
/// driver is parked on another (idle) worker, leaving the request awaiting
/// frames nobody ever reads.
pub(super) struct AsyncNetworkCore {
    local: LocalSet,
    rt: Runtime,
    inner: Arc<SharedNetState>,
    sender_pool: Arc<std::sync::RwLock<SenderPool>>,
}

impl AsyncNetworkCore {
    pub fn new(inner: Arc<SharedNetState>) -> Self {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build tokio runtime");

        let local = LocalSet::new();

        Self {
            rt,
            local,
            inner,
            sender_pool: Arc::new(std::sync::RwLock::new(SenderPool::new())),
        }
    }

    /// Runs a fetch to completion on this worker's local set.
    pub fn fetch_request_blocking(
        &self,
        request: &NetworkRequest,
    ) -> Result<Response, NetworkError> {
        self.local
            .block_on(&self.rt, async { self.fetch_request(request).await })
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct StatusCode(u16);

impl From<hyper::StatusCode> for StatusCode {
    fn from(value: hyper::StatusCode) -> Self {
        Self(value.as_u16())
    }
}

impl StatusCode {
    pub fn as_u16(&self) -> u16 {
        self.0
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.0)
    }

    pub fn is_redirection(&self) -> bool {
        (300..400).contains(&self.0)
    }

    pub fn canonical_reason(&self) -> Option<&'static str> {
        let hyper_code: hyper::StatusCode = self.as_u16().try_into().ok()?;
        hyper_code.canonical_reason()
    }
}

/// HTTP response
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Response {
    pub url: String,
    pub status: StatusCode,
    pub reason_phrase: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// TLS, cache and config shared by every fetch worker.
///
/// All fields are thread-safe: the cache is internally synchronized, and the
/// config is swapped atomically through an `Arc` so workers observe updates
/// without blocking a fetch in progress. Connection pools are *not* shared:
/// each [`AsyncNetworkCore`] owns its pool because a connection is only valid
/// on the runtime that drives it.
pub(super) struct SharedNetState {
    tls_config: Arc<ClientConfig>,
    insecure_tls_config: Arc<ClientConfig>,
    network_config: RwLock<Arc<NetworkConfig>>,
    cache: super::Cache,
}

impl SharedNetState {
    pub fn new() -> Self {
        Self {
            tls_config: Arc::new(Self::build_tls_config(true)),
            insecure_tls_config: Arc::new(Self::build_tls_config(false)),
            network_config: RwLock::new(Arc::new(NetworkConfig::default())),
            cache: super::Cache::new(),
        }
    }

    pub fn set_network_config(&self, config: NetworkConfig) {
        self.cache.set_enabled(config.enable_cache);
        *self.network_config.write().unwrap() = Arc::new(config);
    }

    /// Removes all cached responses.
    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    /// Returns the TLS configuration for the current `verify_tls` setting.
    fn tls_config(&self) -> Arc<ClientConfig> {
        if self.network_config.read().unwrap().verify_tls {
            Arc::clone(&self.tls_config)
        } else {
            Arc::clone(&self.insecure_tls_config)
        }
    }

    fn build_tls_config(verify_certs: bool) -> ClientConfig {
        let mut roots = RootCertStore::empty();
        let result = load_native_certs();

        for cert in result.certs {
            let _ = roots.add(cert);
        }

        let mut config = if verify_certs {
            ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth()
        } else {
            let provider = CryptoProvider::get_default()
                .expect("rustls default crypto provider must be installed");
            ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(SkipServerVerification(
                    provider.clone(),
                )))
                .with_no_client_auth()
        };
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        config
    }
}

impl AsyncNetworkCore {
    pub async fn fetch_request(&self, request: &NetworkRequest) -> Result<Response, NetworkError> {
        let mut current: Uri = request.url.parse().map_err(|_| NetworkError::InvalidUri)?;
        let mut method = Method::from_bytes(request.method.as_bytes())
            .map_err(|_| NetworkError::HttpRequestFailed)?;
        let mut body = request.body.clone();
        let mut redirects = 0usize;

        loop {
            if method == Method::GET
                && let Some(cached) = self.inner.cache.get(&current.to_string())
            {
                log::info!("NetworkCache: hit for url={}", current);
                return Ok(cached);
            }

            let resp = self
                .send_request(&current, &method, &request.headers, &body)
                .await?;

            if self.inner.network_config.read().unwrap().follow_redirects
                && hyper::StatusCode::try_from(resp.status.0)
                    .map_err(|_| NetworkError::InvalidIpcStatusCode)?
                    .is_redirection()
            {
                if redirects >= 10 {
                    return Err(NetworkError::TooManyRedirects);
                }

                if let Some(loc) = resp
                    .headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("location"))
                    .map(|(_, v)| v)
                {
                    current = resolve_redirect(&current, loc)?;
                    if resp.status.as_u16() == 303
                        || ((resp.status.as_u16() == 301 || resp.status.as_u16() == 302)
                            && method == Method::POST)
                    {
                        method = Method::GET;
                        body.clear();
                    }
                    redirects += 1;
                    continue;
                }
            }

            if method == Method::GET && resp.status.is_success() {
                self.inner.cache.set(&current.to_string(), &resp);
            }

            return Ok(resp);
        }
    }

    async fn send_request(
        &self,
        uri: &Uri,
        method: &Method,
        headers: &[(String, String)],
        body: &[u8],
    ) -> Result<Response, NetworkError> {
        let host = uri.host().ok_or(NetworkError::MissingHost)?;
        let scheme = uri.scheme().unwrap_or(&Scheme::HTTP);
        let port = uri
            .port_u16()
            .unwrap_or(if scheme == &Scheme::HTTPS { 443 } else { 80 });

        let key = HostKey {
            scheme: scheme.clone(),
            host: host.to_string(),
            port,
        };

        let mut sender = self.get_or_create_sender(&key).await?;

        let user_agent = self.inner.network_config.read().unwrap().user_agent.clone();
        let is_h2 = matches!(sender, HttpSender::Http2(_));
        log::debug!(
            target: "network",
            "send via {} to {}",
            if is_h2 { "h2" } else { "http1" },
            uri
        );
        // HTTP/2 derives `:authority` from the request URI (a relative URI is
        // rejected), while HTTP/1.1 wants an origin-form target plus a `Host`
        // header, which hyper refuses to add for us.
        let request_uri = if is_h2 {
            let authority = uri
                .authority()
                .map(|a| a.as_str().to_string())
                .unwrap_or_else(|| format!("{}:{}", host, port));
            format!(
                "{}://{}{}",
                scheme,
                authority,
                uri.path_and_query().map_or("/", |p| p.as_str())
            )
        } else {
            uri.path_and_query().map_or("/", |p| p.as_str()).to_string()
        };
        let mut request = Request::builder()
            .method(method.clone())
            .uri(&request_uri)
            .header("User-Agent", user_agent);
        if !is_h2 {
            request = request.header("Host", host);
        }
        if !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("accept-language"))
        {
            request = request.header(
                "Accept-Language",
                crate::platform::locale::accept_language_header(),
            );
        }
        if !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("accept-encoding"))
        {
            request = request.header("Accept-Encoding", "gzip, deflate, br");
        }
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let req = request
            .body(Full::new(Bytes::copy_from_slice(body)))
            .map_err(|_| NetworkError::HttpRequestFailed)?;

        let mut res = match &mut sender {
            HttpSender::Http1(s) => s
                .send_request(req)
                .await
                .map_err(|_| NetworkError::HttpRequestFailed)?,
            HttpSender::Http2(s) => s
                .send_request(req)
                .await
                .map_err(|_| NetworkError::HttpRequestFailed)?,
        };

        let response = Self::collect_response(uri.to_string(), &mut res).await?;

        self.sender_pool
            .write()
            .unwrap()
            .add_connection(key, sender);

        Ok(response)
    }

    async fn collect_response(
        url: String,
        res: &mut hyper::Response<Incoming>,
    ) -> Result<Response, NetworkError> {
        let status = res.status();
        let reason_phrase = status.canonical_reason().unwrap_or("").to_string();

        let headers = res
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        let mut body = Vec::new();
        while let Some(frame) = res.frame().await {
            let frame = frame.map_err(|_| NetworkError::HttpResponseFailed)?;
            if let Some(chunk) = frame.data_ref() {
                body.extend_from_slice(chunk);
            }
        }

        let content_encoding = res
            .headers()
            .get("content-encoding")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.to_string());
        let body = Self::decode_content(content_encoding.as_deref(), body);

        Ok(Response {
            url,
            status: status.into(),
            reason_phrase,
            headers,
            body,
        })
    }

    /// Decodes a response body according to the `Content-Encoding` header value
    /// (e.g. `gzip`, `deflate`, `br`). Unknown or undecodable encodings leave
    /// the body untouched.
    fn decode_content(encoding: Option<&str>, mut body: Vec<u8>) -> Vec<u8> {
        let Some(encoding) = encoding else {
            return body;
        };
        for part in encoding.split(',').map(|s| s.trim().to_ascii_lowercase()) {
            body = match part.as_str() {
                "gzip" | "x-gzip" => Self::gunzip(&body).unwrap_or(body),
                "deflate" => Self::inflate(&body).unwrap_or(body),
                "br" => Self::brotli(&body).unwrap_or(body),
                _ => body,
            };
        }
        body
    }

    fn gunzip(body: &[u8]) -> Option<Vec<u8>> {
        let mut output = Vec::new();
        GzDecoder::new(body).read_to_end(&mut output).ok()?;
        Some(output)
    }

    /// HTTP `deflate` is zlib-wrapped in practice; fall back to raw DEFLATE if
    /// some servers omit the zlib header.
    fn inflate(body: &[u8]) -> Option<Vec<u8>> {
        let mut output = Vec::new();
        if ZlibDecoder::new(body).read_to_end(&mut output).is_ok() {
            return Some(output);
        }
        let mut output = Vec::new();
        RawDeflateDecoder::new(body).read_to_end(&mut output).ok()?;
        Some(output)
    }

    fn brotli(body: &[u8]) -> Option<Vec<u8>> {
        let mut output = Vec::new();
        BrotliDecompressor::new(body, 1 << 16)
            .read_to_end(&mut output)
            .ok()?;
        Some(output)
    }

    async fn get_or_create_sender(&self, key: &HostKey) -> Result<HttpSender, NetworkError> {
        if let Some(s) = self.sender_pool.write().unwrap().get_connection(key) {
            return Ok(s);
        }

        self.create_connection(key).await
    }

    async fn create_connection(&self, key: &HostKey) -> Result<HttpSender, NetworkError> {
        let addr = format!("{}:{}", key.host, key.port);
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|_| NetworkError::ConnectionFailed)?;

        if key.scheme == Scheme::HTTPS {
            let tls = TlsConnector::from(self.inner.tls_config());
            let key = key.clone();
            let domain =
                ServerName::try_from(key.host.clone()).map_err(|_| NetworkError::InvalidDnsName)?;

            let stream = tls
                .connect(domain, stream)
                .await
                .map_err(|_| NetworkError::TlsFailed)?;
            let negotiated_h2 = stream.get_ref().1.alpn_protocol() == Some(b"h2");

            let io = TokioIo::new(stream);
            if negotiated_h2 {
                let (sender, conn) = conn::http2::handshake(TokioExecutor::new(), io)
                    .await
                    .map_err(|_| NetworkError::HttpHandshakeFailed)?;
                self.spawn_connection_task(conn, key);
                Ok(HttpSender::Http2(sender))
            } else {
                let (sender, conn) = conn::http1::handshake(io)
                    .await
                    .map_err(|_| NetworkError::HttpHandshakeFailed)?;
                self.spawn_connection_task(conn, key);
                Ok(HttpSender::Http1(sender))
            }
        } else {
            let (sender, conn) = conn::http1::handshake(TokioIo::new(stream))
                .await
                .map_err(|_| NetworkError::HttpHandshakeFailed)?;

            self.spawn_connection_task(conn, key.clone());
            Ok(HttpSender::Http1(sender))
        }
    }

    fn spawn_connection_task<F>(&self, conn: F, key: HostKey)
    where
        F: std::future::Future<Output = Result<(), hyper::Error>> + 'static,
    {
        let pool = Arc::clone(&self.sender_pool);
        tokio::task::spawn_local(async move {
            let _ = conn.await;
            pool.write().unwrap().remove_connection(&key);
        });
    }
}

/// A `ServerCertVerifier` that accepts any server certificate (dev/diagnostic
/// mode, used when `verify_tls` is disabled).
#[derive(Debug)]
struct SkipServerVerification(Arc<CryptoProvider>);

impl ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn resolve_redirect(base: &Uri, location: &str) -> Result<Uri, NetworkError> {
    if location.starts_with("http://") || location.starts_with("https://") {
        return location.parse().map_err(|_| NetworkError::InvalidUri);
    }

    let scheme = base.scheme_str().unwrap_or("https");
    let authority = base.authority().ok_or(NetworkError::InvalidUri)?;

    let next = if location.starts_with("//") {
        format!("{scheme}:{location}")
    } else if location.starts_with('/') {
        format!("{scheme}://{}{location}", authority)
    } else {
        let base_path = base.path();
        let prefix = base_path.rsplit_once('/').map_or("", |x| x.0);
        format!("{scheme}://{}{prefix}/{location}", authority)
    };

    next.parse().map_err(|_| NetworkError::InvalidUri)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    /// Keep-alive HTTP/1.1 server answering every request with
    /// `ok:<request line>`, so pooled connections stay open across sequential
    /// fetches. Each connection is served until the peer hangs up. Returns
    /// the bound address.
    fn spawn_keep_alive_server(listener: TcpListener) -> String {
        let address = listener.local_addr().unwrap().to_string();
        thread::spawn(move || {
            while let Ok((stream, _)) = listener.accept() {
                thread::spawn(move || serve_keep_alive_connection(stream));
            }
        });
        address
    }

    fn serve_keep_alive_connection(mut stream: std::net::TcpStream) {
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut buffer = [0_u8; 4096];
        loop {
            let mut request = Vec::new();
            loop {
                let read = match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => return,
                    Ok(read) => read,
                };
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    break;
                }
            }
            let request_line = String::from_utf8_lossy(&request)
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
            let body = format!("ok:{request_line}");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            if stream.write_all(response.as_bytes()).is_err() {
                return;
            }
        }
    }

    #[test]
    fn enable_cache_config_is_applied_to_cache() {
        let state = SharedNetState::new();
        assert!(state.cache.is_enabled());

        state.set_network_config(NetworkConfig {
            enable_cache: false,
            ..NetworkConfig::default()
        });
        assert!(!state.cache.is_enabled());
    }

    #[test]
    fn sends_request_method_headers_and_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let read = stream.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .unwrap();
            request
        });

        let core = AsyncNetworkCore::new(Arc::new(SharedNetState::new()));
        let response = core
            .fetch_request_blocking(&NetworkRequest {
                url: format!("http://{address}/submit"),
                method: "POST".to_string(),
                headers: vec![("X-Orinium-Test".to_string(), "yes".to_string())],
                body: b"hello".to_vec(),
            })
            .unwrap();
        assert_eq!(response.body, b"ok");

        let request = String::from_utf8(server.join().unwrap()).unwrap();
        assert!(request.starts_with("POST /submit HTTP/1.1\r\n"));
        assert!(request.to_ascii_lowercase().contains("x-orinium-test: yes"));
        assert!(request.to_ascii_lowercase().contains("accept-language: "));
        assert!(request.ends_with("\r\n\r\nhello"));
    }

    #[test]
    fn pooled_connections_are_not_shared_between_worker_runtimes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = spawn_keep_alive_server(listener);

        let shared = Arc::new(SharedNetState::new());
        let core_a = AsyncNetworkCore::new(Arc::clone(&shared));
        let core_b = AsyncNetworkCore::new(shared);

        // Worker A fetches and returns its keep-alive connection to its own
        // pool; the connection stays open on A's local set.
        let first = core_a
            .fetch_request_blocking(&NetworkRequest::get(format!("http://{address}/first")))
            .unwrap();
        assert_eq!(first.body, b"ok:GET /first HTTP/1.1");

        // Worker B must open its own connection for the same host instead of
        // checking out the sender parked on A's idle runtime, where nobody
        // would ever poll it.
        assert!(
            core_b.sender_pool.read().unwrap().is_empty(),
            "A's pooled connection must not be visible to another worker"
        );
        let second = core_b
            .fetch_request_blocking(&NetworkRequest::get(format!("http://{address}/second")))
            .unwrap();
        assert_eq!(second.body, b"ok:GET /second HTTP/1.1");

        // A still reuses its own pooled connection.
        let third = core_a
            .fetch_request_blocking(&NetworkRequest::get(format!("http://{address}/third")))
            .unwrap();
        assert_eq!(third.body, b"ok:GET /third HTTP/1.1");
    }

    /// Drive the real [`WebView`] pipeline against https://www.youtube.com/
    /// using live fetches, and report how far the page bootstraps.
    ///
    /// Manual probe, not part of the normal suite:
    ///
    /// ```sh
    /// cargo test --lib -- --ignored --nocapture drive_youtube_with_real_fetches
    /// RUST_LOG=info YT_RUN_SECS=60 cargo test --lib -- --ignored --nocapture drive_youtube_with_real_fetches
    /// ```
    #[test]
    #[ignore = "live network probe against youtube.com; manual run only"]
    fn drive_youtube_with_real_fetches() {
        use crate::browser::core::webview::{FetchKind, JsPolicy, WebView, WebViewTask};
        use crate::engine::js::JsFetchResponse;
        use std::collections::BTreeMap;
        use std::time::Instant;
        use url::Url;

        fn fetch_ok(core: &AsyncNetworkCore, url: &Url) -> Option<Response> {
            if url.scheme() == "data" {
                let body =
                    crate::browser::core::resource_loader::DataURI::decode(url.as_str()).ok()?;
                return Some(Response {
                    url: url.to_string(),
                    status: StatusCode(200),
                    reason_phrase: "OK".to_string(),
                    headers: vec![(
                        "Content-Type".to_string(),
                        "application/octet-stream".to_string(),
                    )],
                    body,
                });
            }
            if url.scheme() != "http" && url.scheme() != "https" {
                return None;
            }
            core.fetch_request_blocking(&NetworkRequest {
                url: url.to_string(),
                method: "GET".to_string(),
                headers: vec![
                    (
                        "User-Agent".to_string(),
                        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
                         (KHTML, like Gecko) Chrome/120.0 Safari/537.36"
                            .to_string(),
                    ),
                    ("Accept-Language".to_string(), "en-US,en;q=0.9".to_string()),
                    ("Accept".to_string(), "*/*".to_string()),
                ],
                body: Vec::new(),
            })
            .ok()
        }

        fn count_tags(
            value: &serde_json::Value,
            counts: &mut BTreeMap<String, usize>,
            text: &mut usize,
        ) {
            match value {
                serde_json::Value::Object(obj) => {
                    if let Some(tag) = obj.get("tag") {
                        if let Some(tag) = tag.as_str() {
                            *counts.entry(tag.to_string()).or_insert(0) += 1;
                        }
                    }
                    if let Some(t) = obj.get("text") {
                        if let Some(t) = t.as_str() {
                            *text += t.len();
                        }
                    }
                    if let Some(children) = obj.get("children") {
                        if let Some(children) = children.as_array() {
                            for child in children {
                                count_tags(child, counts, text);
                            }
                        }
                    }
                }
                serde_json::Value::Array(arr) => {
                    for child in arr {
                        count_tags(child, counts, text);
                    }
                }
                _ => {}
            }
        }

        let run_secs: u64 = std::env::var("YT_RUN_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(45)
            .min(120);

        let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
            .try_init();

        let core = AsyncNetworkCore::new(Arc::new(SharedNetState::new()));
        let base = Url::parse("https://www.youtube.com/").unwrap();

        log::info!(target: "ytprobe", "fetching initial HTML (cap {}s)", run_secs);
        let initial = core
            .fetch_request_blocking(&NetworkRequest {
                url: base.to_string(),
                method: "GET".to_string(),
                headers: vec![
                    (
                        "User-Agent".to_string(),
                        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
                         (KHTML, like Gecko) Chrome/120.0 Safari/537.36"
                            .to_string(),
                    ),
                    ("Accept-Language".to_string(), "en-US,en;q=0.9".to_string()),
                    (
                        "Accept".to_string(),
                        "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
                            .to_string(),
                    ),
                ],
                body: Vec::new(),
            })
            .expect("fetch youtube.com");
        let html = String::from_utf8_lossy(&initial.body).to_string();
        log::info!(target: "ytprobe", "youtube.com HTML: {} bytes", html.len());

        let mut wv = WebView::new(
            crate::engine::layouter::types::ColorScheme::Light,
            JsPolicy::Enabled,
        );
        wv.relayout((1280.0, 720.0));

        let mut fetch_counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        let start = Instant::now();
        let mut last_dom_version = 0u64;
        let mut stable_for: u32 = 0;
        let mut layout_seen = false;
        let mut tick_iterations: u64 = 0;

        loop {
            tick_iterations += 1;
            if tick_iterations % 200 == 0 {
                let mut kinds = String::new();
                for (k, v) in &fetch_counts {
                    if !kinds.is_empty() {
                        kinds.push(' ');
                    }
                    kinds.push_str(&format!("{k}={v}"));
                }
                eprintln!(
                    "[ytprobe] t={:.1}s ticks={} tasks={{{}}} dom={} layout={}",
                    start.elapsed().as_secs_f64(),
                    tick_iterations,
                    kinds,
                    wv.inspect("getVersion", "{}")
                        .ok()
                        .and_then(|v| v["domVersion"].as_u64())
                        .unwrap_or(0),
                    wv.layout_and_info().is_some(),
                );
            }
            if start.elapsed().as_secs() >= run_secs {
                break;
            }
            let tasks = wv.tick();
            for task in tasks {
                match task {
                    WebViewTask::AskTabHtml => {
                        wv.on_html_fetched(html.clone(), base.clone());
                        *fetch_counts.entry("html").or_insert(0) += 1;
                    }
                    WebViewTask::DevToolsRequest { id, .. } => {
                        wv.on_devtools_response(id, "{}".to_string());
                        *fetch_counts.entry("devtools").or_insert(0) += 1;
                    }
                    WebViewTask::Fetch { url, kind } => {
                        let label: &'static str = match &kind {
                            FetchKind::Html => "html",
                            FetchKind::Css => "css",
                            FetchKind::Script { .. } => "script",
                            FetchKind::DynamicScript { .. } => "dyn-script",
                            FetchKind::DynamicCss { .. } => "dyn-css",
                            FetchKind::Image { .. } => "image",
                            FetchKind::Audio { .. } => "audio",
                            FetchKind::JavaScript { .. } => "js-fetch",
                            FetchKind::Iframe { .. } => "iframe",
                        };
                        *fetch_counts.entry(label).or_insert(0) += 1;
                        match kind {
                            FetchKind::Html => {
                                if let Some(r) = fetch_ok(&core, &url) {
                                    let text = String::from_utf8_lossy(&r.body).to_string();
                                    wv.on_html_fetched(text, url);
                                }
                            }
                            FetchKind::Css => {
                                if let Some(r) = fetch_ok(&core, &url) {
                                    wv.on_css_fetched_from(
                                        String::from_utf8_lossy(&r.body).to_string(),
                                        &url,
                                    );
                                }
                            }
                            FetchKind::Script { index } => match fetch_ok(&core, &url) {
                                Some(r) => wv.on_script_fetched(
                                    index,
                                    String::from_utf8_lossy(&r.body).to_string(),
                                ),
                                None => wv.on_script_fetch_failed(index),
                            },
                            FetchKind::DynamicScript { node_id } => match fetch_ok(&core, &url) {
                                Some(r) => wv.on_dynamic_script_fetched(
                                    node_id,
                                    String::from_utf8_lossy(&r.body).to_string(),
                                ),
                                None => wv.on_dynamic_script_fetch_failed(node_id),
                            },
                            FetchKind::DynamicCss { node_id } => {
                                if let Some(r) = fetch_ok(&core, &url) {
                                    wv.on_dynamic_style_fetched(
                                        node_id,
                                        String::from_utf8_lossy(&r.body).to_string(),
                                    );
                                }
                            }
                            FetchKind::Image { source } => {
                                if let Some(r) = fetch_ok(&core, &url) {
                                    if let Err(e) = wv.on_image_fetched(source, &r.body) {
                                        log::warn!(target: "ytprobe", "image fetch rejected: {e}");
                                    }
                                }
                            }
                            FetchKind::Audio { source } => {
                                if let Some(r) = fetch_ok(&core, &url) {
                                    wv.on_audio_fetched(source, &r.body);
                                }
                            }
                            FetchKind::JavaScript {
                                request_id,
                                method,
                                headers,
                                body,
                            } => match core.fetch_request_blocking(&NetworkRequest {
                                url: url.to_string(),
                                method,
                                headers,
                                body,
                            }) {
                                Ok(r) => wv.on_js_fetch_succeeded(
                                    request_id,
                                    JsFetchResponse {
                                        url: r.url,
                                        status: r.status.as_u16(),
                                        status_text: r.reason_phrase,
                                        redirected: false,
                                        body: r.body,
                                        headers: r.headers,
                                    },
                                ),
                                Err(e) => {
                                    wv.on_js_fetch_failed(request_id, e.to_string());
                                }
                            },
                            FetchKind::Iframe { dom_id } => {
                                wv.on_iframe_fetch_failed(dom_id);
                            }
                        }
                    }
                    WebViewTask::Navigate { url } => {
                        // Simulate the tab's script-initiated navigation by
                        // re-serving the fetched HTML at the new URL.
                        if let Some(r) = fetch_ok(&core, &url) {
                            let text = String::from_utf8_lossy(&r.body).to_string();
                            wv.on_html_fetched(text, url);
                            *fetch_counts.entry("nav").or_insert(0) += 1;
                        }
                    }
                }
            }

            if wv.layout_and_info().is_some() {
                layout_seen = true;
            }
            let dom_version = wv
                .inspect("getVersion", "{}")
                .ok()
                .and_then(|v| v["domVersion"].as_u64())
                .unwrap_or(0);
            if layout_seen && dom_version == last_dom_version {
                stable_for += 1;
            } else {
                stable_for = 0;
                last_dom_version = dom_version;
            }
            if stable_for > 250 {
                log::info!(target: "ytprobe", "DOM stable for 2.5s; stopping");
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let elapsed = start.elapsed();
        eprintln!(
            "== YOUTUBE PROBE SUMMARY ({:.1}s, {} ticks) ==",
            elapsed.as_secs_f64(),
            tick_iterations
        );
        eprintln!("fetches: {fetch_counts:?}");
        let version = wv.inspect("getVersion", "{}");
        if let Ok(v) = &version {
            eprintln!("versions: {v}");
        }

        match wv.inspect("getDocument", "{}") {
            Ok(doc) => {
                let mut counts: BTreeMap<String, usize> = BTreeMap::new();
                let mut text_len = 0usize;
                count_tags(&doc, &mut counts, &mut text_len);
                eprintln!("document text length: {text_len}");
                eprintln!("element tag counts: {counts:?}");
                for marker in [
                    "video",
                    "ytd-app",
                    "ytd-masthead",
                    "ytd-searchbox",
                    "ytd-player",
                    "ytd-thumbnail",
                    "ytp-cued-thumbnail-overlay-image",
                    "input",
                    "iframe",
                ] {
                    eprintln!(
                        "marker <{marker}>: {}",
                        counts.get(marker).copied().unwrap_or(0)
                    );
                }
            }
            Err(e) => eprintln!("getDocument failed: {e}"),
        }

        if let Some((layout, info)) = wv.layout_and_info() {
            eprintln!(
                "layout present: layout-root children={}, info children={}",
                layout.children.len(),
                info.children.len()
            );
        } else {
            eprintln!("layout: none");
        }
    }
}
