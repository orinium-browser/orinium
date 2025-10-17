use anyhow::Result;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::io::{self, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::RwLock;
use url::Url;

use crate::platform::network::{
    cache::Cache,
    config::NetworkConfig,
    connection_pool::{Connection, ConnectionPool, HostKey},
    cookie_store::CookieStore,
    tcp::TcpConnection,
    tls::TlsConnection,
};

/// HTTPレスポンスを表す構造体
///
/// サーバーからのHTTPレスポンスの詳細情報を格納します。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Response {
    /// HTTPバージョン (例: "HTTP/1.1")
    pub http_version: String,
    /// HTTPステータスコード (例: 200, 404)
    pub status_code: u16,
    /// ステータスコードに対応する説明文 (例: "OK", "Not Found")
    pub reason_phrase: String,
    /// HTTPヘッダーのキーと値のペアのリスト
    pub headers: Vec<(String, String)>,
    /// レスポンスボディのバイナリデータ
    pub body: Vec<u8>,
}

/// キャッシュに保存されるレスポンスエントリを表す構造体
#[derive(Debug)]
#[allow(dead_code)]
struct CacheEntry {
    /// キャッシュされたレスポンスデータ
    response: Response,
    /// キャッシュが作成された時刻
    cached_at: SystemTime,
    /// キャッシュの有効期限（Noneの場合は期限なし）
    expires_at: Option<SystemTime>,
}

/// ネットワーク通信の中核機能を提供する構造体
///
/// HTTP/HTTPS通信、キャッシュ管理、Cookie管理、接続プールなどの機能を統合します。
#[derive(Debug)]
pub struct NetworkCore {
    /// ネットワーク設定情報
    pub config: Arc<RwLock<NetworkConfig>>,
    /// コネクションプール（接続を再利用するため）
    pub connection_pool: ConnectionPool,
    /// Cookieストア（サイト間でCookieを管理）
    pub cookie_store: CookieStore,
    /// レスポンスキャッシュ
    pub cache: Cache,
}

#[allow(dead_code)]
impl NetworkCore {
    /// 新しいNetworkCoreインスタンスを作成します
    ///
    /// デフォルト設定でネットワークコアを初期化します。
    ///
    /// # 戻り値
    /// * 成功した場合は`NetworkCore`のインスタンスを返します
    /// * 初期化に失敗した場合は`anyhow::Error`を返します
    pub fn new() -> Result<Self> {
        let config = NetworkConfig::default();
        Ok(Self {
            config: Arc::new(RwLock::new(config)),
            connection_pool: ConnectionPool::new(),
            cookie_store: CookieStore::new(),
            cache: Cache::new(),
        })
    }

