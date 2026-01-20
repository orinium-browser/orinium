use super::{HostKey, HttpSender, NetworkConfig, NetworkError, SenderPool};

use http_body_util::{BodyExt, Empty};
use hyper::{
    Method, Request, Uri,
    body::{Bytes, Incoming},
    client::conn,
    http::uri::Scheme,
};
use hyper_util::rt::TokioIo;
use rustls::{ClientConfig, RootCertStore};
use rustls_native_certs::load_native_certs;
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

    /// UI スレッドなどから呼ばれる blocking API
    pub fn fetch_blocking(&self, url: &str) -> Result<Response, NetworkError> {
        // network スレッド内で完結させる
        self.local
            .block_on(&self.rt, async { self.inner.fetch_url(url).await })
    }
}

/// HTTP response
pub struct Response {
    pub status: hyper::StatusCode,
    pub reason_phrase: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub(super) struct NetworkInner {
    sender_pool: Arc<std::sync::RwLock<SenderPool>>,
    tls_config: Arc<ClientConfig>,
    network_config: Arc<NetworkConfig>,
}

impl NetworkInner {
    pub fn new() -> Self {
        Self {
            sender_pool: Arc::new(std::sync::RwLock::new(SenderPool::new())),
            tls_config: Arc::new(Self::build_tls_config()),
            network_config: Arc::new(NetworkConfig::default()),
        }
    }

    pub fn set_network_config(&mut self, confing: NetworkConfig) {
        self.network_config = Arc::new(confing)
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

    pub async fn fetch_url(&self, url: &str) -> Result<Response, NetworkError> {
        let mut current: Uri = url.parse().map_err(|_| NetworkError::InvalidUri)?;
        let mut redirects = 0usize;

        loop {
            let resp = self.send_request(&current).await?;

            if self.network_config.follow_redirects && resp.status.is_redirection() {
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
                    redirects += 1;
                    continue;
                }
            }

            return Ok(resp);
        }
    }

    async fn send_request(&self, uri: &Uri) -> Result<Response, NetworkError> {
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

        let req = Request::builder()
            .method(Method::GET)
            .uri(uri.path_and_query().map(|p| p.as_str()).unwrap_or("/"))
            .header("Host", host)
            .header("User-Agent", self.network_config.user_agent.as_str())
            .body(Empty::<Bytes>::new())
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

        let response = Self::collect_response(&mut res).await?;

        self.sender_pool
            .write()
            .unwrap()
            .add_connection(key, sender);

        Ok(response)
    }

    async fn collect_response(
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
            status,
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
            let tls = TlsConnector::from(self.tls_config.clone());
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
            Empty<Bytes>,
        >,
        key: HostKey,
    ) {
        let pool = self.sender_pool.clone();
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
        let prefix = base_path.rsplit_once('/').map(|x| x.0).unwrap_or("");
        format!("{scheme}://{}{prefix}/{location}", authority)
    };

    next.parse().map_err(|_| NetworkError::InvalidUri)
}
