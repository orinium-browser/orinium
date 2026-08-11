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
use std::sync::Arc;
use tokio::{net::TcpStream, runtime::Runtime, task::LocalSet};
use tokio_rustls::TlsConnector;

pub(super) struct AsyncNetworkCore {
    local: LocalSet,
    rt: Runtime,
    inner: NetworkInner,
}

impl AsyncNetworkCore {
    pub fn new() -> Self {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build tokio runtime");

        let local = LocalSet::new();

        Self {
            rt,
            local,
            inner: NetworkInner::new(),
        }
    }

    pub fn set_network_config(&mut self, config: NetworkConfig) {
        self.inner.set_network_config(config)
    }

    /// Removes all cached responses.
    pub fn clear_cache(&self) {
        self.inner.cache.clear();
    }
    pub fn fetch_request_blocking(
        &self,
        request: &NetworkRequest,
    ) -> Result<Response, NetworkError> {
        // network スレッド内で完結させる
        self.local
            .block_on(&self.rt, async { self.inner.fetch_request(request).await })
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

pub(super) struct NetworkInner {
    sender_pool: Arc<std::sync::RwLock<SenderPool>>,
    tls_config: Arc<ClientConfig>,
    network_config: Arc<NetworkConfig>,
    cache: super::Cache,
}

impl NetworkInner {
    pub fn new() -> Self {
        Self {
            sender_pool: Arc::new(std::sync::RwLock::new(SenderPool::new())),
            tls_config: Arc::new(Self::build_tls_config()),
            network_config: Arc::new(NetworkConfig::default()),
            cache: super::Cache::new(),
        }
    }

    pub fn set_network_config(&mut self, config: NetworkConfig) {
        self.cache.set_enabled(config.enable_cache);
        self.network_config = Arc::new(config)
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

    pub async fn fetch_request(&self, request: &NetworkRequest) -> Result<Response, NetworkError> {
        let mut current: Uri = request.url.parse().map_err(|_| NetworkError::InvalidUri)?;
        let mut method = Method::from_bytes(request.method.as_bytes())
            .map_err(|_| NetworkError::HttpRequestFailed)?;
        let mut body = request.body.clone();
        let mut redirects = 0usize;

        loop {
            if method == Method::GET
                && let Some(cached) = self.cache.get(&current.to_string())
            {
                log::info!("NetworkCache: hit for url={}", current);
                return Ok(cached);
            }

            let resp = self
                .send_request(&current, &method, &request.headers, &body)
                .await?;

            if self.network_config.follow_redirects
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
                self.cache.set(&current.to_string(), &resp);
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

        let mut request = Request::builder()
            .method(method.clone())
            .uri(uri.path_and_query().map_or("/", |p| p.as_str()))
            .header("Host", host)
            .header("User-Agent", self.network_config.user_agent.as_str());
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
            let tls = TlsConnector::from(Arc::clone(&self.tls_config));
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

    #[test]
    fn enable_cache_config_is_applied_to_cache() {
        let mut inner = NetworkInner::new();
        assert!(inner.cache.is_enabled());

        inner.set_network_config(NetworkConfig {
            enable_cache: false,
            ..NetworkConfig::default()
        });
        assert!(!inner.cache.is_enabled());
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

        let core = AsyncNetworkCore::new();
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
        assert!(request.ends_with("\r\n\r\nhello"));
    }
}