    /// 汎用 HTTP リクエスト関数
    ///
    /// 指定されたURLにHTTPリクエストを送信し、レスポンスを取得します。
    /// この関数はGET、POST、その他のHTTPメソッドでの通信を処理します。
    ///
    /// # 引数
    /// * `method` - HTTPメソッド（例: "GET", "POST"）
    /// * `url` - 接続先URL
    /// * `extra_headers` - 追加のHTTPヘッダー
    /// * `body` - リクエストボディ（省略可能）
    /// * `use_cache` - キャッシュを使用するかどうか
    ///
    /// # 戻り値
    /// * 成功した場合は`Response`を返します
    /// * 接続エラーなどの場合は`anyhow::Error`を返します
    async fn send_request(
        &self,
        method: &str,
        url: &Url,
        extra_headers: Vec<(String, String)>,
        body: Option<Vec<u8>>,
        use_cache: bool,
    ) -> Result<Response> {
        let host = url
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid host"))?
            .to_string();

        let default_port = if url.scheme() == "https" { 443 } else { 80 };
        let port = url.port().unwrap_or(default_port);

        let key = HostKey {
            scheme: url.scheme().to_string(),
            host: host.clone(),
            port,
        };

        // GET キャッシュチェック
        if use_cache {
            if let Some(cached) = self.cache.get(url).await {
                return Ok(Response {
                    http_version: "HTTP/1.1".to_string(),
                    status_code: 200,
                    reason_phrase: "OK (cache)".to_string(),
                    headers: cached.headers,
                    body: cached.body,
                });
            }
        }

        // Connection取得
        let mut conn = match self.connection_pool.get_connection(&key).await {
            Some(c) => c,
            None => {
                let cfg = self.config.read().await;
                if url.scheme() == "https" {
                    Connection::Tls(TlsConnection::connect(&host, port, cfg.connect_timeout).await?)
                } else {
                    Connection::Tcp(TcpConnection::connect(&host, port, cfg.connect_timeout).await?)
                }
            }
        };

        // Cookie
        let cookie_header = self.cookie_store.get_cookie_header(url).await;

        // パスとクエリパラメータを含める
        let path_and_query = if let Some(query) = url.query() {
            format!("{}?{}", url.path(), query)
        } else {
            url.path().to_string()
        };

        // ヘッダ作成
        let mut request = format!(
            "{} {} HTTP/1.1\r\nHost: {}\r\nConnection: keep-alive\r\nUser-Agent: {}\r\n",
            method,
            path_and_query,
            host,
            self.config.read().await.user_agent
        );

        for (k, v) in extra_headers {
            request.push_str(&format!("{k}: {v}\r\n"));
        }

        if let Some(cookie) = cookie_header {
            request.push_str(&format!("Cookie: {cookie}\r\n"));
        }

        if let Some(ref b) = body {
            request.push_str(&format!("Content-Length: {}\r\n", b.len()));
        }
        request.push_str("\r\n"); // ヘッダ終端

        // 送信 & レスポンス受信
        let (headers, body) = match &mut conn {
            Connection::Tcp(c) => {
                c.stream.write_all(request.as_bytes()).await?;
                if let Some(body_bytes) = body.clone() {
                    c.stream.write_all(&body_bytes).await?;
                }
                let (body_start, headers) = Self::read_headers(&mut c.stream).await?;
                let content_length = headers
                    .iter()
                    .find(|(k, _)| k.to_lowercase() == "content-length")
                    .and_then(|(_, v)| v.parse::<usize>().ok())
                    .unwrap_or(0);
                let body = Self::read_body(&mut c.stream, body_start, content_length).await?;
                (headers, body)
            }
            Connection::Tls(c) => {
                c.stream.write_all(request.as_bytes()).await?;
                if let Some(body_bytes) = body.clone() {
                    c.stream.write_all(&body_bytes).await?;
                }
                let (body_start, headers) = Self::read_headers(&mut c.stream).await?;
                let content_length = headers
                    .iter()
                    .find(|(k, _)| k.to_lowercase() == "content-length")
                    .and_then(|(_, v)| v.parse::<usize>().ok())
                    .unwrap_or(0);
                let body = Self::read_body(&mut c.stream, body_start, content_length).await?;
                (headers, body)
            }
        };

        // ステータス行パース
        let status_line = headers
            .iter()
            .find(|(k, _)| k == "Status-Line")
            .map(|(_, v)| v.as_str())
            .unwrap_or("");
        let http_version = status_line
            .split_whitespace()
            .next()
            .unwrap_or("HTTP/1.1")
            .to_string();
        let status_code = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(200);
        let reason_phrase = status_line.splitn(3, ' ').nth(2).unwrap_or("").to_string();

        // Cookie 保存
        let set_cookie_headers = headers
            .iter()
            .filter(|(k, _)| k.to_lowercase() == "set-cookie")
            .map(|(_, v)| v.clone())
            .collect::<Vec<_>>();
        self.cookie_store
            .set_cookies(url, &set_cookie_headers)
            .await;

        // Cache 保存（GETのみ）
        if use_cache {
            self.cache.set(url, body.clone(), headers.clone()).await;
        }

        // Connection プールに戻す
        self.connection_pool.add_connection(key, conn).await;

        Ok(Response {
            http_version,
            status_code,
            reason_phrase,
            headers,
            body,
        })
    }

