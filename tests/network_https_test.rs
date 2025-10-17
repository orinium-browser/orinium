use orinium_browser::platform::network::network_core::NetworkCore;

#[tokio::test]
async fn test_https_connection() {
    let network_core = NetworkCore::new().expect("Failed to create NetworkCore");

    let response = network_core
        .fetch("https://www.example.com")
        .await
        .expect("Failed to fetch HTTPS URL");

    assert_eq!(response.status_code, 200, "Expected status code 200");
    assert!(!response.body.is_empty(), "Response body should not be empty");

    log::info!("✓ HTTPS接続に成功しました");
    log::info!("  ステータスコード: {}", response.status_code);
    log::info!("  レスポンスサイズ: {} bytes", response.body.len());
}

#[tokio::test]
async fn test_http_and_https() {
    let network_core = NetworkCore::new().expect("Failed to create NetworkCore");

    let http_response = network_core
        .fetch("http://www.example.com")
        .await
        .expect("Failed to fetch HTTP URL");

    assert_eq!(http_response.status_code, 200);
    log::info!("✓ HTTP接続に成功しました");

    let https_response = network_core
        .fetch("https://www.example.com")
        .await
        .expect("Failed to fetch HTTPS URL");

    assert_eq!(https_response.status_code, 200);
    log::info!("✓ HTTPS接続に成功しました");
}

#[tokio::test]
async fn test_https_with_redirect() {
    let network_core = NetworkCore::new().expect("Failed to create NetworkCore");

    // HTTPSでリダイレクトがあるサイト
    let response = network_core
        .fetch("https://github.com")
        .await
        .expect("Failed to fetch HTTPS URL with redirect");

    assert!(
        response.status_code == 200 || response.status_code == 301 || response.status_code == 302,
        "Expected successful response or redirect"
    );

    log::info!("✓ リダイレクトを含むHTTPS接続に成功しました");
    log::info!("  ステータスコード: {}", response.status_code);
}

