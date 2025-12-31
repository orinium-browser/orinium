use crate::platform::network::{NetworkCore, Response};

/// パスからMIMEタイプを推測する
fn guess_mime_from_path(path: &str) -> &'static str {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// URLをフェッチしてResponseを返す。
/// "resource:///" スキームの場合は組み込みリソースを返す
pub async fn fetch_url(net: &NetworkCore, url: &str) -> anyhow::Result<Response> {
    if url.starts_with("resource:///") {
        // resource:///path -> ./resource/path
        let rel = &url[12..];
        let bytes = crate::platform::io::load_resource(rel).await?;
        let resp = Response {
            status: hyper::StatusCode::OK,
            reason_phrase: "OK".to_string(),
            headers: vec![("content-type".to_string(), guess_mime_from_path(rel).to_string())],
            body: bytes,
        };
        Ok(resp)
    } else {
        match net.fetch_url(url).await {
            Ok(r) => Ok(r),
            Err(e) => Err(anyhow::anyhow!(e.to_string())),
        }
    }
}
