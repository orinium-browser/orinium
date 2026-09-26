//! Manual, network-dependent probes of the full page pipeline.
//!
//! These drive a real [`WebView`] against live sites, driving each fetch from
//! the network core directly so no separate network process is involved. They
//! are `#[ignore]`d and only run when invoked explicitly:
//!
//! ```sh
//! cargo test --lib -- --ignored --nocapture drive_youtube_with_real_fetches
//! RUST_LOG=info YT_RUN_SECS=60 cargo test --lib -- --ignored --nocapture drive_youtube_with_real_fetches
//! ```
//!
//! They live in the browser layer rather than next to the network core because
//! they exercise the browser and the engine, not the network alone.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use url::Url;

use super::{FetchKind, JsPolicy, WebView, WebViewTask};
use crate::engine::js::JsFetchResponse;
use crate::platform::network::{
    AsyncNetworkCore, NetworkRequest, Response, SharedNetState, StatusCode,
};

/// Drive the real [`WebView`] pipeline against https://www.youtube.com/
/// using live fetches, and report how far the page bootstraps.
///
/// Manual probe, not part of the normal suite:
///
/// ```sh
/// cargo test --lib -- --ignored --nocapture drive_youtube_with_real_fetches
/// RUST_LOG=info YT_RUN_SECS=60 cargo test --lib -- --ignored --nocapture drive_youtube_with_real_fetches
/// ```
#[test]
#[ignore = "live network probe against youtube.com; manual run only"]
fn drive_youtube_with_real_fetches() {
    fn fetch_ok(core: &AsyncNetworkCore, url: &Url) -> Option<Response> {
        if url.scheme() == "data" {
            let body =
                crate::browser::core::resource_loader::DataURI::decode(url.as_str()).ok()?;
            return Some(Response {
                url: url.to_string(),
                status: StatusCode(200),
                reason_phrase: "OK".to_string(),
                headers: vec![(
                    "Content-Type".to_string(),
                    "application/octet-stream".to_string(),
                )],
                body,
            });
        }
        if url.scheme() != "http" && url.scheme() != "https" {
            return None;
        }
        core.fetch_request_blocking(&NetworkRequest {
            url: url.to_string(),
            method: "GET".to_string(),
            headers: vec![
                (
                    "User-Agent".to_string(),
                    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
                     (KHTML, like Gecko) Chrome/120.0 Safari/537.36"
                        .to_string(),
                ),
                ("Accept-Language".to_string(), "en-US,en;q=0.9".to_string()),
                ("Accept".to_string(), "*/*".to_string()),
            ],
            body: Vec::new(),
        })
        .ok()
    }

    fn count_tags(
        value: &serde_json::Value,
        counts: &mut BTreeMap<String, usize>,
        text: &mut usize,
    ) {
        match value {
            serde_json::Value::Object(obj) => {
                if let Some(tag) = obj.get("tag")
                    && let Some(tag) = tag.as_str()
                {
                    *counts.entry(tag.to_string()).or_insert(0) += 1;
                }
                if let Some(t) = obj.get("text")
                    && let Some(t) = t.as_str()
                {
                    *text += t.len();
                }
                if let Some(children) = obj.get("children")
                    && let Some(children) = children.as_array()
                {
                    for child in children {
                        count_tags(child, counts, text);
                    }
                }
            }
            serde_json::Value::Array(arr) => {
                for child in arr {
                    count_tags(child, counts, text);
                }
            }
            _ => {}
        }
    }

    let run_secs: u64 = std::env::var("YT_RUN_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(45)
        .min(120);

    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();

    let core = AsyncNetworkCore::new(Arc::new(SharedNetState::new()));
    let base = Url::parse("https://www.youtube.com/").unwrap();

    log::info!(target: "ytprobe", "fetching initial HTML (cap {}s)", run_secs);
    let initial = core
        .fetch_request_blocking(&NetworkRequest {
            url: base.to_string(),
            method: "GET".to_string(),
            headers: vec![
                (
                    "User-Agent".to_string(),
                    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
                     (KHTML, like Gecko) Chrome/120.0 Safari/537.36"
                        .to_string(),
                ),
                ("Accept-Language".to_string(), "en-US,en;q=0.9".to_string()),
                (
                    "Accept".to_string(),
                    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
                        .to_string(),
                ),
            ],
            body: Vec::new(),
        })
        .expect("fetch youtube.com");
    let html = String::from_utf8_lossy(&initial.body).to_string();
    log::info!(target: "ytprobe", "youtube.com HTML: {} bytes", html.len());

    let mut wv = WebView::new(
        crate::engine::layouter::types::ColorScheme::Light,
        JsPolicy::Enabled,
    );
    wv.relayout((1280.0, 720.0));

    let mut fetch_counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let start = Instant::now();
    let mut last_dom_version = 0u64;
    let mut stable_for: u32 = 0;
    let mut layout_seen = false;
    let mut tick_iterations: u64 = 0;

    loop {
        tick_iterations += 1;
        if tick_iterations.is_multiple_of(200) {
            let mut kinds = String::new();
            for (k, v) in &fetch_counts {
                if !kinds.is_empty() {
                    kinds.push(' ');
                }
                kinds.push_str(&format!("{k}={v}"));
            }
            eprintln!(
                "[ytprobe] t={:.1}s ticks={} tasks={{{}}} dom={} layout={}",
                start.elapsed().as_secs_f64(),
                tick_iterations,
                kinds,
                wv.inspect("getVersion", "{}")
                    .ok()
                    .and_then(|v| v["domVersion"].as_u64())
                    .unwrap_or(0),
                wv.layout_and_info().is_some(),
            );
        }
        if start.elapsed().as_secs() >= run_secs {
            break;
        }
        let tasks = wv.tick();
        for task in tasks {
            match task {
                WebViewTask::AskTabHtml => {
                    wv.on_html_fetched(html.clone(), base.clone());
                    *fetch_counts.entry("html").or_insert(0) += 1;
                }
                WebViewTask::DevToolsRequest { id, .. } => {
                    wv.on_devtools_response(id, "{}".to_string());
                    *fetch_counts.entry("devtools").or_insert(0) += 1;
                }
                WebViewTask::Fetch { url, kind } => {
                    let label: &'static str = match &kind {
                        FetchKind::Html => "html",
                        FetchKind::Css => "css",
                        FetchKind::Script { .. } => "script",
                        FetchKind::DynamicScript { .. } => "dyn-script",
                        FetchKind::DynamicCss { .. } => "dyn-css",
                        FetchKind::Image { .. } => "image",
                        FetchKind::Audio { .. } => "audio",
                        FetchKind::JavaScript { .. } => "js-fetch",
                        FetchKind::Iframe { .. } => "iframe",
                        FetchKind::CssImport { .. } => "css-import",
                    };
                    *fetch_counts.entry(label).or_insert(0) += 1;
                    match kind {
                        FetchKind::Html => {
                            if let Some(r) = fetch_ok(&core, &url) {
                                let text = String::from_utf8_lossy(&r.body).to_string();
                                wv.on_html_fetched(text, url);
                            }
                        }
                        FetchKind::Css => {
                            if let Some(r) = fetch_ok(&core, &url) {
                                wv.on_css_fetched_from(
                                    String::from_utf8_lossy(&r.body).to_string(),
                                    &url,
                                );
                            }
                        }
                        FetchKind::Script { index } => match fetch_ok(&core, &url) {
                            Some(r) => wv.on_script_fetched(
                                index,
                                String::from_utf8_lossy(&r.body).to_string(),
                            ),
                            None => wv.on_script_fetch_failed(index),
                        },
                        FetchKind::DynamicScript { node_id } => match fetch_ok(&core, &url) {
                            Some(r) => wv.on_dynamic_script_fetched(
                                node_id,
                                String::from_utf8_lossy(&r.body).to_string(),
                            ),
                            None => wv.on_dynamic_script_fetch_failed(node_id),
                        },
                        FetchKind::DynamicCss { node_id } => {
                            if let Some(r) = fetch_ok(&core, &url) {
                                wv.on_dynamic_style_fetched(
                                    node_id,
                                    String::from_utf8_lossy(&r.body).to_string(),
                                    &url,
                                );
                            }
                        }
                        FetchKind::CssImport { target } => {
                            if let Some(r) = fetch_ok(&core, &url) {
                                wv.on_css_import_fetched(
                                    String::from_utf8_lossy(&r.body).to_string(),
                                    &target,
                                );
                            } else {
                                wv.on_css_import_fetch_failed(&target);
                            }
                        }
                        FetchKind::Image { source } => {
                            if let Some(r) = fetch_ok(&core, &url)
                                && let Err(e) = wv.on_image_fetched(source, &r.body)
                            {
                                log::warn!(target: "ytprobe", "image fetch rejected: {e}");
                            }
                        }
                        FetchKind::Audio { source } => {
                            if let Some(r) = fetch_ok(&core, &url) {
                                wv.on_audio_fetched(source, &r.body);
                            }
                        }
                        FetchKind::JavaScript {
                            request_id,
                            method,
                            headers,
                            body,
                        } => match core.fetch_request_blocking(&NetworkRequest {
                            url: url.to_string(),
                            method,
                            headers,
                            body,
                        }) {
                            Ok(r) => wv.on_js_fetch_succeeded(
                                request_id,
                                JsFetchResponse {
                                    url: r.url,
                                    status: r.status.as_u16(),
                                    status_text: r.reason_phrase,
                                    redirected: false,
                                    body: r.body,
                                    headers: r.headers,
                                },
                            ),
                            Err(e) => {
                                wv.on_js_fetch_failed(request_id, e.to_string());
                            }
                        },
                        FetchKind::Iframe { dom_id } => {
                            wv.on_iframe_fetch_failed(dom_id);
                        }
                    }
                }
                WebViewTask::Navigate { url } => {
                    // Simulate the tab's script-initiated navigation by
                    // re-serving the fetched HTML at the new URL.
                    if let Some(r) = fetch_ok(&core, &url) {
                        let text = String::from_utf8_lossy(&r.body).to_string();
                        wv.on_html_fetched(text, url);
                        *fetch_counts.entry("nav").or_insert(0) += 1;
                    }
                }
            }
        }

        if wv.layout_and_info().is_some() {
            layout_seen = true;
        }
        let dom_version = wv
            .inspect("getVersion", "{}")
            .ok()
            .and_then(|v| v["domVersion"].as_u64())
            .unwrap_or(0);
        if layout_seen && dom_version == last_dom_version {
            stable_for += 1;
        } else {
            stable_for = 0;
            last_dom_version = dom_version;
        }
        if stable_for > 250 {
            log::info!(target: "ytprobe", "DOM stable for 2.5s; stopping");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let elapsed = start.elapsed();
    eprintln!(
        "== YOUTUBE PROBE SUMMARY ({:.1}s, {} ticks) ==",
        elapsed.as_secs_f64(),
        tick_iterations
    );
    eprintln!("fetches: {fetch_counts:?}");
    let version = wv.inspect("getVersion", "{}");
    if let Ok(v) = &version {
        eprintln!("versions: {v}");
    }

    match wv.inspect("getDocument", "{}") {
        Ok(doc) => {
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            let mut text_len = 0usize;
            count_tags(&doc, &mut counts, &mut text_len);
            eprintln!("document text length: {text_len}");
            eprintln!("element tag counts: {counts:?}");
            for marker in [
                "video",
                "ytd-app",
                "ytd-masthead",
                "ytd-searchbox",
                "ytd-player",
                "ytd-thumbnail",
                "ytp-cued-thumbnail-overlay-image",
                "input",
                "iframe",
            ] {
                eprintln!(
                    "marker <{marker}>: {}",
                    counts.get(marker).copied().unwrap_or(0)
                );
            }
        }
        Err(e) => eprintln!("getDocument failed: {e}"),
    }

    if let Some((layout, info)) = wv.layout_and_info() {
        eprintln!(
            "layout present: layout-root children={}, info children={}",
            layout.children.len(),
            info.children.len()
        );
    } else {
        eprintln!("layout: none");
    }
}
