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

/// Upper bound on a decoded response body (before or after decompression),
/// guarding against memory exhaustion from oversized or decompression-bomb
/// responses.
const MAX_RESPONSE_BODY_BYTES: usize = 64 * 1024 * 1024;

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
pub struct AsyncNetworkCore {
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
pub struct StatusCode(pub(crate) u16);

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
pub struct SharedNetState {
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
                .with_custom_certificate_verifier(Arc::new(SkipServerVerification(Arc::clone(
                    provider,
                ))))
                .with_no_client_auth()
        };
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        config
    }
}

impl Default for SharedNetState {
    fn default() -> Self {
        Self::new()
    }
}

impl AsyncNetworkCore {
    pub async fn fetch_request(&self, request: &NetworkRequest) -> Result<Response, NetworkError> {
        let mut current: Uri = request.url.parse().map_err(|_| NetworkError::InvalidUri)?;
        let mut method = Method::from_bytes(request.method.as_bytes())
            .map_err(|_| NetworkError::HttpRequestFailed)?;
        let mut body = request.body.clone();
        let mut headers = request.headers.clone();
        let user_agent = self.inner.network_config.read().unwrap().user_agent.clone();
        let mut redirects = 0usize;

        loop {
            let cache_headers = effective_cache_headers(&headers, &user_agent);
            if method == Method::GET
                && let Some(cached) = self
                    .inner
                    .cache
                    .get_for_request(&current.to_string(), &cache_headers)
            {
                log::info!("NetworkCache: hit for url={}", current);
                return Ok(cached);
            }

            let read_timeout = self.inner.network_config.read().unwrap().read_timeout;
            let resp = tokio::time::timeout(
                read_timeout,
                self.send_request(&current, &method, &headers, &body),
            )
            .await
            .map_err(|_| NetworkError::Timeout)??;

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
                    let next = resolve_redirect(&current, loc)?;
                    strip_origin_bound_headers_on_cross_origin_redirect(
                        &current,
                        &next,
                        &mut headers,
                    );
                    current = next;
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
                self.inner
                    .cache
                    .set_for_request(&current.to_string(), &resp, &cache_headers);
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
            .header(
                "User-Agent",
                self.inner.network_config.read().unwrap().user_agent.clone(),
            );
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
                if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BODY_BYTES {
                    return Err(NetworkError::ResponseTooLarge);
                }
                body.extend_from_slice(chunk);
            }
        }

        let content_encoding = res
            .headers()
            .get("content-encoding")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.to_string());
        let body = Self::decode_content(content_encoding.as_deref(), body)?;

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
    /// the body untouched; a decoded body larger than
    /// [`MAX_RESPONSE_BODY_BYTES`] is rejected.
    fn decode_content(encoding: Option<&str>, mut body: Vec<u8>) -> Result<Vec<u8>, NetworkError> {
        let Some(encoding) = encoding else {
            return Ok(body);
        };
        for part in encoding.split(',').map(|s| s.trim().to_ascii_lowercase()) {
            let decoded = match part.as_str() {
                "gzip" | "x-gzip" => Self::read_decompressed(GzDecoder::new(&body[..]))?,
                "deflate" => match Self::read_decompressed(ZlibDecoder::new(&body[..]))? {
                    Some(decoded) => Some(decoded),
                    None => Self::read_decompressed(RawDeflateDecoder::new(&body[..]))?,
                },
                "br" => Self::read_decompressed(BrotliDecompressor::new(&body[..], 1 << 16))?,
                _ => None,
            };
            if let Some(decoded) = decoded {
                body = decoded;
            }
        }
        Ok(body)
    }

    /// Reads a decompression stream to completion.
    ///
    /// Returns `Ok(None)` when the stream cannot be decoded (matching the
    /// previous lenient behavior of leaving the body untouched), and
    /// `Err(ResponseTooLarge)` when the decoded output would exceed
    /// [`MAX_RESPONSE_BODY_BYTES`].
    fn read_decompressed<R: std::io::Read>(reader: R) -> Result<Option<Vec<u8>>, NetworkError> {
        let mut output = Vec::new();
        let mut limited = reader.take(MAX_RESPONSE_BODY_BYTES as u64 + 1);
        match limited.read_to_end(&mut output) {
            Ok(_) if output.len() > MAX_RESPONSE_BODY_BYTES => Err(NetworkError::ResponseTooLarge),
            Ok(_) => Ok(Some(output)),
            Err(_) => Ok(None),
        }
    }

    async fn get_or_create_sender(&self, key: &HostKey) -> Result<HttpSender, NetworkError> {
        if let Some(s) = self.sender_pool.write().unwrap().get_connection(key) {
            return Ok(s);
        }

        self.create_connection(key).await
    }

    async fn create_connection(&self, key: &HostKey) -> Result<HttpSender, NetworkError> {
        let addr = format!("{}:{}", key.host, key.port);
        let connect_timeout = self.inner.network_config.read().unwrap().connect_timeout;
        let stream = tokio::time::timeout(connect_timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| NetworkError::Timeout)?
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

fn uri_origin(uri: &Uri) -> Option<String> {
    let url = url::Url::parse(&uri.to_string()).ok()?;
    Some(url.origin().ascii_serialization())
}

fn is_origin_bound_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization" | "proxy-authorization" | "cookie" | "cookie2" | "host"
    )
}

