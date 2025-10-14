use http_body_util::Empty;
use hyper::body::Bytes;
use hyper::client::conn;
use hyper::{Request, Uri};
use hyper_util::rt::TokioIo;
use std::error::Error;
use tokio::net::TcpStream;

use std::sync::{Arc, Mutex};

use crate::network::ConnectionPool;

pub struct NetworkCore {
    connection_pool: Arc<Mutex<ConnectionPool>>,
}
impl NetworkCore {
    pub fn new() -> Self {
        Self {
            connection_pool: Arc::new(Mutex::new(ConnectionPool::new())),
        }
    }

    async fn fetch_url(&self, url: Uri) -> Result<hyper::Response<hyper::body::Incoming>, Box<dyn Error>> {
        let host = url.host().expect("uri has no host");
        let port = url.port_u16().unwrap_or(80);
        let addr = format!("{}:{}", host, port);

        // TCP接続
        let stream = TcpStream::connect(addr).await?;
        let io = TokioIo::new(stream);

        // HTTP/1.1 ハンドシェイク
        let (mut sender, connection) = conn::http1::handshake(io).await?;

        tokio::spawn(async move {
            if let Err(err) = connection.await {
                eprintln!("Connection failed: {:?}", err);

            }
        });

        let authority = url.authority().unwrap();
        let path = url.path_and_query().map(|p| p.as_str()).unwrap_or("/");

        // リクエスト作成
        let req = Request::builder()
            .method("GET")
            .uri(path)
            .header("Host", authority.as_str())
            .body(Empty::<Bytes>::new())?;

        // リクエスト送信
        let res: hyper::Response<hyper::body::Incoming> = sender.send_request(req).await?;

        println!("Status: {}", res.status());
        println!("Headers: {:#?}\n", res.headers());

        println!("\nDone!");
        Ok(res)
    }
}
