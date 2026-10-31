//! Manual, network-dependent probes of the full page pipeline.
//!
//! These drive a real [`WebView`] against live sites, driving each fetch from
//! the network core directly so no separate network process is involved. They
//! are `#[ignore]`d and only run when invoked explicitly:
//!
//! ```sh
//! cargo test --lib -- --ignored --nocapture drive_youtube_with_real_fetches
//! cargo test --lib -- --ignored --nocapture drive_youtube_watch_with_real_fetches
//! RUST_LOG=info YT_RUN_SECS=60 cargo test --lib -- --ignored --nocapture drive_youtube_with_real_fetches
//! YT_VIDEO_ID=dQw4w9WgXcQ cargo test --lib -- --ignored --nocapture drive_youtube_watch_with_real_fetches
//! ```
//!
//! Both probes collect, in a single pass:
//!
//! - every external classic script with its URL, HTTP status, size and eval time
//! - JavaScript parse/runtime errors (message, pc, function, stack) from the log
//! - the `missing-api` report of absent Web Platform APIs
//! - page state after boot: `ytcfg`/`yt`/`Polymer`/`customElements.get(...)`,
//!   the DOM commit count, and whether `ytd-app`, `ytd-searchbox`,
//!   `ytd-player` and `<video>` exist
//!
//! The instrumentation is probe-only: an ES5 collector script is injected
//! ahead of the page's classic scripts, and per-script marks bracket every
//! external script on the JS thread so eval time comes from the page clock.
//! Inline scripts that run between two external scripts are counted in the
//! earlier row; a failed script is listed without an eval time.
//!
//! They live in the browser layer rather than next to the network core because
//! they exercise the browser and the engine, not the network alone.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use log::{Level, LevelFilter, Log, Metadata, Record};
use url::Url;

use super::{ClassicScript, FetchKind, JsPolicy, WebView, WebViewTask};
use crate::engine::js::web_apis::missing_api_detector;
use crate::engine::js::{JsFetchResponse, JsTask};
use crate::platform::network::{
    AsyncNetworkCore, NetworkRequest, Response, SharedNetState, StatusCode,
};

/// User agent sent with every probe request.
const PROBE_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36";

/// A log line kept for the probe report.
struct CapturedLog {
    at: Instant,
    message: String,
}

/// Log lines captured so far. JS parse/runtime failures only reach the log
/// (`JsRuntime::run_script`), so the probe mirrors them here to attribute them
/// to the script that produced them.
static CAPTURED_LOGS: Mutex<Vec<CapturedLog>> = Mutex::new(Vec::new());

/// Upper bound on captured log lines, so a chatty run cannot grow unbounded.
const LOG_CAPACITY: usize = 20_000;

/// The DOM-stability shortcut may end a run only after this much time: a large
/// script evaluates without touching the DOM, so an early stop would hide it.
const MIN_RUN_SECS: u64 = 30;

/// Forwards every record to `env_logger` (so `RUST_LOG` still works) while
/// keeping the JS-error records for the summary.
struct CaptureLogger {
    inner: env_logger::Logger,
}

impl Log for CaptureLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        let message = match record.level() {
            Level::Error | Level::Warn => Some(record.args().to_string()),
            // JS failures are reported at info level. Only keep the ones that
            // carry a diagnostic, so the probe does not mirror the whole log.
            Level::Info => {
                let message = record.args().to_string();
                (message.contains("JS error") || message.contains("missing-api")).then_some(message)
            }
            _ => None,
        };
        if let Some(message) = message
            && let Ok(mut logs) = CAPTURED_LOGS.lock()
            && logs.len() < LOG_CAPACITY
        {
            logs.push(CapturedLog {
                at: Instant::now(),
                message,
            });
        }
        self.inner.log(record);
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

