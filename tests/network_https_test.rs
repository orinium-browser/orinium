use orinium_browser::platform::network::NetworkCore;
use std::error::Error;

#[ignore]
#[test]
fn test_https_connection() -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    let network_core = NetworkCore::new();

    let url = "https://www.google.com";
    println!("Fetching URL: {}", url);

    let response = network_core.fetch_blocking(url)?;

    assert!(
        response.status.is_success(),
        "Expected successful status code, got: {}",
        response.status
    );

    assert!(
        !response.body.is_empty(),
        "Response body should not be empty"
    );

    println!("HTTPS test passed successfully!");
    Ok(())
}

#[ignore]
#[test]
fn test_http_and_https_comparison() -> Result<(), Box<dyn Error>> {
    let network_core = NetworkCore::new();

    let http_url = "http://httpbin.org/get";
    let https_url = "https://httpbin.org/get";

    println!("Fetching HTTP URL: {}", http_url);
    let http_response = network_core.fetch_blocking(http_url)?;

    println!("Fetching HTTPS URL: {}", https_url);
    let https_response = network_core.fetch_blocking(https_url)?;

    assert!(http_response.status.is_success());
    assert!(https_response.status.is_success());

    assert!(!http_response.body.is_empty());
    assert!(!https_response.body.is_empty());

    println!("HTTP vs HTTPS comparison test passed successfully!");
    Ok(())
}

#[ignore]
#[test]
fn test_https_redirect() -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    let network_core = NetworkCore::new();

    let url = "https://www.github.com";
    println!("Fetching URL with expected redirect: {}", url);

    let response = network_core.fetch_blocking(url)?;

    assert!(
        response.status.is_success() || response.status.is_redirection(),
        "Expected success or redirection status, got: {}",
        response.status
    );

    println!(
        "HTTPS redirect test completed with status: {} ({})",
        response.status.as_u16(),
        response.status.canonical_reason().unwrap_or("")
    );

    Ok(())
}

#[ignore]
#[test]
fn test_secure_site_certificate() -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    let network_core = NetworkCore::new();

    let urls = [
        "https://www.google.com",
        "https://github.com",
        "https://www.microsoft.com",
    ];

    for url in &urls {
        println!("Testing secure connection to: {}", url);

        let response = network_core.fetch_blocking(url)?;

        assert!(
            response.status.is_success() || response.status.is_redirection(),
            "Failed to connect to {} with status: {}",
            url,
            response.status
        );

        println!(
            "Successfully connected to {} with status: {} ({})",
            url,
            response.status.as_u16(),
            response.status.canonical_reason().unwrap_or("")
        );
    }

    println!("Secure site certificate test passed for all sites!");
    Ok(())
}
