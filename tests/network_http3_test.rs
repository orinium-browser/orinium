use orinium_browser::platform::network::NetworkCore;
use std::error::Error;
use tokio::runtime::Runtime;

#[test]
fn test_http3_connection() -> Result<(), Box<dyn Error>> {
    let rt = Runtime::new()?;

    rt.block_on(async {
        let network_core = NetworkCore::new();

        let url = "https://cloudflare-quic.com/";
        println!("Fetching URL (HTTP/3 candidate): {}", url);
        let response = network_core.fetch_url(url).await?;

        assert!(
            response.status.is_success(),
            "Expected successful status code, got: {}",
            response.status
        );

        assert!(
            !response.body.is_empty(),
            "Response body should not be empty"
        );

        println!("HTTP/3 candidate test completed with status: {}", response.status);

        Ok(())
    })
}

#[test]
fn test_http3_multiple_sites() -> Result<(), Box<dyn Error>> {
    let rt = Runtime::new()?;

    rt.block_on(async {
        let network_core = NetworkCore::new();

        let urls = [
            "https://cloudflare-quic.com/",
            "https://www.cloudflare.com/",
        ];

        for url in &urls {
            println!("Testing HTTP/3-capable site: {}", url);
            let response = network_core.fetch_url(url).await?;

            assert!(
                response.status.is_success() || response.status.is_redirection(),
                "Failed to connect to {} with status: {}",
                url,
                response.status
            );

            assert!(
                !response.body.is_empty(),
                "Response body for {} should not be empty",
                url
            );

            println!(
                "Successfully fetched {} with status: {}",
                url,
                response.status.as_u16()
            );
        }

        Ok(())
    })
}