    /// GET要求を送信し、結果を取得します（キャッシュを使用）
    ///
    /// 指定されたURLにGETリクエストを送信し、レスポンスを返します。
    /// キャッシュが有効な場合は、キャッシュからレスポンスが返される場合があります。
    ///
    /// # 引数
    /// * `url` - 取得するURL（文字列）
    ///
    /// # 戻り値
    /// * 成功した場合は`Response`を返します
    /// * URL解析エラーや接続エラーなどの場合は`anyhow::Error`を返します
    pub async fn fetch(&self, url: &str) -> Result<Response> {
        let url = Url::parse(url)?;
        self.send_request("GET", &url, vec![], None, true).await
    }

    /// POSTリクエストを送信します（キャッシュなし）
    ///
    /// 指定されたURLにPOSTリクエストを送信し、レスポンスを返します。
    /// POSTリクエストはキャッシュを使用しません。
    ///
    /// # 引数
    /// * `url` - 送信先URL（文字列）
    /// * `body` - POSTリクエストのボディデータ
    /// * `content_type` - コンテンツタイプ（例: "application/json"）
    ///
    /// # 戻り値
    /// * 成功した場合は`Response`を返します
    /// * URL解析エラーや接続エラーなどの場合は`anyhow::Error`を返します
    pub async fn post(&self, url: &str, body: Vec<u8>, content_type: &str) -> Result<Response> {
        let url = Url::parse(url)?;
        let headers = vec![("Content-Type".to_string(), content_type.to_string())];
        self.send_request("POST", &url, headers, Some(body), false)
            .await
    }

    /// HTTPヘッダーを読み取ります
    ///
    /// 指定されたストリームからHTTPレスポンスヘッダーを読み取ります。
    /// HTTPヘッダーの終端（空行）まで読み取り、ヘッダー情報とボディの先頭部分を返します。
    ///
    /// # 引数
    /// * `stream` - 読み取り元のストリーム（AsyncRead + Unpin）
    ///
    /// # 戻り値
    /// * 成功した場合は`(Vec<u8>, Vec<(String, String)>)`を返します（ボディの先頭部分とヘッダーのペア）
    /// * 読み取りエラーの場合は`io::Error`を返します
    async fn read_headers<R>(stream: &mut R) -> io::Result<(Vec<u8>, Vec<(String, String)>)>
    where
        R: AsyncRead + Unpin,
    {
        let mut reader = BufReader::new(stream);
        let mut headers = Vec::new();
        let mut line = String::new();

        loop {
            line.clear();
            let n = reader.read_line(&mut line).await?;
            if n == 0 || line == "\r\n" {
                break;
            }
            if let Some((k, v)) = line.split_once(':') {
                headers.push((k.trim().to_string(), v.trim().to_string()));
            } else if line.starts_with("HTTP/") {
                headers.push(("Status-Line".to_string(), line.trim().to_string()));
            }
        }

        let body_start = reader.buffer().to_vec();
        Ok((body_start, headers))
    }

    /// HTTPレスポンスボディを読み取ります
    ///
    /// 指定されたストリームからHTTPレスポンスボディを読み取ります。
    /// Content-Lengthヘッダーに基づいて、適切な量のデータを読み取ります。
    ///
    /// # 引数
    /// * `stream` - 読み取り元のストリーム（AsyncRead + Unpin）
    /// * `body_start` - すでに読み取られたボディの先頭部分
    /// * `content_length` - 読み取るべきボディの合計サイズ
    ///
    /// # 戻り値
    /// * 成功した場合は`Vec<u8>`としてボディデータを返します
    /// * 読み取りエラーの場合は`anyhow::Error`を返します
    async fn read_body<R>(
        stream: &mut R,
        body_start: Vec<u8>,
        content_length: usize,
    ) -> anyhow::Result<Vec<u8>>
    where
        R: AsyncRead + Unpin,
    {
        let mut body = Vec::with_capacity(content_length);
        body.extend_from_slice(&body_start);
        while body.len() < content_length {
            let mut tmp = [0u8; 1024];
            let n = stream.read(&mut tmp).await?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }
        Ok(body)
    }
}
