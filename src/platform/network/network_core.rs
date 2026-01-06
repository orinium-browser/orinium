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

use crate::network::{HostKey, HttpSender, SenderPool};

pub struct Response {
    pub status: hyper::StatusCode,
    pub reason_phrase: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub struct NetworkCore {
    sender_pool: Arc<RwLock<SenderPool>>,
    tls_config: Arc<ClientConfig>,
}

impl Default for NetworkCore {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkCore {
    pub fn new() -> Self {
        // TLS設定を作成
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
            eprintln!(
                "There were {} error(s) while loading system certificates:",
                errors.len()
            );
            for e in errors {
                eprintln!("{e:?}");
            }
        }

        let tls_config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        Self {
            sender_pool: Arc::new(RwLock::new(SenderPool::new())),
            tls_config: Arc::new(tls_config),
        }
    }

    pub async fn send_request(
        &self,
        url: &str,
        method: Method,
    ) -> Result<Response, Box<dyn std::error::Error + Send + Sync + 'static>> {
        let url: Uri = url.parse()?;
        let host = url.host().expect("uri has no host");

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

        let mut sender = match self.sender_pool.read().await.get_connection(&key).await {
            Some(sender) => sender,
            _ => {
                if scheme == &hyper::http::uri::Scheme::HTTPS {
                    // HTTPS接続
                    let stream = TcpStream::connect(&addr).await?;
                    let tls = TlsConnector::from(self.tls_config.clone());
                    let domain = rustls::pki_types::ServerName::try_from(host)
                        .map_err(|_| anyhow::anyhow!("Invalid DNS name"))?
                        .to_owned();
                    let tls_stream = tls.connect(domain, stream).await?;
                    let io = TokioIo::new(tls_stream);

                    let (sender, connection) = conn::http1::handshake(io).await?;
                    let sender = HttpSender::Http1(sender);

                    let key_clone = key.clone();
                    let pool_clone = self.sender_pool.clone();
                    tokio::spawn(async move {
                        if let Err(err) = connection.await {
                            eprintln!("HTTPS Connection failed: {err:?}");
                            let pool = pool_clone.write().await;
                            pool.remove_connection(&key_clone).await;
                        }
                    });

                    sender
                } else {
                    // HTTP接続
                    let stream = TcpStream::connect(&addr).await?;
                    let io = TokioIo::new(stream);

                    let (sender, connection) = conn::http1::handshake(io).await?;
                    let sender = HttpSender::Http1(sender);

                    let key_clone = key.clone();
                    let pool_clone = self.sender_pool.clone();
                    tokio::spawn(async move {
                        if let Err(err) = connection.await {
                            eprintln!("HTTP Connection failed: {err:?}");
                            let pool = pool_clone.write().await;
                            pool.remove_connection(&key_clone).await;
                        }
                    });

                    sender
                }
            }
        };

        let authority = url.authority().unwrap();
        let path = url.path_and_query().map(|p| p.as_str()).unwrap_or("/");

        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("Host", authority.as_str())
            .body(Empty::<Bytes>::new())?;

        let mut res = match &mut sender {
            HttpSender::Http1(s) => s.send_request(req).await?,
            _ => {
                return Err(anyhow::anyhow!("HTTP/2 not implemented yet").into());
            }
        };

        let status = res.status();
        let reason_phrase = status.canonical_reason().unwrap_or("").to_string();
        let headers = res
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
            .collect::<Vec<(String, String)>>();
        let mut body = Vec::new();
        // Stream the body, writing each frame to stdout as it arrives
        while let Some(next) = res.frame().await {
            let frame = next?;
            if let Some(chunk) = frame.data_ref() {
                body.extend_from_slice(chunk);
            }
        }

        self.sender_pool
            .write()
            .await
            .add_connection((*key).clone(), sender)
            .await;

        let response = Response {
            status,
            reason_phrase,
            headers,
            body,
        };

        Ok(response)
    }

    pub async fn fetch_url(
        &self,
        url: &str,
    ) -> Result<Response, Box<dyn std::error::Error + Send + Sync + 'static>> {
        // MAX10回
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

                // Locationさがす
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
                    // Location ヘッダがない場合はそのまま返す
                    return Ok(resp);
                }
            }

            return Ok(resp);
        }
    }
}
