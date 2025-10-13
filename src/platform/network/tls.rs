use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::ClientConfig;
use rustls_native_certs::load_native_certs;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

use crate::platform::network::tcp::TcpConnection;

/// TLS接続を管理するための構造体
#[derive(Debug)]
pub struct TlsConnection {
    /// 内部のTLS暗号化されたストリーム
    pub stream: TlsStream<TcpConnection>,
}

impl TlsConnection {
    /// TLS接続を作成します。
    ///
    /// # 引数
    /// * `host` - 接続先のホスト名（証明書検証に使用）
    /// * `port` - 接続先のポート番号
    /// * `timeout` - 接続タイムアウト時間
    ///
    /// # 戻り値
    /// * 成功した場合は`TlsConnection`のインスタンスを返します
    /// * 証明書検証失敗、タイムアウト、または接続エラーの場合は`anyhow::Error`を返します
    ///
    /// # エラー
    /// * ホスト名が無効な場合
    /// * 証明書検証に失敗した場合
    /// * 接続がタイムアウトした場合
    /// * 下層のTCP接続に失敗した場合
    pub async fn connect(host: &str, port: u16, timeout: Duration) -> anyhow::Result<Self> {
        let tcp_conn = TcpConnection::connect(host, port, timeout).await?;

        let mut roots = rustls::RootCertStore::empty();
        for cert in load_native_certs()? {
            roots.add(cert)?;
        }

        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();

        let connector = TlsConnector::from(Arc::new(config));

        let server_name = ServerName::try_from(host.to_string())
            .map_err(|_| anyhow::anyhow!("Invalid DNS name: {host}"))?;

        let stream =
            tokio::time::timeout(timeout, connector.connect(server_name, tcp_conn)).await??;

        Ok(Self { stream })
    }
}

/// `AsyncRead`トレイトの実装により、TlsConnectionから非同期的にデータを読み取る機能を提供します。
/// この実装はTLS暗号化されたデータを透過的に復号化して読み取ります。
impl AsyncRead for TlsConnection {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().stream).poll_read(cx, buf)
    }
}

/// `AsyncWrite`トレイトの実装により、TlsConnectionに非同期的にデータを書き込む機能を提供します。
/// この実装はデータを透過的に暗号化してTLSストリームに書き込みます。
impl AsyncWrite for TlsConnection {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.get_mut().stream).poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}