/// Installs the capturing logger once per process.
///
/// Other tests in this binary may initialize logging first; in that case the
/// probe still runs, it simply cannot report JS errors from the log.
fn install_log_capture() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let inner =
            env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
                .build();
        // Keep debug/trace visible to `env_logger` when `RUST_LOG` asks for
        // them; the capture filter above decides what the probe keeps.
        let max_level = match std::env::var("RUST_LOG").ok().as_deref() {
            Some(spec) if spec.contains("trace") => LevelFilter::Trace,
            Some(spec) if spec.contains("debug") => LevelFilter::Debug,
            _ => LevelFilter::Info,
        };
        log::set_max_level(max_level);
        if log::set_boxed_logger(Box::new(CaptureLogger { inner })).is_err() {
            eprintln!("[probe] logging already initialized; JS error capture disabled");
        }
    });
}

/// Snapshot of the captured log lines.
fn captured_logs() -> Vec<CapturedLog> {
    CAPTURED_LOGS
        .lock()
        .map(|logs| {
            logs.iter()
                .map(|log| CapturedLog {
                    at: log.at,
                    message: log.message.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// ES5 collector injected before the page's own scripts.
///
/// It mirrors `console.warn`/`console.error` (including the engine's own
/// `missing-api` report) into an array, exposes per-script timing marks, and
/// writes a JSON report into the live DOM so the probe can read it back through
/// the inspector without a JS eval channel.
const PROBE_COLLECTOR: &str = r#"(function () {
  if (window.__probe) { return; }
  var errors = [];
  var levels = ["log", "warn", "error", "info", "debug"];
  for (var i = 0; i < levels.length; i++) {
    (function (level) {
      var orig = null;
      try { orig = console[level]; } catch (e) { orig = null; }
      console[level] = function () {
        var parts = [];
        for (var j = 0; j < arguments.length; j++) {
          var a = arguments[j];
          try {
            if (typeof a === "string") { parts.push(a); }
            else if (a && a.message !== undefined) { parts.push(String(a.message)); }
            else { parts.push(String(a)); }
          } catch (e2) { parts.push("<unprintable>"); }
        }
        var text = parts.join(" ");
        if (level === "warn" || level === "error") { errors.push("[" + level + "] " + text); }
        if (orig) { try { orig.apply(console, arguments); } catch (e3) {} }
      };
    })(levels[i]);
  }
  window.__probe = { errors: errors, times: [] };
  errors.push("[probe] collector installed");
  try { console.warn("[probe] console capture active"); } catch (e0) {}
  window.__probeNow = function () {
    try {
      if (typeof performance !== "undefined" && performance.now) { return performance.now(); }
    } catch (e) {}
    try { return Date.now(); } catch (e) { return 0; }
  };
  window.__probeMark = function (label) {
    window.__probe.times.push([label, window.__probeNow()]);
  };
  window.onerror = function (msg) {
    errors.push("[onerror] " + String(msg));
    return false;
  };
  window.onunhandledrejection = function (e) {
    errors.push("[unhandledrejection] " + String(e && e.reason || e));
    return false;
  };
  window.__probeReport = function () {
    var out = {};
    function probe(name, fn) {
      try { out[name] = String(fn()); } catch (e) { out[name] = "ERR:" + String(e); }
    }
    probe("location", function () { return window.location.href; });
    probe("readyState", function () { return document.readyState; });
    probe("elementCount", function () { return document.querySelectorAll("*").length; });
    probe("scriptsLength", function () { return document.scripts ? document.scripts.length : "undefined"; });
    probe("currentScript", function () { return document.currentScript ? "present" : "undefined"; });
    probe("getElementById:base-js", function () { var n = document.getElementById("base-js"); return n ? (n.tagName + " " + (n.src || "")) : "null"; });
    probe("baseJs._F_jsUrl", function () { return typeof window._F_jsUrl; });
    probe("ytcfg", function () { return typeof ytcfg; });
    probe("ytcfg.get", function () { return typeof ytcfg !== "undefined" && ytcfg ? typeof ytcfg.get : "n/a"; });
    probe("yt", function () { return typeof yt; });
    probe("yt.player", function () { return typeof yt !== "undefined" && yt ? typeof yt.player : "n/a"; });
    probe("Polymer", function () { return typeof Polymer; });
    probe("customElements", function () { return typeof customElements; });
    probe("customElements.get", function () { return typeof customElements !== "undefined" && customElements ? typeof customElements.get : "n/a"; });
    var tags = ["ytd-app", "ytd-masthead", "ytd-searchbox", "ytd-player", "ytd-watch-flexy", "video"];
    for (var t = 0; t < tags.length; t++) {
      (function (tag) {
        probe("defined:" + tag, function () {
          return customElements && customElements.get ? !!(customElements.get(tag)) : "no-registry";
        });
        probe("count:" + tag, function () { return document.querySelectorAll(tag).length; });
      })(tags[t]);
    }
    probe("HTMLMediaElement", function () { return typeof HTMLMediaElement; });
    probe("MediaSource", function () { return typeof MediaSource; });
    probe("SourceBuffer", function () { return typeof SourceBuffer; });
    probe("ShadowRoot", function () { return typeof ShadowRoot; });
    probe("requestAnimationFrame", function () { return typeof requestAnimationFrame; });
    probe("MutationObserver", function () { return typeof MutationObserver; });
    probe("IntersectionObserver", function () { return typeof IntersectionObserver; });
    probe("ResizeObserver", function () { return typeof ResizeObserver; });
    probe("fetch", function () { return typeof fetch; });
    probe("XMLHttpRequest", function () { return typeof XMLHttpRequest; });
    probe("ytInitialData", function () { return typeof ytInitialData; });
    probe("ytInitialPlayerResponse", function () { return typeof ytInitialPlayerResponse; });
    probe("playerResponse.streamingData", function () {
      return typeof ytInitialPlayerResponse !== "undefined" && ytInitialPlayerResponse
        ? typeof ytInitialPlayerResponse.streamingData
        : "n/a";
    });
    out.console = errors.slice(0, 120);
    out.times = window.__probe.times;
    var node = document.getElementById("__probe_report");
    if (!node) {
      node = document.createElement("pre");
      node.setAttribute("id", "__probe_report");
      document.documentElement.appendChild(node);
    }
    node.textContent = JSON.stringify(out);
  };
})();"#;

/// One external classic script executed by the probe.
struct ScriptRow {
    index: usize,
    url: String,
    status: Option<u16>,
    bytes: usize,
    /// First 60 chars of the body, matching the `source` the engine logs.
    body_prefix: String,
    /// Wall time on the JS thread, from the mark before to the mark after.
    eval_ms: Option<f64>,
    errors: Vec<String>,
}

/// One live page driven by the probe.
struct Probe {
    core: AsyncNetworkCore,
    wv: WebView,
    initial_html: String,
    initial_url: Url,
    fetch_counts: BTreeMap<&'static str, usize>,
    scripts: Vec<ScriptRow>,
    dom_commits: usize,
    last_dom_ptr: Option<usize>,
}

impl Probe {
    fn new(core: AsyncNetworkCore, initial_html: String, initial_url: Url) -> Self {
        let mut wv = WebView::new(
            crate::engine::layouter::types::ColorScheme::Light,
            JsPolicy::Enabled,
        );
        wv.relayout((1280.0, 720.0));
        Self {
            core,
            wv,
            initial_html,
            initial_url,
            fetch_counts: BTreeMap::new(),
            scripts: Vec::new(),
            dom_commits: 0,
            last_dom_ptr: None,
        }
    }

    /// Injects the collector and the missing-API prologue ahead of the page's
    /// classic scripts. Called after every document load, because navigation
    /// recreates the JS runtime.
    fn install_instrumentation(&mut self) {
        send_probe_script(&mut self.wv, PROBE_COLLECTOR);
        send_probe_script(&mut self.wv, &missing_api_detector::prologue());
        // Install the collector first so it captures the missing-api report.
        // The prologue runs after the collector so any warn emitted by the
        // prologue is mirrored into __probe.errors.
        self.last_dom_ptr = None;
        self.note_dom_commit();
    }

    fn note_dom_commit(&mut self) {
        let Some(info) = self.wv.docment_info.as_ref() else {
            return;
        };
        let ptr = Rc::as_ptr(&info.dom) as usize;
        if self.last_dom_ptr != Some(ptr) {
            self.last_dom_ptr = Some(ptr);
            self.dom_commits += 1;
        }
    }

    fn bump(&mut self, label: &'static str) {
        *self.fetch_counts.entry(label).or_insert(0) += 1;
    }

    fn answer(&mut self, task: WebViewTask) {
        match task {
            WebViewTask::AskTabHtml => {
                self.bump("html");
                self.wv
                    .on_html_fetched(self.initial_html.clone(), self.initial_url.clone());
                self.install_instrumentation();
            }
            WebViewTask::DevToolsRequest { id, .. } => {
                self.bump("devtools");
                self.wv.on_devtools_response(id, "{}".to_string());
            }
            WebViewTask::Fetch { url, kind } => self.answer_fetch(url, kind),
            WebViewTask::Navigate { url } => {
                if let Some(r) = fetch_ok(&self.core, &url) {
                    self.bump("nav");
                    let text = String::from_utf8_lossy(&r.body).to_string();
                    self.wv.on_html_fetched(text, url);
                    self.install_instrumentation();
                }
            }
        }
    }

    fn answer_fetch(&mut self, url: Url, kind: FetchKind) {
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
        self.bump(label);

        match kind {
            FetchKind::Html => {
                if let Some(r) = fetch_ok(&self.core, &url) {
                    let text = String::from_utf8_lossy(&r.body).to_string();
                    self.wv.on_html_fetched(text, url);
                    self.install_instrumentation();
                }
            }
            FetchKind::Css => {
                if let Some(r) = fetch_ok(&self.core, &url) {
                    self.wv
                        .on_css_fetched_from(String::from_utf8_lossy(&r.body).to_string(), &url);
                }
            }
            FetchKind::Script { index } => match fetch_ok(&self.core, &url) {
                Some(r) => {
                    let body = String::from_utf8_lossy(&r.body).to_string();
                    self.scripts.push(ScriptRow {
                        index,
                        url: url.to_string(),
                        status: Some(r.status.as_u16()),
                        bytes: r.body.len(),
                        body_prefix: body.chars().take(60).collect(),
                        eval_ms: None,
                        errors: Vec::new(),
                    });
                    // Bracket the script so the page clock measures its eval.
                    // The closing mark is queued right behind the script body,
                    // so it runs as soon as the script returns, even for the
                    // last script of the page.
                    send_probe_script(
                        &mut self.wv,
                        &format!("window.__probeMark && window.__probeMark({index})"),
                    );
                    self.wv.on_script_fetched(index, body);
                    send_probe_script(
                        &mut self.wv,
                        &format!(
                            "window.__probeMark && window.__probeMark(-{})",
                            index as i64 + 1
                        ),
                    );
                }
                None => {
                    self.scripts.push(ScriptRow {
                        index,
                        url: url.to_string(),
                        status: None,
                        bytes: 0,
                        body_prefix: String::new(),
                        eval_ms: None,
                        errors: vec!["fetch failed".to_string()],
                    });
                    self.wv.on_script_fetch_failed(index);
                }
            },
            FetchKind::DynamicScript { node_id } => match fetch_ok(&self.core, &url) {
                Some(r) => self.wv.on_dynamic_script_fetched(
                    node_id,
                    String::from_utf8_lossy(&r.body).to_string(),
                ),
                None => self.wv.on_dynamic_script_fetch_failed(node_id),
            },
            FetchKind::DynamicCss { node_id } => {
                if let Some(r) = fetch_ok(&self.core, &url) {
                    self.wv.on_dynamic_style_fetched(
                        node_id,
                        String::from_utf8_lossy(&r.body).to_string(),
                        &url,
                    );
                }
            }
            FetchKind::CssImport { target } => {
                if let Some(r) = fetch_ok(&self.core, &url) {
                    self.wv.on_css_import_fetched(
                        String::from_utf8_lossy(&r.body).to_string(),
                        &target,
                    );
                } else {
                    self.wv.on_css_import_fetch_failed(&target);
                }
            }
            FetchKind::Image { source } => {
                if let Some(r) = fetch_ok(&self.core, &url)
                    && let Err(e) = self.wv.on_image_fetched(source, &r.body)
                {
                    log::warn!(target: "ytprobe", "image fetch rejected: {e}");
                }
            }
            FetchKind::Audio { source } => {
                if let Some(r) = fetch_ok(&self.core, &url) {
                    self.wv.on_audio_fetched(source, &r.body);
                }
            }
            FetchKind::JavaScript {
                request_id,
                method,
                headers,
                body,
            } => match self.core.fetch_request_blocking(&NetworkRequest {
                url: url.to_string(),
                method,
                headers,
                body,
            }) {
                Ok(r) => self.wv.on_js_fetch_succeeded(
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
                Err(e) => self.wv.on_js_fetch_failed(request_id, e.to_string()),
            },
            FetchKind::Iframe { dom_id } => self.wv.on_iframe_fetch_failed(dom_id),
        }
    }

    /// Answers one round of webview tasks and records DOM commits.
    fn tick(&mut self) {
        let tasks = self.wv.tick();
        for task in tasks {
            self.answer(task);
        }
        self.note_dom_commit();
    }

    fn dom_version(&mut self) -> u64 {
        self.wv
            .inspect("getVersion", "{}")
            .ok()
            .and_then(|v| v["domVersion"].as_u64())
            .unwrap_or(0)
    }

    /// Keeps the pipeline running for `period`, answering the tasks script
    /// execution and timer callbacks emit.
    fn pump_for(&mut self, period: Duration) {
        let deadline = Instant::now() + period;
        while Instant::now() < deadline {
            self.tick();
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Runs the report task and returns the serialized document together with
    /// the parsed probe report, waiting until the report DOM node shows up.
    fn read_probe_state(&mut self) -> (Option<serde_json::Value>, Option<serde_json::Value>) {
        send_probe_script(
            &mut self.wv,
            "try { if (window.__probeReport) { window.__probeReport(); } else if (window.__probe) { console.log('[probe] has probe'); } else { console.log('[probe] no probe'); } } catch(e) { console.log('[probe] err ' + e); }",
        );
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            self.pump_for(Duration::from_millis(250));
            let document = self.wv.inspect("getDocument", "{}").ok();
            let report = document
                .as_ref()
                .and_then(|doc| element_text_by_id(doc, "__probe_report"))
                .and_then(|text| serde_json::from_str(&text).ok());
            if report.is_some() {
                return (document, report);
            }
            if Instant::now() >= deadline {
                eprintln!(
                    "[probe-debug] failed to find report, doc present: {}",
                    document.is_some()
                );
                return (document, report);
            }
        }
    }

    fn script_kinds(&self) -> (usize, usize) {
        let mut inline = 0;
        let mut external = 0;
        for script in &self.wv.classic_scripts {
            match script {
                ClassicScript::Inline(_) => inline += 1,
                ClassicScript::External { .. } => external += 1,
            }
        }
        (inline, external)
    }

    /// Copies eval times out of the page report onto the matching script rows.
    fn apply_timings(&mut self, report: &serde_json::Value) {
        let Some(times) = report.get("times").and_then(|v| v.as_array()) else {
            return;
        };
        let marks: BTreeMap<i64, f64> = times
            .iter()
            .filter_map(|entry| {
                let pair = entry.as_array()?;
                Some((pair.first()?.as_i64()?, pair.get(1)?.as_f64()?))
            })
            .collect();
        for row in &mut self.scripts {
            // Opening marks carry the script index; closing marks carry its
            // negated one-based form.
            if let (Some(start), Some(end)) = (
                marks.get(&(row.index as i64)),
                marks.get(&-(row.index as i64 + 1)),
            ) {
                row.eval_ms = Some(end - start);
            }
        }
    }

    /// Attributes captured `JS error` log lines to the script whose source
    /// prefix the message quotes.
    fn apply_errors(&mut self, logs: &[CapturedLog]) {
        for row in &mut self.scripts {
            if row.body_prefix.len() < 16 {
                continue;
            }
            let quoted = format!("{:?}", row.body_prefix);
            let needle = quoted.trim_matches('"');
            row.errors.extend(
                logs.iter()
                    .filter(|log| log.message.contains("JS error"))
                    .filter(|log| log.message.contains(needle))
                    .map(|log| log.message.clone()),
            );
        }
    }
}

/// Sends a probe-owned script to the JS thread, keeping the pending-task
/// bookkeeping in sync.
fn send_probe_script(wv: &mut WebView, source: &str) {
    let sent = if let Some(processor) = wv.js_processor.as_ref() {
        processor.send(JsTask::RunScript {
            source: source.to_string(),
        });
        true
    } else {
        false
    };
    if sent {
        wv.pending_js_tasks += 1;
    }
}

fn fetch_ok(core: &AsyncNetworkCore, url: &Url) -> Option<Response> {
    if url.scheme() == "data" {
        let body = crate::browser::core::resource_loader::DataURI::decode(url.as_str()).ok()?;
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
            ("User-Agent".to_string(), PROBE_USER_AGENT.to_string()),
            ("Accept-Language".to_string(), "en-US,en;q=0.9".to_string()),
            ("Accept".to_string(), "*/*".to_string()),
        ],
        body: Vec::new(),
    })
    .ok()
}

/// Fetches the probe's initial document with navigation-grade headers.
fn fetch_document(core: &AsyncNetworkCore, url: &Url) -> Response {
    core.fetch_request_blocking(&NetworkRequest {
        url: url.to_string(),
        method: "GET".to_string(),
        headers: vec![
            ("User-Agent".to_string(), PROBE_USER_AGENT.to_string()),
            ("Accept-Language".to_string(), "en-US,en;q=0.9".to_string()),
            (
                "Accept".to_string(),
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8".to_string(),
            ),
        ],
        body: Vec::new(),
    })
    .unwrap_or_else(|e| panic!("fetch {url}: {e}"))
}

fn count_tags(value: &serde_json::Value, counts: &mut BTreeMap<String, usize>, text: &mut usize) {
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

/// Concatenates the text of the first element with `id="<id>"`.
fn element_text_by_id(value: &serde_json::Value, id: &str) -> Option<String> {
    if let Some(attributes) = value.get("attributes").and_then(|a| a.as_array()) {
        let is_target = attributes.iter().any(|attribute| {
            attribute.as_array().is_some_and(|pair| {
                pair.len() == 2 && pair[0].as_str() == Some("id") && pair[1].as_str() == Some(id)
            })
        });
        if is_target {
            let mut text = String::new();
            collect_text(value, &mut text);
            return Some(text);
        }
    }
    if let Some(children) = value.get("children").and_then(|c| c.as_array()) {
        for child in children {
            if let Some(found) = element_text_by_id(child, id) {
                return Some(found);
            }
        }
    }
    None
}

fn collect_text(value: &serde_json::Value, out: &mut String) {
    if let Some(text) = value.get("text").and_then(|t| t.as_str()) {
        out.push_str(text);
    }
    if let Some(children) = value.get("children").and_then(|c| c.as_array()) {
        for child in children {
            collect_text(child, out);
        }
    }
}

fn run_secs() -> u64 {
    std::env::var("YT_RUN_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(45)
        .min(120)
}

/// Drives one live page end to end and prints the measurement report.
fn drive(target: Url) {
    install_log_capture();
    let started = Instant::now();
    let logs_before = captured_logs().len();

    let core = AsyncNetworkCore::new(Arc::new(SharedNetState::new()));
    log::info!(target: "ytprobe", "fetching {target}");
    let initial = fetch_document(&core, &target);
    log::info!(
        target: "ytprobe",
        "{} HTML: status {}, {} bytes",
        target,
        initial.status.as_u16(),
        initial.body.len()
    );
    let html = String::from_utf8_lossy(&initial.body).to_string();

    let mut probe = Probe::new(core, html, target.clone());

    let limit = run_secs();
    let mut iterations: u64 = 0;
    let mut last_version = 0u64;
    let mut stable_for: u32 = 0;

    loop {
        if started.elapsed().as_secs() >= limit {
            break;
        }
        probe.tick();
        iterations += 1;

        if iterations.is_multiple_of(400) {
            eprintln!(
                "[ytprobe] t={:.1}s ticks={} dom={} commits={} scripts={} pending={}",
                started.elapsed().as_secs_f64(),
                iterations,
                probe.dom_version(),
                probe.dom_commits,
                probe.scripts.len(),
                probe.wv.pending_js_tasks,
            );
        }

        let version = probe.dom_version();
        if version == last_version {
            stable_for += 1;
        } else {
            stable_for = 0;
            last_version = version;
        }
        if stable_for > 250 && started.elapsed().as_secs() >= MIN_RUN_SECS {
            log::info!(target: "ytprobe", "DOM stable for 2.5s; stopping early");
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    // Let the last script finish, then collect the page report.
    probe.pump_for(Duration::from_secs(3));
    let (document, report) = probe.read_probe_state();
    let elapsed = started.elapsed();

    eprintln!(
        "\n== YOUTUBE PROBE SUMMARY ({target}) ==\nelapsed: {:.1}s, ticks: {iterations}, dom commits: {}",
        elapsed.as_secs_f64(),
        probe.dom_commits
    );
    eprintln!("fetches: {:?}", probe.fetch_counts);
    let (inline, external) = probe.script_kinds();
    eprintln!(
        "classic scripts: {external} external ({}/{} fetched), {inline} inline",
        probe
            .scripts
            .iter()
            .filter(|row| row.status.is_some())
            .count(),
        external
    );
    if let Ok(version) = probe.wv.inspect("getVersion", "{}") {
        eprintln!("versions: {version}");
    }

    if let Some(report) = report.as_ref() {
        probe.apply_timings(report);
        if let Some(console) = report.get("console").and_then(|c| c.as_array()) {
            let mut missing: Vec<String> = Vec::new();
            for line in console.iter().filter_map(|line| line.as_str()) {
                if let Some(names) = line.strip_prefix("[warn] [missing-api] ") {
                    missing.extend(names.split(',').map(|name| name.trim().to_string()));
                }
            }
            missing.sort();
            missing.dedup();
            eprintln!("\n-- missing Web Platform APIs ({}) --", missing.len());
            for name in &missing {
                eprintln!("  {name}");
            }

            eprintln!("\n-- page console (warn/error) --");
            let mut printed = 0usize;
            for line in console.iter().filter_map(|line| line.as_str()) {
                if line.starts_with("[warn] [missing-api] ") {
                    continue;
                }
                if printed >= 40 {
                    eprintln!("  ... ({} more)", console.len() - printed);
                    break;
                }
                eprintln!("  {line}");
                printed += 1;
            }
            if printed == 0 {
                eprintln!("  (none)");
            }
        }

        eprintln!("\n-- page state --");
        if let Some(fields) = report.as_object() {
            for (name, value) in fields {
                if name == "console" || name == "times" {
                    continue;
                }
                eprintln!("  {name} = {value}");
            }
        }
    } else {
        eprintln!("\n[probe] page report unavailable (collector did not run)");
    }

    probe.apply_errors(&captured_logs());
    probe.scripts.sort_by_key(|row| row.index);

    eprintln!("\n-- external scripts (fetch → eval) --");
    if probe.scripts.is_empty() {
        eprintln!("  (none fetched)");
    }
    for row in &probe.scripts {
        let status = row
            .status
            .map(|status| status.to_string())
            .unwrap_or_else(|| "FAILED".to_string());
        let eval = row
            .eval_ms
            .map(|ms| format!("{ms:.1}ms"))
            .unwrap_or_else(|| "-".to_string());
        eprintln!(
            "  [{:>2}] {:>6} {:>9}B {:>9} {}",
            row.index, status, row.bytes, eval, row.url
        );
    }

    eprintln!("\n-- JS errors --");
    let logs = captured_logs();
    let errors: Vec<&CapturedLog> = logs
        .iter()
        .skip(logs_before.min(logs.len()))
        .filter(|log| log.message.contains("JS error"))
        .collect();
    if errors.is_empty() {
        eprintln!("  (none captured)");
    }
    for log in &errors {
        eprintln!(
            "  t={:.2}s {}",
            log.at.duration_since(started).as_secs_f64(),
            log.message
        );
    }

    let mut attributed = 0usize;
    for row in &probe.scripts {
        for error in &row.errors {
            attributed += 1;
            eprintln!("  -> script {} ({}): {}", row.index, row.url, error);
        }
    }
    if attributed > 0 {
        eprintln!("  ({attributed} attributed to an external script)");
    }

    if let Some(document) = document.as_ref() {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut text_len = 0usize;
        count_tags(document, &mut counts, &mut text_len);
        eprintln!("\n-- DOM tags --");
        eprintln!(
            "  text length: {text_len}, elements: {}",
            counts.values().sum::<usize>()
        );
        eprintln!("  {counts:?}");
        for marker in [
            "video",
            "ytd-app",
            "ytd-masthead",
            "ytd-searchbox",
            "ytd-player",
            "ytd-watch-flexy",
            "ytd-thumbnail",
            "input",
            "iframe",
        ] {
            eprintln!(
                "  marker <{marker}>: {}",
                counts.get(marker).copied().unwrap_or(0)
            );
        }
    } else {
        eprintln!("\n[probe] getDocument failed");
    }

    if let Some((layout, info)) = probe.wv.layout_and_info() {
        eprintln!(
            "layout present: layout-root children={}, info children={}",
            layout.children.len(),
            info.children.len()
        );
    } else {
        eprintln!("layout: none");
    }
}

/// Drives https://www.youtube.com/ and reports how far the home page boots.
///
/// ```sh
/// cargo test --lib -- --ignored --nocapture drive_youtube_with_real_fetches
/// RUST_LOG=info YT_RUN_SECS=60 cargo test --lib -- --ignored --nocapture drive_youtube_with_real_fetches
/// ```
#[test]
#[ignore = "live network probe against youtube.com; manual run only"]
fn drive_youtube_with_real_fetches() {
    drive(Url::parse("https://www.youtube.com/").unwrap());
}

/// Drives a watch page and reports how far the player page boots.
///
/// Set `YT_VIDEO_ID` to probe another video.
///
/// ```sh
/// cargo test --lib -- --ignored --nocapture drive_youtube_watch_with_real_fetches
/// YT_VIDEO_ID=dQw4w9WgXcQ YT_RUN_SECS=60 \
///   cargo test --lib -- --ignored --nocapture drive_youtube_watch_with_real_fetches
/// ```
#[test]
#[ignore = "live network probe against youtube.com; manual run only"]
fn drive_youtube_watch_with_real_fetches() {
    let video_id = std::env::var("YT_VIDEO_ID").unwrap_or_else(|_| "dQw4w9WgXcQ".to_string());
    drive(Url::parse(&format!("https://www.youtube.com/watch?v={video_id}")).unwrap());
}
