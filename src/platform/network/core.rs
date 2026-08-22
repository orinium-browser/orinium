//! ネットワークコア
//! HTTP通信とレスポンス処理を担当する。

use super::{HostKey, HttpSender, NetworkConfig, NetworkError, NetworkRequest, SenderPool};

use http_body_util::{BodyExt, Full};
use hyper::{
    Method, Request, Uri,
    body::{Bytes, Incoming},
    client::conn,
    http::uri::Scheme,
};
use hyper_util::rt::TokioIo;
use rustls::{ClientConfig, RootCertStore};
use rustls_native_certs::load_native_certs;
use serde::{Deserialize, Serialize};
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
    network_config: RwLock<Arc<NetworkConfig>>,
    cache: super::Cache,
}

impl SharedNetState {
    pub fn new() -> Self {
        Self {
            tls_config: Arc::new(Self::build_tls_config()),
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

    fn build_tls_config() -> ClientConfig {
        let mut roots = RootCertStore::empty();
        let result = load_native_certs();

        for cert in result.certs {
            let _ = roots.add(cert);
        }

        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth()
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

        let user_agent = self
            .inner
            .network_config
            .read()
            .unwrap()
            .user_agent
            .clone();
        let mut request = Request::builder()
            .method(method.clone())
            .uri(uri.path_and_query().map_or("/", |p| p.as_str()))
            .header("Host", host)
            .header("User-Agent", user_agent);
        if !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("accept-language"))
        {
            request = request.header(
                "Accept-Language",
                crate::platform::locale::accept_language_header(),
            );
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
            _ => {
                return Err(NetworkError::UnsupportedHttpVersion);
            }
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

        Ok(Response {
            url,
            status: status.into(),
            reason_phrase,
            headers,
            body,
        })
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
            let tls = TlsConnector::from(Arc::clone(&self.inner.tls_config));
            let key = key.clone();
            let domain = rustls::pki_types::ServerName::try_from(key.host.clone())
                .map_err(|_| NetworkError::InvalidDnsName)?;

            let stream = tls
                .connect(domain, stream)
                .await
                .map_err(|_| NetworkError::TlsFailed)?;

            let (sender, conn) = conn::http1::handshake(TokioIo::new(stream))
                .await
                .map_err(|_| NetworkError::HttpHandshakeFailed)?;

            self.spawn_connection_task(conn, key);
            Ok(HttpSender::Http1(sender))
        } else {
            let (sender, conn) = conn::http1::handshake(TokioIo::new(stream))
                .await
                .map_err(|_| NetworkError::HttpHandshakeFailed)?;

            self.spawn_connection_task(conn, key.clone());
            Ok(HttpSender::Http1(sender))
        }
    }

    fn spawn_connection_task(
        &self,
        conn: conn::http1::Connection<
            TokioIo<impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + 'static>,
            Full<Bytes>,
        >,
        key: HostKey,
    ) {
        let pool = Arc::clone(&self.sender_pool);
        tokio::task::spawn_local(async move {
            let _ = conn.await;
            pool.write().unwrap().remove_connection(&key);
        });
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
}
