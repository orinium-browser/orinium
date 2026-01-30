use std::time::Duration;

/// ネットワーク層全体の設定
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    /// User-Agent文字列
    pub user_agent: String,

    /// タイムアウト設定
    pub connect_timeout: Duration,
    pub read_timeout: Duration,

    /// キャッシュを有効化するか
    pub enable_cache: bool,

    /// Cookie管理を有効化するか
    pub enable_cookies: bool,

    /// TLS証明書の検証を有効化するか
    pub verify_tls: bool,

    /// プロキシ設定
    pub proxies: Vec<ProxyConfig>,

    /// 最大同時接続数
    pub max_connections: usize,

    /// リダイレクトを自動フォローするか
    pub follow_redirects: bool,

    /// WebSocketを有効化するか
    pub enable_websocket: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum ProxyType {
    Http,
    Https,
    Socks5,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    pub proxy_type: String,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct ProxySettings {
    pub proxies: Vec<ProxyConfig>, // 複数種類を保持できる
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            user_agent: String::from(
                "OrinionBrowser/0.1 (+https://github.com/Orinas-github/Orinium-browser)",
            ),
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(30),
            enable_cache: true,
            enable_cookies: true,
            verify_tls: true,
            proxies: vec![],
            max_connections: 100,
            follow_redirects: true,
            enable_websocket: true,
        }
    }
}
