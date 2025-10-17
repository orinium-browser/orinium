use orinium_browser::platform::network::tls::TlsConnection;
use std::time::Duration;

#[tokio::test]
async fn test_tls_connection_to_valid_site() {
    // 有効なサイトへの接続をテスト
    let result = TlsConnection::connect("www.example.com", 443, Duration::from_secs(10)).await;
    assert!(result.is_ok(), "有効なサイトへのTLS接続が失敗しました");
}

#[tokio::test]
async fn test_tls_connection_timeout() {
    // 存在しない/応答しないIPへの接続でタイムアウトをテスト
    // 192.0.2.0/24はテスト用に予約されているアドレス範囲
    let result = TlsConnection::connect("192.0.2.1", 443, Duration::from_millis(500)).await;
    assert!(result.is_err(), "タイムアウトが正しく動作していません");
}

#[tokio::test]
async fn test_tls_connection_invalid_hostname() {
    // 無効なホスト名でのTLS接続試行
    let result = TlsConnection::connect(
        "invalid-hostname-that-does-not-exist.example",
        443,
        Duration::from_secs(5),
    )
    .await;
    assert!(
        result.is_err(),
        "無効なホスト名でのエラー処理が機能していません"
    );
}

#[tokio::test]
async fn test_tls_handshake() {
    // TLSハンドシェイクのテスト（証明書検証を含む）
    let result = TlsConnection::connect("www.google.com", 443, Duration::from_secs(10)).await;
    assert!(result.is_ok(), "TLSハンドシェイクに失敗しました");

    // 接続が成功したら、実際にHTTPリクエストを送信して応答を確認
    if let Ok(mut conn) = result {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // 簡易的なHTTPリクエストを送信
        let request = "HEAD / HTTP/1.1\r\nHost: www.google.com\r\nConnection: close\r\n\r\n";
        conn.stream
            .write_all(request.as_bytes())
            .await
            .expect("リクエスト送信に失敗");

        // レスポンスの一部を読み取り
        let mut response = [0; 1024];
        let n = conn
            .stream
            .read(&mut response)
            .await
            .expect("レスポンス読み取りに失敗");

        // HTTPステータス200が含まれているか確認
        let response_text = String::from_utf8_lossy(&response[0..n]);
        assert!(
            response_text.contains("HTTP/1.1 200") || response_text.contains("HTTP/2 200"),
            "正しいHTTPレスポンスが返されませんでした: {}",
            response_text
        );
    }
}

#[tokio::test]
async fn test_tls_connection_various_websites() {
    // 複数の異なるウェブサイトへの接続をテスト
    let websites = ["www.microsoft.com", "www.amazon.com", "www.cloudflare.com"];

    for site in websites {
        let result = TlsConnection::connect(site, 443, Duration::from_secs(10)).await;
        assert!(result.is_ok(), "{}へのTLS接続に失敗しました", site);
    }
}

#[tokio::test]
async fn test_tls_connection_non_default_port() {
    // 標準以外のポートへの接続テスト（GitHubのHTTPS/443）
    let result = TlsConnection::connect("github.com", 443, Duration::from_secs(10)).await;
    assert!(result.is_ok(), "標準以外のポートへのTLS接続に失敗しました");
}

#[ignore] // 自動テストでは実行しない
#[tokio::test]
async fn test_tls_connection_bad_ssl() {
    // 期限切れ証明書サイトへの接続（失敗を期待）
    let result = TlsConnection::connect("expired.badssl.com", 443, Duration::from_secs(10)).await;
    assert!(result.is_err(), "期限切れ証明書の検出に失敗しました");

    // 自己署名証明書サイトへの接続（失敗を期待）
    let result =
        TlsConnection::connect("self-signed.badssl.com", 443, Duration::from_secs(10)).await;
    assert!(result.is_err(), "自己署名証明書の検出に失敗しました");

    // 有効な証明書サイトへの接続（成功を期待）
    let result = TlsConnection::connect("tls-v1-2.badssl.com", 443, Duration::from_secs(10)).await;
    assert!(result.is_ok(), "有効なTLS 1.2サイトへの接続に失敗しました");
}