fn strip_origin_bound_headers(headers: &mut Vec<(String, String)>) {
    headers.retain(|(name, _)| !is_origin_bound_header(name));
}

fn strip_origin_bound_headers_on_cross_origin_redirect(
    current: &Uri,
    next: &Uri,
    headers: &mut Vec<(String, String)>,
) {
    let origins_match = matches!(
        (uri_origin(current), uri_origin(next)),
        (Some(current), Some(next)) if current == next
    );
    if !origins_match {
        strip_origin_bound_headers(headers);
    }
}

fn effective_cache_headers(
    request_headers: &[(String, String)],
    user_agent: &str,
) -> Vec<(String, String)> {
    let mut headers = request_headers.to_vec();
    let defaults = [
        ("user-agent", user_agent.to_string()),
        (
            "accept-language",
            crate::platform::locale::accept_language_header(),
        ),
        ("accept-encoding", "gzip, deflate, br".to_string()),
    ];
    for (name, value) in defaults {
        if !headers
            .iter()
            .any(|(header, _)| header.eq_ignore_ascii_case(name))
        {
            headers.push((name.to_string(), value));
        }
    }
    headers
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

    #[test]
    fn redirect_strips_credentials_only_when_origin_changes() {
        let current: Uri = "https://example.test/start".parse().unwrap();
        let same_origin: Uri = "https://example.test:443/next".parse().unwrap();
        let cross_origin: Uri = "https://other.test/next".parse().unwrap();
        let mut headers = vec![
            ("Authorization".to_string(), "Bearer secret".to_string()),
            ("Cookie".to_string(), "session=secret".to_string()),
            ("X-Request".to_string(), "keep".to_string()),
        ];

        strip_origin_bound_headers_on_cross_origin_redirect(&current, &same_origin, &mut headers);
        assert_eq!(headers.len(), 3);
        strip_origin_bound_headers_on_cross_origin_redirect(&current, &cross_origin, &mut headers);
        assert_eq!(headers, [("X-Request".to_string(), "keep".to_string())]);
    }

    #[test]
    fn redirect_never_escalates_scheme_to_file() {
        let base: Uri = "https://example.test/start".parse().unwrap();
        let next = resolve_redirect(&base, "file:///etc/passwd").unwrap();
        assert_eq!(next.scheme_str(), Some("https"));
        assert_eq!(next.authority().unwrap().as_str(), "example.test");
    }

    #[test]
    fn redirect_never_escalates_scheme_to_resource() {
        let base: Uri = "https://example.test/start".parse().unwrap();
        let next = resolve_redirect(&base, "resource:///devtools/index.html").unwrap();
        assert_eq!(next.scheme_str(), Some("https"));
        assert_eq!(next.authority().unwrap().as_str(), "example.test");
    }

    #[test]
    fn redirect_keeps_http_and_https_absolute_locations() {
        let base: Uri = "https://example.test/start".parse().unwrap();
        assert_eq!(
            resolve_redirect(&base, "http://other.test/x")
                .unwrap()
                .scheme_str(),
            Some("http")
        );
        assert_eq!(
            resolve_redirect(&base, "https://other.test/x")
                .unwrap()
                .scheme_str(),
            Some("https")
        );
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

    #[test]
    fn decode_content_gzip_round_trips() {
        use flate2::{Compression, write::GzEncoder};
        use std::io::Write as _;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"hello world").unwrap();
        let compressed = encoder.finish().unwrap();

        let decoded =
            AsyncNetworkCore::decode_content(Some("gzip"), compressed).expect("decodes gzip");
        assert_eq!(decoded, b"hello world");
    }

    #[test]
    fn decode_content_rejects_decompression_bomb() {
        use flate2::{Compression, write::GzEncoder};
        use std::io::Write as _;

        // Compresses to a few KiB but expands past the body limit.
        let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
        encoder
            .write_all(&vec![0_u8; MAX_RESPONSE_BODY_BYTES + 1024])
            .unwrap();
        let compressed = encoder.finish().unwrap();
        assert!(
            compressed.len() < MAX_RESPONSE_BODY_BYTES,
            "payload must be a bomb (small on the wire)"
        );

        assert!(matches!(
            AsyncNetworkCore::decode_content(Some("gzip"), compressed),
            Err(NetworkError::ResponseTooLarge)
        ));
    }
}
