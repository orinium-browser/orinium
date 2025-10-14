use http_body_util::BodyExt;
use http_body_util::Empty;
use hyper::body::Bytes;
use hyper::client::conn;
use hyper::{Request, Uri};
use hyper_util::rt::TokioIo;
use std::error::Error;
//use std::sync::{Arc, Mutex};
use tokio::net::TcpStream;

//use crate::network::{sender_pool::Sender, HostKey, SenderPool};

pub struct Response {
    pub status: hyper::StatusCode,
    pub reason_phrase: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub struct NetworkCore {
    //connection_pool: Arc<Mutex<SenderPool>>,
}

impl NetworkCore {
    pub fn new() -> Self {
        Self {
            //connection_pool: Arc::new(Mutex::new(SenderPool::new())),
        }
    }

    pub async fn fetch_url(&self, url: &str) -> Result<Response, Box<dyn Error>> {
        let url: Uri = url.parse()?;
        let host = url.host().expect("uri has no host");
        let port = url.port_u16().unwrap_or(80);
        let addr = format!("{}:{}", host, port);

        let stream = TcpStream::connect(addr).await?;
        let io = TokioIo::new(stream);

        /*
        let key = Arc::new(HostKey {
            scheme: url.scheme().unwrap_or(&hyper::http::uri::Scheme::HTTP).clone(),
            host: host.to_string(),
            port,
        });
        */

        // handshake
        let (mut sender, connection) = conn::http1::handshake(io).await?;
        tokio::spawn(async move {
            if let Err(err) = connection.await {
                eprintln!("Connection failed: {:?}", err);
            }
        });

        let authority = url.authority().unwrap();
        let path = url.path_and_query().map(|p| p.as_str()).unwrap_or("/");

        let req = Request::builder()
            .method("GET")
            .uri(path)
            .header("Host", authority.as_str())
            .body(Empty::<Bytes>::new())?;

        let mut res = sender.send_request(req).await?;

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
        let response = Response {
            status,
            reason_phrase,
            headers,
            body,
        };

        println!("Status: {}", res.status());
        println!("Headers: {:#?}\n", res.headers());
        println!("Done!");

        Ok(response)
    }
}
