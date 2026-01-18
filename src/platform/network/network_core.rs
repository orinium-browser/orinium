use http_body_util::BodyExt;
use http_body_util::Empty;
use hyper::Method;
use hyper::body::Bytes;
use hyper::client::conn;
use hyper::{Request, Uri};
use hyper_util::rt::TokioIo;
use rustls::ClientConfig;
use rustls::RootCertStore;
use rustls_native_certs::load_native_certs;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio::time::{Duration, sleep};
use tokio_rustls::TlsConnector;

use super::{HostKey, HttpSender, NetworkConfig, SenderPool};

/// Represents an HTTP response.
pub struct Response {
    pub status: hyper::StatusCode,
    pub reason_phrase: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Core network component for sending HTTP/HTTPS requests.
pub struct NetworkCore {
    network_config: Arc<NetworkConfig>,
    sender_pool: Arc<RwLock<SenderPool>>,
    tls_config: Arc<ClientConfig>,
}

impl Default for NetworkCore {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkCore {
    /// Creates a new NetworkCore with TLS configuration and sender pool.
    pub fn new() -> Self {
        // Load system root certificates for TLS
        let mut root_store = RootCertStore::empty();
        let load_result = load_native_certs();
        let certs = load_result.certs;
        let errors = load_result.errors;

        for cert in certs {
            root_store.add(cert).unwrap_or_else(|e| {
                eprintln!("Error adding certificate: {e:?}");
            });
        }
        println!("Loaded {} certificates", root_store.len());

        if !errors.is_empty() {
            eprintln!("{} errors while loading system certificates:", errors.len());
            for e in errors {
                eprintln!("{e:?}");
            }
        }

        let tls_config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        Self {
            network_config: Arc::new(NetworkConfig::default()),
            sender_pool: Arc::new(RwLock::new(SenderPool::new())),
            tls_config: Arc::new(tls_config),
        }
    }

    /// Sends an HTTP request to the given URL using the specified method.
    /// Reuses connections from the pool if possible.
    pub async fn send_request(
        &self,
        url: &str,
        method: Method,
    ) -> Result<Response, Box<dyn std::error::Error + Send + Sync + 'static>> {
        let url: Uri = url.parse()?;
        let host = url
            .host()
            .ok_or_else(|| anyhow::anyhow!("URI has no host"))?;
        let scheme = url.scheme().unwrap_or(&hyper::http::uri::Scheme::HTTP);
        let default_port = if scheme == &hyper::http::uri::Scheme::HTTPS {
            443
        } else {
            80
        };
        let port = url.port_u16().unwrap_or(default_port);
        let addr = format!("{host}:{port}");

        let key = Arc::new(HostKey {
            scheme: scheme.clone(),
            host: host.to_string(),
            port,
        });

        // Attempt to get a connection from the pool
        let mut sender = match self.sender_pool.read().await.get_connection(&key).await {
            Some(sender) => sender,
            None => {
                // Create new connection (HTTP or HTTPS)
                self.create_connection(&addr, scheme, host, key.clone())
                    .await?
            }
        };

        let authority = url.authority().unwrap();
        let path = url.path_and_query().map(|p| p.as_str()).unwrap_or("/");

        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("Host", authority.as_str())
            .header("User-Agent", self.network_config.user_agent.as_str())
            .body(Empty::<Bytes>::new())?;

        let mut res = match &mut sender {
            HttpSender::Http1(s) => s.send_request(req).await?,
            _ => {
                // TODO: Implement HTTP/2 support
                return Err(anyhow::anyhow!("HTTP/2 not implemented yet").into());
            }
        };

        // Collect response
        let status = res.status();
        let reason_phrase = status.canonical_reason().unwrap_or("").to_string();
        let headers = res
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
            .collect::<Vec<(String, String)>>();

        let mut body = Vec::new();
        while let Some(next) = res.frame().await {
            let frame = next?;
            if let Some(chunk) = frame.data_ref() {
                body.extend_from_slice(chunk);
            }
        }

        // Return connection to pool
        self.sender_pool
            .write()
            .await
            .add_connection((*key).clone(), sender)
            .await;

        Ok(Response {
            status,
            reason_phrase,
            headers,
            body,
        })
    }

