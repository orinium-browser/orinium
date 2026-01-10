use crate::network::NetworkCore;
use anyhow::{Result, anyhow};
use hyper::StatusCode;
use std::sync::Arc;

/// Unified resource loader for `resource:///` and HTTP/HTTPS URLs
pub struct BrowserResourceLoader {
    network: Option<Arc<NetworkCore>>,
}

impl BrowserResourceLoader {
    /// Create a new loader with optional NetworkCore
    pub fn new(network: Option<Arc<NetworkCore>>) -> Self {
        Self { network }
    }

    /// Fetch a resource by URL
    /// - `resource:///` URLs are loaded via platform IO
    /// - HTTP/HTTPS URLs are fetched via NetworkCore
    pub async fn fetch(&self, url: &str) -> Result<BrowserResponse> {
        if url.starts_with("resource:///") {
            // Load resource from platform IO
            let data = ResourceURI::load(url).await?;
            Ok(BrowserResponse {
                status: StatusCode::OK,
                body: data,
                headers: vec![],
            })
        } else {
            // Ensure network is available
            let network = self
                .network
                .as_ref()
                .ok_or_else(|| anyhow!("NetworkCore not available for URL: {}", url))?;

            // Fetch via network
            let resp = network.fetch_url(url).await.map_err(|e| anyhow!(e))?;
            Ok(BrowserResponse {
                status: resp.status,
                body: resp.body,
                headers: resp.headers,
            })
        }
    }
}

/// Unified response type for both network and resource URLs
pub struct BrowserResponse {
    pub status: StatusCode,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

/// Resource Manager for `resource:///` scheme
pub struct ResourceURI;

impl ResourceURI {
    /// Load a resource by its URL.
    /// Only supports `resource:///` scheme for now.
    pub async fn load(url: &str) -> Result<Vec<u8>, anyhow::Error> {
        use crate::platform::io;
        if let Some(path) = url.strip_prefix("resource:///") {
            io::load_resource(path).await
        } else {
            Err(anyhow!("Unsupported scheme: {}", url))
        }
    }
}
