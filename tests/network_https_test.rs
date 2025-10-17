use orinium_browser::platform::network::NetworkCore;
use std::error::Error;
use tokio::runtime::Runtime;

#[test]
fn test_https_connection() -> Result<(), Box<dyn Error>> {
    let rt = Runtime::new()?;

    rt.block_on(async {
        let network_core = NetworkCore::new();

        let url = "https://www.google.com";
        log::info!("Fetching URL: {}", url);
        let response = network_core.fetch_url(url).await?;

        assert!(response.status.is_success(),
                "Expected successful status code, got: {}", response.status);

        assert!(!response.body.is_empty(), "Response body should not be empty");

        log::info!("HTTPS test passed successfully!");

        Ok(())
    })
}

#[test]
fn test_http_and_https_comparison() -> Result<(), Box<dyn Error>> {
    let rt = Runtime::new()?;

    rt.block_on(async {
        let network_core = NetworkCore::new();

        let http_url = "http://httpbin.org/get";
        let https_url = "https://httpbin.org/get";

        log::info!("Fetching HTTP URL: {}", http_url);
        let http_response = network_core.fetch_url(http_url).await?;

        log::info!("Fetching HTTPS URL: {}", https_url);
        let https_response = network_core.fetch_url(https_url).await?;

        assert!(http_response.status.is_success(),
                "HTTP request failed with status: {}", http_response.status);
        assert!(https_response.status.is_success(),
                "HTTPS request failed with status: {}", https_response.status);

        assert!(!http_response.body.is_empty(), "HTTP response body should not be empty");
        assert!(!https_response.body.is_empty(), "HTTPS response body should not be empty");

        log::info!("HTTP vs HTTPS comparison test passed successfully!");

        Ok(())
    })
}

#[test]
fn test_https_redirect() -> Result<(), Box<dyn Error>> {
    let rt = Runtime::new()?;

    rt.block_on(async {
        let network_core = NetworkCore::new();

        let url = "https://www.github.com";
        log::info!("Fetching URL with expected redirect: {}", url);

        let response = network_core.fetch_url(url).await?;

        assert!(response.status.is_success() || response.status.is_redirection(),
                "Expected success or redirection status, got: {}", response.status);

        log::info!("HTTPS redirect test completed with status: {} ({})",
                 response.status.as_u16(), response.status.canonical_reason().unwrap_or(""));

        Ok(())
    })
}

#[test]
fn test_secure_site_certificate() -> Result<(), Box<dyn Error>> {
    let rt = Runtime::new()?;

    rt.block_on(async {
        let network_core = NetworkCore::new();

        let urls = [
            "https://www.google.com",
            "https://github.com",
            "https://www.microsoft.com"
        ];

        for url in &urls {
            log::info!("Testing secure connection to: {}", url);
            let response = network_core.fetch_url(url).await?;

            assert!(response.status.is_success() || response.status.is_redirection(),
                    "Failed to connect to {} with status: {}", url, response.status);

            log::info!("Successfully connected to {} with status: {} ({})",
                     url, response.status.as_u16(), response.status.canonical_reason().unwrap_or(""));
        }

        log::info!("Secure site certificate test passed for all sites!");

        Ok(())
    })
}