    /// Creates a new connection (HTTP or HTTPS) and spawns a task to monitor it.
    async fn create_connection(
        &self,
        addr: &str,
        scheme: &hyper::http::uri::Scheme,
        host: &str,
        key: Arc<HostKey>,
    ) -> Result<HttpSender, Box<dyn std::error::Error + Send + Sync>> {
        if scheme == &hyper::http::uri::Scheme::HTTPS {
            // HTTPS connection
            let stream = TcpStream::connect(addr).await?;
            let tls = TlsConnector::from(self.tls_config.clone());
            let domain = rustls::pki_types::ServerName::try_from(host)
                .map_err(|_| anyhow::anyhow!("Invalid DNS name"))?
                .to_owned();
            let tls_stream = tls.connect(domain, stream).await?;
            let io = TokioIo::new(tls_stream);

            let (sender, connection) = conn::http1::handshake(io).await?;
            let sender = HttpSender::Http1(sender);

            let pool_clone = self.sender_pool.clone();
            tokio::spawn(async move {
                if let Err(err) = connection.await {
                    eprintln!("HTTPS connection failed: {err:?}");
                    pool_clone.write().await.remove_connection(&key).await;
                }
            });

            Ok(sender)
        } else {
            // HTTP connection
            let stream = TcpStream::connect(addr).await?;
            let io = TokioIo::new(stream);

            let (sender, connection) = conn::http1::handshake(io).await?;
            let sender = HttpSender::Http1(sender);

            let pool_clone = self.sender_pool.clone();
            tokio::spawn(async move {
                if let Err(err) = connection.await {
                    eprintln!("HTTP connection failed: {err:?}");
                    pool_clone.write().await.remove_connection(&key).await;
                }
            });

            Ok(sender)
        }
    }

    /// Fetches the URL with automatic redirect handling and retry logic.
    /// Limits redirects to 10 and retries to 5 for server errors.
    pub async fn fetch_url(
        &self,
        url: &str,
    ) -> Result<Response, Box<dyn std::error::Error + Send + Sync + 'static>> {
        let mut current = url.to_string();
        let mut redirects = 0usize;
        let max_redirects = 10usize;
        let max_retries = 5usize;

        loop {
            let mut attempt = 0usize;
            let resp = loop {
                let r = self.send_request(&current, Method::GET).await?;
                if r.status.is_server_error() && attempt < max_retries {
                    attempt += 1;
                    let backoff = 100u64.saturating_mul(1u64 << (attempt - 1));
                    sleep(Duration::from_millis(backoff)).await;
                    continue;
                }
                break r;
            };

            if resp.status.is_redirection() {
                if redirects >= max_redirects {
                    return Err(anyhow::anyhow!("Too many redirects").into());
                }

                // Extract "Location" header
                let location = resp
                    .headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("location"))
                    .map(|(_, v)| v.clone());

                if let Some(loc) = location {
                    let base_uri: Uri = current.parse()?;
                    let next_url = if loc.starts_with("http://") || loc.starts_with("https://") {
                        loc
                    } else if loc.starts_with("//") {
                        let scheme = base_uri.scheme_str().unwrap_or("https");
                        format!("{scheme}:{loc}")
                    } else if loc.starts_with('/') {
                        let scheme = base_uri.scheme_str().unwrap_or("https");
                        let authority = base_uri
                            .authority()
                            .ok_or_else(|| anyhow::anyhow!("base URI has no authority"))?
                            .as_str();
                        format!("{scheme}://{authority}{loc}")
                    } else {
                        let scheme = base_uri.scheme_str().unwrap_or("https");
                        let authority = base_uri
                            .authority()
                            .ok_or_else(|| anyhow::anyhow!("base URI has no authority"))?
                            .as_str();
                        let base_path =
                            base_uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
                        let prefix = if let Some(pos) = base_path.rfind('/') {
                            &base_path[..=pos]
                        } else {
                            "/"
                        };
                        format!("{scheme}://{authority}{prefix}{loc}")
                    };

                    current = next_url;
                    redirects += 1;
                    continue;
                } else {
                    return Ok(resp);
                }
            }

            return Ok(resp);
        }
    }
}
