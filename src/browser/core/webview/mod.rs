//! ブラウザのwebview機能。タスクとレンダリング情報の管理を行う。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::{Arc, mpsc};

use crate::engine::image_decoder::ImageDecoder;
use crate::engine::layouter::types::{ColorScheme, TextFlowStyle};
use crate::engine::{
    css::{
        self,
        matcher::ElementChain,
        parser::{CssNode, CssNodeType, Parser as CssParser},
        values::CssValue,
    },
    html::HtmlNodeType,
    html::parser::{
        ClassicScriptExecution, ClassicScriptSource, DomTree, Parser as HtmlParser, ScriptingMode,
    },
    js::{
        JsDevToolsRequest, JsDynamicImageRequest, JsDynamicScriptRequest, JsDynamicScriptSource,
        JsDynamicStyleRequest, JsFetchRequest, JsFetchResponse, JsIframeFetchRequest,
        JsLayoutMetrics, JsProcessor, JsTask, JsTaskResult,
    },
    layouter::{
        self, InheritedCss, LayoutResult, NodeId,
        dom_snapshot::DomSnapshot,
        types::{InfoNode, NodeKind},
    },
    origin::Origin,
    renderer_model::Image,
    tree::{NodeRef, TreeNode},
};
use crate::platform::{locale, renderer::text_measurer::PlatformTextMeasurer};
use crate::{perf_scope, profile_log};
use ui_layout::{LayoutChild, LayoutNode};
use url::Url;

const USER_AGENT_CSS: &str = include_str!("../../../../resource/user-agent.css");

pub enum WebViewTask {
    AskTabHtml,
    Fetch {
        url: Url,
        kind: FetchKind,
    },
    /// A page asked the DevTools bridge to inspect rendered state.
    DevToolsRequest {
        id: u64,
        method: String,
        params: String,
    },
}

mod inspector;

#[derive(Debug, Clone, PartialEq)]
pub enum FetchKind {
    Html,
    Css,
    Script {
        index: usize,
    },
    DynamicScript {
        node_id: u64,
    },
    DynamicCss {
        node_id: u64,
    },
    Image {
        source: String,
    },
    Audio {
        source: String,
    },
    JavaScript {
        request_id: u64,
        method: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    Iframe {
        dom_id: u64,
    },
}

/// CSS application strategy.
///
/// - `Batch`: wait for all external CSS to be fetched, then process everything
///   at once on a background thread and apply the result.
/// - `Incremental`: process each CSS file on a background thread as it arrives,
///   applying results progressively.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CssApplicationStrategy {
    Batch,
    Incremental,
}

/// JavaScript execution policy for a page.
///
/// This drives both the HTML parser's scripting mode (so `<noscript>` fallbacks
/// are either hidden or shown) and whether the WebView executes scripts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JsPolicy {
    /// Scripts run and `<noscript>` contents are kept as raw text.
    #[default]
    Enabled,
    /// Scripts are never executed and `<noscript>` fallbacks are shown.
    Disabled,
}

impl From<JsPolicy> for ScriptingMode {
    fn from(value: JsPolicy) -> ScriptingMode {
        match value {
            JsPolicy::Enabled => ScriptingMode::Enabled,
            JsPolicy::Disabled => ScriptingMode::Disabled,
        }
    }
}

impl From<ScriptingMode> for JsPolicy {
    fn from(value: ScriptingMode) -> JsPolicy {
        match value {
            ScriptingMode::Enabled => JsPolicy::Enabled,
            ScriptingMode::Disabled => JsPolicy::Disabled,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum PagePhase {
    Init,
    BeforeHtmlParsing,
    HtmlParsed,
    CssPending,
    CssProcessing,
    CssApplied,
    ScriptApplied,
}

pub struct WebView {
    phase: PagePhase,

    docment_info: Option<DocumentInfo>,

    pending_css_urls: Vec<Url>,
    pending_images: Vec<(String, Url)>,
    pending_audio: Vec<(String, Url)>,
    loaded_css: Vec<String>,
    linked_css: Vec<String>,
    images: HashMap<String, Image>,
    image_decoder: ImageDecoder,
    audio: HashMap<String, Arc<[u8]>>,

    resolved_styles: Arc<layouter::css_resolver::ResolvedStyles>,
    /// Monotonic version of `resolved_styles`, bumped on every in-place or
    /// wholesale mutation so the layout processor can detect stale rule sets.
    resolved_styles_version: u64,
    layout_and_info: Option<(LayoutNode, InfoNode)>,

    needs_redraw: bool,

    text_measurer: Option<Arc<PlatformTextMeasurer>>,

    system_color_scheme: ColorScheme,
    viewport: (f32, f32),

    css_processor: css::processor::CssProcessor,
    css_strategy: CssApplicationStrategy,
    css_results_expected: usize,
    css_results_received: usize,

    /// Policy controlling whether page scripts are executed and how
    /// `<noscript>` contents are parsed.
    js_policy: JsPolicy,

    layout_processor: layouter::LayoutProcessor,
    layout_pending: bool,
    layout_requested_version: u64,
    layout_applied_version: u64,
    /// The `(layout version, viewport)` the current tree was last positioned
    /// for. Applied background results start out unpositioned; hit-testing
    /// must never observe them before [`WebView::position_layout_if_needed`]
    /// has run, so this memo gate runs the positioning pass eagerly.
    positioned_layout: Option<(u64, (f32, f32))>,
    /// Live DOM references for the latest snapshot, used to apply write-backs.
    layout_dom_refs: Vec<Weak<RefCell<TreeNode<HtmlNodeType>>>>,
    /// Cached DOM snapshot, reused while the tree's mutation version is
    /// unchanged so that CSS/image-driven relayouts skip the full clone.
    snapshot_cache: Option<SnapshotCache>,
    /// The most recent serialized content documents of any `<iframe>`s, keyed
    /// by the iframe's JS-facing dom id. Used by layout to render them nested.
    iframe_content: HashMap<u64, DomSnapshot>,
    /// Channel on which text inputs report value write-backs (received here).
    write_back_tx: mpsc::Sender<(u32, String)>,
    write_back_rx: mpsc::Receiver<(u32, String)>,
    /// JS runtime on a background thread, sharing a mirror of the current
    /// document's DOM. Results are applied in [`WebView::try_apply_js_results`].
    js_processor: Option<JsProcessor>,
    /// JS-facing dom id per live node address of the committed tree.
    ///
    /// Rebuilt whenever a JS result is committed, so hit-tested layout
    /// nodes and write-back serialization can be translated to JS dom ids.
    js_dom_ids: HashMap<usize, u64>,
    /// Ordered JS tasks sent but not yet applied. Write-backs are only synced
    /// to the JS thread once this reaches zero, so the mirror and the real tree
    /// cannot diverge mid-task.
    pending_js_tasks: usize,
    /// The real DOM diverged from the JS thread's mirror and needs syncing.
    js_dom_dirty: bool,
    /// Version of the newest `RunTimers` poke still in flight. Write-backs are
    /// not synced while one is pending: a timer callback can mutate the mirror,
    /// and a snapshot produced behind the sync would clobber it.
    in_flight_timer_version: Option<u64>,
    /// Whether the window `load` event has been dispatched for the current page.
    window_load_dispatched: bool,
    /// `fetch()` requests collected from applied JS results.
    pending_js_fetches: Vec<JsFetchRequest>,
    /// DevTools inspection requests collected from applied JS results.
    pending_devtools_requests: Vec<JsDevToolsRequest>,
    /// Stable DOM ids for the inspector, assigned lazily over the live tree.
    inspector_ids: RefCell<inspector::DomIdRegistry>,
    /// Dynamically inserted scripts collected from applied JS results.
    pending_dynamic_scripts: Vec<JsDynamicScriptRequest>,
    /// Dynamically inserted stylesheet links collected from JS results.
    pending_dynamic_styles: Vec<JsDynamicStyleRequest>,
    /// Images created or populated by scripts, awaiting network scheduling.
    pending_dynamic_images: Vec<JsDynamicImageRequest>,
    /// `<iframe src="...">` requests queued by JS results, awaiting fetch.
    pending_iframe_fetches: Vec<JsIframeFetchRequest>,
    /// Classic scripts in document order. Execution starts after CSS is applied.
    classic_scripts: Vec<ClassicScript>,
    next_script_index: usize,
    pending_script_fetches: HashMap<usize, ClassicScriptExecution>,
    non_blocking_scripts_scheduled: bool,
    deferred_script_results: HashMap<usize, Option<String>>,
    next_deferred_script_index: usize,
    /// Fragment to reveal once the document has a completed layout.
    pending_fragment_scroll: Option<String>,
    /// Layout generation that contains every initially linked stylesheet.
    fragment_ready_version: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
enum ClassicScript {
    Inline(String),
    External {
        url: Url,
        execution: ClassicScriptExecution,
    },
}

/// A DOM snapshot paired with the tree mutation version it was built from.
///
/// The snapshot and its live references stay valid as long as the DOM has not
/// mutated (`Tree::version()` unchanged). They are shared with layout tasks via
/// `Arc` instead of being cloned per task.
struct SnapshotCache {
    dom_version: u64,
    snapshot: Arc<DomSnapshot>,
    dom_refs: Vec<Weak<RefCell<TreeNode<HtmlNodeType>>>>,
}

fn js_snapshot_from_tree(dom: &DomTree) -> (DomSnapshot, HashMap<usize, u64>) {
    let mut dom_ids = HashMap::new();
    let mut next_id = 1u64;
    dom.traverse(|node| {
        dom_ids.insert(Rc::as_ptr(node) as usize, next_id);
        next_id += 1;
    });
    (DomSnapshot::from_mirror(&dom.root, &dom_ids), dom_ids)
}

/// Grafts each committed iframe's content document into its host `<iframe>`
/// node so the normal layout/paint pipeline renders the content nested inside
/// the host box. The JS domain keeps iframe documents in a separate tree, so we
/// splice their `<html>` subtree under the matching host node here.
fn graft_iframe_documents(
    dom: &Rc<DomTree>,
    js_dom_ids: &HashMap<usize, u64>,
    iframes: &HashMap<u64, DomSnapshot>,
) {
    if iframes.is_empty() {
        return;
    }
    // Build a reverse map: js_dom_id -> live node, so host lookups are O(1)
    // per iframe instead of O(n) DOM traversals.
    let mut node_by_dom_id: HashMap<u64, NodeRef<HtmlNodeType>> = HashMap::new();
    dom.traverse(|node| {
        if let Some(&dom_id) = js_dom_ids.get(&(Rc::as_ptr(node) as usize)) {
            node_by_dom_id.insert(dom_id, Rc::clone(node));
        }
    });
    for (iframe_dom_id, content) in iframes {
        let (content_tree, _ids) = content.into_tree();
        let Some(html) = content_tree.query_selector("html") else {
            continue;
        };
        let Some(host) = node_by_dom_id.get(iframe_dom_id) else {
            continue;
        };
        if host.borrow().value.tag_name() != Some("iframe") {
            continue;
        }
        TreeNode::add_child(host, html);
    }
}

impl std::fmt::Debug for SnapshotCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnapshotCache")
            .field("dom_version", &self.dom_version)
            .field("snapshot", &self.snapshot)
            .finish()
    }
}

/// DocumentInfo holds basic information about the HTML document.
/// It includes the document URL, base URL, title, and DOM tree.
///
/// - document_url: The URL of the document.
/// - base_url: The base URL for resolving relative URLs.
/// - origin: The origin of the document.
/// - title: The title of the document.
/// - dom: The DOM tree of the document.
#[derive(Debug)]
pub struct DocumentInfo {
    document_url: Url,
    base_url: Url,
    pub origin: crate::engine::origin::Origin,
    title: String,
    pub dom: Rc<DomTree>,
}

/// ParsedDocument holds the result of parsing an HTML document.
/// It includes the document URL, base URL, DOM tree, title, style links, and inline styles.
///
/// - document_url: The URL of the document.
/// - base_url: The base URL for resolving relative URLs.
/// - dom: The DOM tree of the document.
/// - title: The title of the document.
/// - style_links: A list of URLs for linked stylesheets.
/// - inline_styles: A list of inline CSS styles.
/// - scripts: A list of inline script sources.
struct ParsedDocument {
    document_url: Url,
    base_url: Url,
    dom: Rc<DomTree>,
    title: String,
    style_links: Vec<Url>,
    inline_styles: Vec<String>,
    image_sources: Vec<(String, Url)>,
    audio_sources: Vec<(String, Url)>,
    scripts: Vec<ClassicScript>,
}

impl Default for WebView {
    fn default() -> Self {
        Self::new(ColorScheme::default(), JsPolicy::default())
    }
}

impl WebView {
    pub fn new(system_color_scheme: ColorScheme, js_policy: JsPolicy) -> Self {
        let (write_back_tx, write_back_rx) = mpsc::channel();
        Self {
            phase: PagePhase::Init,

            docment_info: None,

            pending_css_urls: Vec::new(),
            pending_images: Vec::new(),
            pending_audio: Vec::new(),
            loaded_css: Vec::new(),
            linked_css: Vec::new(),
            images: HashMap::new(),
            image_decoder: ImageDecoder::new(),
            audio: HashMap::new(),

            resolved_styles: Arc::new(layouter::css_resolver::ResolvedStyles::default()),
            resolved_styles_version: 0,
            layout_and_info: None,

            needs_redraw: false,

            text_measurer: None,

            system_color_scheme,
            viewport: (800.0, 600.0),

            css_processor: css::processor::CssProcessor::new(),
            css_strategy: CssApplicationStrategy::Incremental,
            css_results_expected: 0,
            css_results_received: 0,

            js_policy,

            layout_processor: layouter::LayoutProcessor::new(),
            layout_pending: false,
            layout_requested_version: 0,
            layout_applied_version: 0,
            positioned_layout: None,
            layout_dom_refs: Vec::new(),
            snapshot_cache: None,
            iframe_content: HashMap::new(),
            write_back_tx,
            write_back_rx,
            js_processor: None,
            js_dom_ids: HashMap::new(),
            pending_js_tasks: 0,
            js_dom_dirty: false,
            in_flight_timer_version: None,
            window_load_dispatched: false,
            pending_js_fetches: Vec::new(),
            pending_devtools_requests: Vec::new(),
            inspector_ids: RefCell::new(inspector::DomIdRegistry::default()),
            pending_dynamic_scripts: Vec::new(),
            pending_dynamic_styles: Vec::new(),
            pending_dynamic_images: Vec::new(),
            pending_iframe_fetches: Vec::new(),
            classic_scripts: Vec::new(),
            next_script_index: 0,
            pending_script_fetches: HashMap::new(),
            non_blocking_scripts_scheduled: false,
            deferred_script_results: HashMap::new(),
            next_deferred_script_index: 0,
            pending_fragment_scroll: None,
            fragment_ready_version: None,
        }
    }

    /// Set the CSS application strategy.
    ///
    /// Default is `Incremental`.
    pub fn set_css_strategy(&mut self, strategy: CssApplicationStrategy) {
        self.css_strategy = strategy;
    }

    /// Sets the JavaScript execution policy.
    ///
    /// The policy is applied to `<noscript>` parsing on the next document load
    /// and takes effect immediately for execution: disabling it drops the JS
    /// runtime, cancels pending script work, and stops running scripts.
    pub fn set_js_policy(&mut self, policy: JsPolicy) {
        if self.js_policy == policy {
            return;
        }
        self.js_policy = policy;

        if policy == JsPolicy::Disabled {
            self.teardown_script_execution();
        } else if self.js_processor.is_none()
            && let Some(dom) = self.docment_info.as_ref().map(|info| Rc::clone(&info.dom))
        {
            // Re-enabling: install a processor so DOM APIs work again, without
            // replaying scripts that were skipped while disabled.
            let document_url = self
                .docment_info
                .as_ref()
                .map(|info| info.document_url.to_string())
                .unwrap_or_default();
            let origin = self
                .docment_info
                .as_ref()
                .map(|info| info.origin.ascii_serialization())
                .unwrap_or_else(|| "null".to_string());
            let (snapshot, dom_ids) = js_snapshot_from_tree(&dom);
            let processor = JsProcessor::new(snapshot);
            processor.send(JsTask::SetDocumentUrl { url: document_url });
            processor.send(JsTask::SetOrigin { origin });
            processor.send(JsTask::SetViewport {
                width: self.viewport.0,
                height: self.viewport.1,
            });
            processor.send(JsTask::SetLanguage {
                language: locale::preferred_language(),
            });
            self.js_processor = Some(processor);
            self.js_dom_ids = dom_ids;
            self.pending_js_tasks = 4;
        }
    }

    /// Returns the current JavaScript execution policy.
    pub fn js_policy(&self) -> JsPolicy {
        self.js_policy
    }

    /// Stops script execution immediately, dropping the runtime and any
    /// pending script work.
    fn teardown_script_execution(&mut self) {
        self.js_processor = None;
        self.pending_js_tasks = 0;
        self.js_dom_dirty = false;
        self.in_flight_timer_version = None;
        self.pending_js_fetches.clear();
        self.pending_devtools_requests.clear();
        self.inspector_ids.borrow_mut().clear();
        self.pending_dynamic_scripts.clear();
        self.pending_dynamic_styles.clear();
        self.pending_dynamic_images.clear();
        self.pending_iframe_fetches.clear();
        self.js_dom_ids.clear();
        self.classic_scripts.clear();
        self.next_script_index = 0;
        self.pending_script_fetches.clear();
        self.non_blocking_scripts_scheduled = false;
        self.deferred_script_results.clear();
        self.next_deferred_script_index = 0;

        // Nothing will advance past CssApplied anymore.
        if self.phase == PagePhase::CssApplied {
            self.phase = PagePhase::ScriptApplied;
        }
    }

    pub fn tick(&mut self) -> Vec<WebViewTask> {
        let mut tasks = Vec::new();

        match self.phase {
            PagePhase::Init => {
                let ua_styles = layouter::css_resolver::CssResolver::resolve_with_origin(
                    &CssParser::new(USER_AGENT_CSS).parse().unwrap(),
                    layouter::css_resolver::StyleOrigin::UserAgent,
                );
                layouter::css_resolver::append_resolved_styles(
                    Arc::make_mut(&mut self.resolved_styles),
                    ua_styles,
                );
                self.resolved_styles_version += 1;

                tasks.push(WebViewTask::AskTabHtml);

                self.phase = PagePhase::BeforeHtmlParsing;
            }

            PagePhase::BeforeHtmlParsing => {}

            PagePhase::HtmlParsed => {
                // Phase 1: UA.css only layout
                self.ensure_text_measurer();
                self.update_layout();

                for (source, url) in std::mem::take(&mut self.pending_images) {
                    log::info!("Image fetch requested in WebView: url={}", url);
                    tasks.push(WebViewTask::Fetch {
                        url,
                        kind: FetchKind::Image { source },
                    });
                }

                for (source, url) in std::mem::take(&mut self.pending_audio) {
                    log::info!("Audio fetch requested in WebView: url={}", url);
                    tasks.push(WebViewTask::Fetch {
                        url,
                        kind: FetchKind::Audio { source },
                    });
                }

                // CSS fetch を要求
                if self.pending_css_urls.is_empty() {
                    self.fragment_ready_version = Some(self.layout_requested_version);
                    self.phase = PagePhase::CssApplied;
                } else {
                    for url in &self.pending_css_urls {
                        log::info!("Fetch requested in WebView: url={}", url);
                        tasks.push(WebViewTask::Fetch {
                            url: url.clone(),
                            kind: FetchKind::Css,
                        });
                    }

                    self.phase = PagePhase::CssPending;
                }
            }

            PagePhase::CssPending => {
                // Poll for CSS processor results (Incremental strategy)
                self.try_apply_css_results();
            }

            PagePhase::CssProcessing => {
                // Poll for the single batch result (Batch strategy)
                self.try_apply_batch_result();
            }

            PagePhase::CssApplied => {
                self.advance_classic_scripts(&mut tasks);
            }

            PagePhase::ScriptApplied => {
                // 安定状態
            }
        }

        self.try_apply_js_results();
        self.schedule_js_fetches(&mut tasks);
        self.schedule_iframe_fetches(&mut tasks);
        self.schedule_dynamic_scripts(&mut tasks);
        for request in std::mem::take(&mut self.pending_devtools_requests) {
            tasks.push(WebViewTask::DevToolsRequest {
                id: request.id,
                method: request.method,
                params: request.params,
            });
        }
        self.schedule_dynamic_styles(&mut tasks);
        self.schedule_dynamic_images(&mut tasks);
        self.schedule_pending_images(&mut tasks);
        self.try_apply_decoded_images();
        self.run_due_js_timers();
        self.try_apply_layout_results();
        self.drain_write_backs();
        self.sync_dom_to_worker();

        // Window `load` fires once the page is stable: after DOMContentLoaded
        // (phase `ScriptApplied`), with no JS-visible subresource work still in
        // flight. Reaching `ScriptApplied` guarantees the page scripts and
        // DOMContentLoaded listeners have already been applied, so the `onload`
        // handler is in place before we dispatch.
        if !self.window_load_dispatched
            && self.phase == PagePhase::ScriptApplied
            && self.pending_js_tasks == 0
            && !self.has_pending_subresource_work()
        {
            self.dispatch_window_load();
        }

        tasks
    }

    pub fn on_html_fetched(&mut self, html: String, document_url: Url) {
        log::info!("Fetched HTML: {}", document_url);
        self.pending_fragment_scroll = document_url.fragment().map(str::to_string);
        self.fragment_ready_version = None;
        perf_scope!(html_parse);
        let parsed = parse_html(&html, document_url, self.js_policy.into());
        #[cfg(any(feature = "profile", debug_assertions))]
        let html_parse_time = html_parse.elapsed();

        self.pending_css_urls = parsed.style_links;
        self.pending_images = parsed.image_sources;
        self.pending_audio = parsed.audio_sources;
        self.classic_scripts = parsed.scripts;
        self.next_script_index = 0;
        self.pending_script_fetches.clear();
        self.non_blocking_scripts_scheduled = false;
        self.deferred_script_results.clear();
        self.next_deferred_script_index = 0;
        self.css_results_expected = self.pending_css_urls.len();

        let mut initial_js_tasks = 0;
        let mut initial_js_dom_ids = HashMap::new();
        #[cfg(any(feature = "profile", debug_assertions))]
        let mut js_snapshot_time = std::time::Duration::ZERO;
        self.js_processor = if self.js_policy == JsPolicy::Enabled {
            perf_scope!(js_snapshot);
            let (snapshot, dom_ids) = js_snapshot_from_tree(&parsed.dom);
            #[cfg(any(feature = "profile", debug_assertions))]
            {
                js_snapshot_time = js_snapshot.elapsed();
            }
            initial_js_dom_ids = dom_ids;
            let processor = JsProcessor::new(snapshot);
            processor.send(JsTask::SetDocumentUrl {
                url: parsed.document_url.to_string(),
            });
            processor.send(JsTask::SetOrigin {
                origin: Origin::from_url(&parsed.document_url).ascii_serialization(),
            });
            processor.send(JsTask::SetViewport {
                width: self.viewport.0,
                height: self.viewport.1,
            });
            processor.send(JsTask::SetLanguage {
                language: locale::preferred_language(),
            });
            initial_js_tasks = 4;
            Some(processor)
        } else {
            None
        };
        self.pending_js_tasks = initial_js_tasks;
        self.js_dom_ids = initial_js_dom_ids;
        self.js_dom_dirty = false;
        self.in_flight_timer_version = None;
        self.window_load_dispatched = false;
        self.pending_js_fetches.clear();
        self.pending_devtools_requests.clear();
        self.inspector_ids.borrow_mut().clear();
        self.pending_dynamic_scripts.clear();
        self.pending_dynamic_styles.clear();
        self.pending_dynamic_images.clear();
        self.pending_iframe_fetches.clear();
        self.iframe_content.clear();

        let css_base_url = parsed.base_url.clone();
        let docment_info = DocumentInfo {
            origin: Origin::from_url(&parsed.document_url),
            document_url: parsed.document_url,
            base_url: parsed.base_url,
            dom: parsed.dom,
            title: parsed.title,
        };
        self.docment_info = Some(docment_info);
        self.snapshot_cache = None;

        for inline_css in &parsed.inline_styles {
            self.queue_css_images(inline_css, &css_base_url);
            let sheet = CssParser::new(inline_css).parse_lossy();
            layouter::css_resolver::append_resolved_styles(
                Arc::make_mut(&mut self.resolved_styles),
                layouter::css_resolver::CssResolver::resolve(&sheet),
            );
            self.resolved_styles_version += 1;
        }
        self.phase = PagePhase::HtmlParsed;
        profile_log!(
            target: "PageLoad",
            log::Level::Info,
            "[HtmlParse] html_parse: {:?} | js_snapshot: {:?}",
            html_parse_time,
            js_snapshot_time,
        );
    }

    pub fn on_css_fetched(&mut self, css: String) {
        let base_url = self
            .docment_info
            .as_ref()
            .map(|info| info.base_url.clone())
            .unwrap_or_else(|| Url::parse("about:blank").expect("valid fallback URL"));
        self.on_css_fetched_from(css, &base_url);
    }

    pub fn on_css_fetched_from(&mut self, css: String, stylesheet_url: &Url) {
        self.queue_css_images(&css, stylesheet_url);
        self.linked_css.push(css.clone());
        match self.css_strategy {
            CssApplicationStrategy::Batch => {
                self.loaded_css.push(css);

                if self.loaded_css.len() == self.pending_css_urls.len() {
                    let all_css = std::mem::take(&mut self.loaded_css);
                    self.css_results_expected = 1;
                    self.css_results_received = 0;
                    self.css_processor.process(all_css);
                    self.phase = PagePhase::CssProcessing;
                }
            }
            CssApplicationStrategy::Incremental => {
                self.css_processor.process(vec![css]);
            }
        }
    }

    /// Decodes a fetched image and rebuilds layout using its intrinsic size.
    pub fn on_image_fetched(&mut self, source: String, bytes: &[u8]) -> anyhow::Result<()> {
        self.image_decoder.decode(source, bytes.to_vec());
        Ok(())
    }

    /// Stores fetched audio bytes for the matching `<audio>` control.
    pub fn on_audio_fetched(&mut self, source: String, bytes: &[u8]) {
        self.audio.insert(source, Arc::from(bytes));
        self.update_layout();
    }

    /// Executes or queues a fetched external classic script by scheduling mode.
    pub fn on_script_fetched(&mut self, index: usize, source: String) {
        let Some(execution) = self.pending_script_fetches.get(&index).copied() else {
            log::warn!("Ignoring unexpected classic script response at index {index}");
            return;
        };

        if execution == ClassicScriptExecution::Default && self.next_script_index != index {
            log::warn!("Ignoring out-of-order blocking script response at index {index}");
            return;
        }
        self.pending_script_fetches.remove(&index);

        match execution {
            ClassicScriptExecution::Default => {
                self.next_script_index += 1;
                self.send_script(&source);
            }
            ClassicScriptExecution::Defer => {
                self.deferred_script_results.insert(index, Some(source));
            }
            ClassicScriptExecution::Async => self.send_script(&source),
        }
    }

    /// Records a failed external classic script without aborting page loading.
    pub fn on_script_fetch_failed(&mut self, index: usize) {
        let Some(execution) = self.pending_script_fetches.get(&index).copied() else {
            log::warn!("Ignoring unexpected classic script failure at index {index}");
            return;
        };

        if execution == ClassicScriptExecution::Default && self.next_script_index != index {
            log::warn!("Ignoring out-of-order blocking script failure at index {index}");
            return;
        }
        self.pending_script_fetches.remove(&index);

        match execution {
            ClassicScriptExecution::Default => self.next_script_index += 1,
            ClassicScriptExecution::Defer => {
                self.deferred_script_results.insert(index, None);
            }
            ClassicScriptExecution::Async => {}
        }
    }

    /// Executes a fetched dynamically inserted script and dispatches `load`.
    pub fn on_dynamic_script_fetched(&mut self, node_id: u64, source: String) {
        self.send_script(&source);
        self.dispatch_js_element_event(node_id, "load");
    }

    /// Dispatches `error` for a dynamically inserted script that failed to load.
    pub fn on_dynamic_script_fetch_failed(&mut self, node_id: u64) {
        self.dispatch_js_element_event(node_id, "error");
    }

    pub fn on_dynamic_style_fetched(&mut self, node_id: u64, source: String) {
        // TODO: Preserve the final stylesheet URL so relative url() and @import resolve correctly.
        self.linked_css.push(source);
        self.rebuild_styles_and_layout();
        self.needs_redraw = true;
        self.dispatch_js_element_event(node_id, "load");
    }

    pub fn on_dynamic_style_fetch_failed(&mut self, node_id: u64) {
        self.dispatch_js_element_event(node_id, "error");
    }

    /// Resolves a JavaScript `fetch()` request with a network response.
    pub fn on_js_fetch_succeeded(&mut self, request_id: u64, response: JsFetchResponse) {
        if let Some(processor) = self.js_processor.as_ref() {
            processor.send(JsTask::ResolveFetch {
                id: request_id,
                response,
            });
            self.pending_js_tasks += 1;
        }
    }

    /// Rejects a JavaScript `fetch()` request after a network failure.
    pub fn on_js_fetch_failed(&mut self, request_id: u64, reason: String) {
        if let Some(processor) = self.js_processor.as_ref() {
            processor.send(JsTask::RejectFetch {
                id: request_id,
                reason,
            });
            self.pending_js_tasks += 1;
        }
    }

    /// Installs parsed iframe HTML as the host element's `contentDocument` and
    /// fires its `load` event.
    pub fn on_iframe_fetched(&mut self, dom_id: u64, html: String) {
        if let Some(processor) = self.js_processor.as_ref() {
            processor.send(JsTask::ResolveIframe { dom_id, html });
            self.pending_js_tasks += 1;
        }
    }

    /// Marks an iframe load as failed so later `contentDocument` accesses do
    /// not keep re-queuing a fetch.
    pub fn on_iframe_fetch_failed(&mut self, dom_id: u64) {
        if let Some(processor) = self.js_processor.as_ref() {
            processor.send(JsTask::RejectIframe { dom_id });
            self.pending_js_tasks += 1;
        }
    }

    /// Settles a DevTools inspection request with its JSON envelope.
    pub fn on_devtools_response(&mut self, id: u64, result: String) {
        if let Some(processor) = self.js_processor.as_ref() {
            processor.send(JsTask::ResolveDevTools { id, result });
            self.pending_js_tasks += 1;
        }
    }

    /// Answers a DevTools inspection query against this page's live state.
    pub(crate) fn inspect(
        &mut self,
        method: &str,
        params: &str,
    ) -> Result<serde_json::Value, String> {
        inspector::handle(self, method, params)
    }

    /// Update page (e.g. DOM changed)
    ///
    /// This is a stub method for now.
    pub fn update_page(&mut self) {
        self.ensure_text_measurer();
        // The caller mutated the DOM (e.g. TreeNode::replace_child), which does
        // not bump Tree::version on its own. Mark the tree dirty so the cached
        // snapshot is rebuilt instead of reused stale.
        if let Some(doc_info) = self.docment_info.as_mut() {
            doc_info.dom.mark_dirty();
        }
        self.update_layout();
        // The JS thread's mirror must reflect the external DOM mutation too.
        self.js_dom_dirty = true;
    }

    fn apply_resolved_styles_and_relayout(
        &mut self,
        resolved: layouter::css_resolver::ResolvedStyles,
    ) {
        layouter::css_resolver::append_resolved_styles(
            Arc::make_mut(&mut self.resolved_styles),
            resolved,
        );
        self.resolved_styles_version += 1;
        self.update_layout();
    }

    fn rebuild_styles_and_layout(&mut self) {
        let Some(document) = self.docment_info.as_ref() else {
            self.update_layout();
            return;
        };
        perf_scope!(resolve_styles);
        let mut resolved = layouter::css_resolver::CssResolver::resolve_with_origin(
            &CssParser::new(USER_AGENT_CSS).parse().unwrap(),
            layouter::css_resolver::StyleOrigin::UserAgent,
        );
        let mut stylesheet_count = 1;
        for source in &self.linked_css {
            let sheet = CssParser::new(source).parse_lossy();
            layouter::css_resolver::append_resolved_styles(
                &mut resolved,
                layouter::css_resolver::CssResolver::resolve(&sheet),
            );
            stylesheet_count += 1;
        }
        for source in document.dom.collect_text_by_tag("style") {
            let sheet = CssParser::new(&source).parse_lossy();
            layouter::css_resolver::append_resolved_styles(
                &mut resolved,
                layouter::css_resolver::CssResolver::resolve(&sheet),
            );
            stylesheet_count += 1;
        }
        #[cfg(any(feature = "profile", debug_assertions))]
        let resolve_styles_time = resolve_styles.elapsed();
        profile_log!(
            target: "PageLoad",
            log::Level::Info,
            "[StyleResolve] stylesheet_resolve: {:?} (sheets: {})",
            resolve_styles_time,
            stylesheet_count,
        );
        self.resolved_styles = Arc::new(resolved);
        self.resolved_styles_version += 1;
        self.update_layout();
    }

    fn try_apply_css_results(&mut self) {
        while let Some(resolved) = self.css_processor.try_receive() {
            self.css_results_received += 1;
            self.apply_resolved_styles_and_relayout(resolved);
            self.needs_redraw = true;

            if self.css_results_received >= self.css_results_expected {
                self.fragment_ready_version = Some(self.layout_requested_version);
                self.phase = PagePhase::CssApplied;
            }
        }
    }

    fn try_apply_batch_result(&mut self) {
        if let Some(resolved) = self.css_processor.try_receive() {
            self.css_results_received += 1;
            self.apply_resolved_styles_and_relayout(resolved);
            self.needs_redraw = true;
            self.fragment_ready_version = Some(self.layout_requested_version);
            self.phase = PagePhase::CssApplied;
        }
    }

    fn advance_classic_scripts(&mut self, tasks: &mut Vec<WebViewTask>) {
        if self.js_policy == JsPolicy::Disabled {
            self.phase = PagePhase::ScriptApplied;
            return;
        }

        self.schedule_non_blocking_scripts(tasks);

        while self.next_script_index < self.classic_scripts.len() {
            match self.classic_scripts[self.next_script_index].clone() {
                ClassicScript::Inline(source) => {
                    self.next_script_index += 1;
                    self.send_script(&source);
                }
                ClassicScript::External { url, execution } => {
                    if execution != ClassicScriptExecution::Default {
                        self.next_script_index += 1;
                        continue;
                    }

                    let index = self.next_script_index;
                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        self.pending_script_fetches.entry(index)
                    {
                        entry.insert(ClassicScriptExecution::Default);
                        tasks.push(WebViewTask::Fetch {
                            url,
                            kind: FetchKind::Script { index },
                        });
                    }
                    return;
                }
            }
        }

        self.advance_deferred_scripts();
    }

    fn schedule_non_blocking_scripts(&mut self, tasks: &mut Vec<WebViewTask>) {
        if self.non_blocking_scripts_scheduled {
            return;
        }
        self.non_blocking_scripts_scheduled = true;

        for (index, script) in self.classic_scripts.iter().enumerate() {
            let ClassicScript::External { url, execution } = script else {
                continue;
            };
            if *execution == ClassicScriptExecution::Default {
                continue;
            }

            self.pending_script_fetches.insert(index, *execution);
            tasks.push(WebViewTask::Fetch {
                url: url.clone(),
                kind: FetchKind::Script { index },
            });
        }
    }

    fn advance_deferred_scripts(&mut self) {
        loop {
            let Some(index) =
                (self.next_deferred_script_index..self.classic_scripts.len()).find(|&index| {
                    matches!(
                        self.classic_scripts.get(index),
                        Some(ClassicScript::External {
                            execution: ClassicScriptExecution::Defer,
                            ..
                        })
                    )
                })
            else {
                self.dispatch_dom_content_loaded();
                self.phase = PagePhase::ScriptApplied;
                return;
            };

            let Some(source) = self.deferred_script_results.remove(&index) else {
                return;
            };
            self.next_deferred_script_index = index + 1;
            if let Some(source) = source {
                self.send_script(&source);
            }
        }
    }

    /// Sends a script to the JS thread for ordered execution.
    fn send_script(&mut self, source: &str) {
        if let Some(processor) = self.js_processor.as_ref() {
            processor.send(JsTask::RunScript {
                source: source.to_string(),
            });
            self.pending_js_tasks += 1;
        }
    }

    fn dispatch_js_element_event(&mut self, dom_id: u64, event_type: &str) {
        if let Some(processor) = self.js_processor.as_ref() {
            processor.send(JsTask::DispatchElementEvent {
                dom_id,
                event_type: event_type.to_string(),
            });
            self.pending_js_tasks += 1;
        }
    }

    fn dispatch_dom_content_loaded(&mut self) {
        if let Some(processor) = self.js_processor.as_ref() {
            processor.send(JsTask::DispatchDomContentLoaded);
            self.pending_js_tasks += 1;
        }
    }

    fn dispatch_window_load(&mut self) {
        if let Some(processor) = self.js_processor.as_ref() {
            processor.send(JsTask::DispatchWindowLoad);
            self.pending_js_tasks += 1;
            self.window_load_dispatched = true;
        }
    }

    /// Whether any JS-visible subresource work still awaits its round trip
    /// (classic/dynamic scripts, stylesheets, images, or `fetch()` requests).
    ///
    /// When this returns `false` the JS thread is idle, so its `mirror` DOM
    /// matches the committed tree and the page can be considered fully loaded.
    fn has_pending_subresource_work(&self) -> bool {
        !self.pending_script_fetches.is_empty()
            || !self.pending_js_fetches.is_empty()
            || !self.pending_iframe_fetches.is_empty()
            || !self.pending_dynamic_scripts.is_empty()
            || !self.pending_dynamic_styles.is_empty()
            || !self.pending_dynamic_images.is_empty()
    }

    fn run_due_js_timers(&mut self) {
        if self.js_dom_dirty {
            // A write-back sync is owed. Pausing pokes keeps a timer snapshot
            // from racing the pending sync; timers resume once it completes.
            return;
        }
        if let Some(processor) = self.js_processor.as_ref() {
            // Timer pokes are coalescable: the JS thread skips this one when a
            // newer task has already been queued. Track the newest poke so the
            // write-back sync waits until its result has been applied.
            let version = processor.send(JsTask::RunTimers);
            self.in_flight_timer_version = Some(version);
        }
    }

    fn schedule_js_fetches(&mut self, tasks: &mut Vec<WebViewTask>) {
        let requests = std::mem::take(&mut self.pending_js_fetches);

        for request in requests {
            match self.resolve_url(&request.url) {
                Ok(url) => tasks.push(WebViewTask::Fetch {
                    url,
                    kind: FetchKind::JavaScript {
                        request_id: request.id,
                        method: request.method,
                        headers: request.headers,
                        body: request.body,
                    },
                }),
                Err(error) => self
                    .on_js_fetch_failed(request.id, format!("Failed to parse fetch URL: {error}")),
            }
        }
    }

    fn schedule_iframe_fetches(&mut self, tasks: &mut Vec<WebViewTask>) {
        let requests = std::mem::take(&mut self.pending_iframe_fetches);

        for request in requests {
            match Url::parse(&request.url) {
                Ok(url) => {
                    log::info!("Iframe fetch requested in WebView: url={}", url);
                    tasks.push(WebViewTask::Fetch {
                        url,
                        kind: FetchKind::Iframe {
                            dom_id: request.dom_id,
                        },
                    })
                }
                Err(error) => {
                    log::warn!("Failed to parse iframe URL: {error}");
                    self.on_iframe_fetch_failed(request.dom_id);
                }
            }
        }
    }

    fn schedule_dynamic_scripts(&mut self, tasks: &mut Vec<WebViewTask>) {
        let requests = std::mem::take(&mut self.pending_dynamic_scripts);

        for request in requests {
            match request.source {
                JsDynamicScriptSource::Inline(source) => {
                    self.send_script(&source);
                    self.dispatch_js_element_event(request.node_id, "load");
                }
                JsDynamicScriptSource::External(source) => match self.resolve_url(&source) {
                    Ok(url) => tasks.push(WebViewTask::Fetch {
                        url,
                        kind: FetchKind::DynamicScript {
                            node_id: request.node_id,
                        },
                    }),
                    Err(error) => {
                        log::warn!("Failed to resolve dynamic script URL: {error}");
                        self.on_dynamic_script_fetch_failed(request.node_id);
                    }
                },
            }
        }
    }

    fn schedule_dynamic_styles(&mut self, tasks: &mut Vec<WebViewTask>) {
        let requests = std::mem::take(&mut self.pending_dynamic_styles);

        for request in requests {
            match self.resolve_url(&request.url) {
                Ok(url) => tasks.push(WebViewTask::Fetch {
                    url,
                    kind: FetchKind::DynamicCss {
                        node_id: request.node_id,
                    },
                }),
                Err(error) => {
                    log::warn!("Failed to resolve dynamic stylesheet URL: {error}");
                    self.on_dynamic_style_fetch_failed(request.node_id);
                }
            }
        }
    }

    fn schedule_dynamic_images(&mut self, tasks: &mut Vec<WebViewTask>) {
        let requests = std::mem::take(&mut self.pending_dynamic_images);

        for request in requests {
            match self.resolve_url(&request.source) {
                Ok(url) => tasks.push(WebViewTask::Fetch {
                    url,
                    kind: FetchKind::Image {
                        source: request.source,
                    },
                }),
                Err(error) => log::warn!("Failed to resolve dynamic image URL: {error}"),
            }
        }
    }

    fn queue_css_images(&mut self, css: &str, base_url: &Url) {
        for source in collect_css_image_sources(css) {
            if self.images.contains_key(&source)
                || self
                    .pending_images
                    .iter()
                    .any(|(pending, _)| pending == &source)
            {
                continue;
            }
            if let Ok(url) = resolve_url(base_url, &source) {
                self.pending_images.push((source, url));
            }
        }
    }

    fn schedule_pending_images(&mut self, tasks: &mut Vec<WebViewTask>) {
        for (source, url) in std::mem::take(&mut self.pending_images) {
            tasks.push(WebViewTask::Fetch {
                url,
                kind: FetchKind::Image { source },
            });
        }
    }

    /// Dispatches a click on the given DOM snapshot node id to the page's JS.
    ///
    /// Resolves the live DOM node behind the snapshot id, translates it to the
    /// JS-facing dom id and hands the click to the JS thread. Returns whether
    /// a redraw is needed; the JS result triggers the relayout once applied.
    pub fn on_js_click(&mut self, dom_id: u32) -> bool {
        let Some(processor) = self.js_processor.as_ref() else {
            return false;
        };
        let Some(node) = self
            .layout_dom_refs
            .get(dom_id as usize)
            .and_then(|weak| weak.upgrade())
        else {
            return false;
        };
        let Some(js_dom_id) = self.js_dom_ids.get(&(Rc::as_ptr(&node) as usize)) else {
            return false;
        };
        processor.send(JsTask::Click { dom_id: *js_dom_id });
        self.pending_js_tasks += 1;
        false
    }

    /// Dispatches a `scroll` event on the given DOM snapshot node id to the
    /// page's JS.
    ///
    /// Resolves the live DOM node behind the snapshot id, translates it to the
    /// JS-facing dom id and hands the scroll to the JS thread. Returns whether
    /// a redraw is needed; the JS result triggers the relayout once applied.
    pub fn on_js_scroll(&mut self, dom_id: u32) -> bool {
        let Some(processor) = self.js_processor.as_ref() else {
            return false;
        };
        let Some(node) = self
            .layout_dom_refs
            .get(dom_id as usize)
            .and_then(|weak| weak.upgrade())
        else {
            return false;
        };
        let Some(js_dom_id) = self.js_dom_ids.get(&(Rc::as_ptr(&node) as usize)) else {
            return false;
        };
        processor.send(JsTask::Scroll { dom_id: *js_dom_id });
        self.pending_js_tasks += 1;
        false
    }

    fn ensure_text_measurer(&mut self) {
        if self.text_measurer.is_none() {
            self.text_measurer = Some(Arc::new(PlatformTextMeasurer::new().unwrap()));
        }
    }

    /// Builds a snapshot and hands the heavy tree construction to the background.
    fn update_layout(&mut self) {
        if self.docment_info.is_none() {
            return;
        }
        self.ensure_text_measurer();

        let doc_info = self.docment_info.as_ref().unwrap();
        let dom_version = doc_info.dom.version();
        // Snapshot construction happens at function scope so the profile log
        // below can read accumulators regardless of which branch ran.
        #[cfg(any(feature = "profile", debug_assertions))]
        let mut snapshot_build_time = std::time::Duration::ZERO;
        #[cfg(any(feature = "profile", debug_assertions))]
        let mut snapshot_cached = false;

        let (snapshot, dom_refs) = if let Some(cache) = &self.snapshot_cache
            // The DOM is unchanged since the last snapshot: reuse it instead of
            // re-cloning the whole tree (CSS/image relayouts dominate).
            && cache.dom_version == dom_version
        {
            #[cfg(any(feature = "profile", debug_assertions))]
            {
                snapshot_cached = true;
            }
            (Arc::clone(&cache.snapshot), cache.dom_refs.clone())
        } else {
            perf_scope!(snapshot_build);
            let (snapshot, dom_refs) = DomSnapshot::from_tree(&doc_info.dom.root);
            #[cfg(any(feature = "profile", debug_assertions))]
            {
                snapshot_build_time = snapshot_build.elapsed();
            }
            let snapshot = Arc::new(snapshot);
            self.snapshot_cache = Some(SnapshotCache {
                dom_version,
                snapshot: Arc::clone(&snapshot),
                dom_refs: dom_refs.clone(),
            });
            (snapshot, dom_refs)
        };
        profile_log!(
            target: "PageLoad",
            log::Level::Info,
            "[DomSnapshot] build: {:?} (cache hit: {})",
            snapshot_build_time,
            snapshot_cached,
        );
        let root = snapshot.roots()[0];

        let media_environment =
            layouter::css_resolver::MediaEnvironment::new(self.viewport, self.system_color_scheme);
        let task = layouter::LayoutTask {
            snapshot,
            root,
            resolved_styles: Arc::clone(&self.resolved_styles),
            media_environment,
            measurer: self.text_measurer.clone().unwrap(),
            system_color_scheme: self.system_color_scheme,
            scripting_mode: self.js_policy.into(),
            images: self.images.clone(),
            audio: self.audio.clone(),
            parent: InheritedCss {
                text_flow_style: TextFlowStyle {
                    font_size: 16.0,
                    ..Default::default()
                },
                ..Default::default()
            },
            chain: ElementChain::default(),
            write_back_sender: Some(self.write_back_tx.clone()),
            styles_version: self.resolved_styles_version,
            version: 0,
        };
        self.layout_dom_refs = dom_refs;
        self.layout_requested_version = self.layout_processor.send(task);
        self.layout_pending = true;
    }

    /// Takes decoded images from the background thread and triggers a relayout.
    fn try_apply_decoded_images(&mut self) {
        while let Some((source, result)) = self.image_decoder.try_receive() {
            match result {
                Ok(image) => {
                    self.images.insert(source, image);
                    self.update_layout();
                }
                Err(error) => {
                    log::warn!("Image decode failed: {error:#}");
                }
            }
        }
    }

    /// Takes completed layout results from the thread and makes them drawable.
    fn try_apply_layout_results(&mut self) {
        while let Some(result) = self.layout_processor.try_receive() {
            let LayoutResult {
                layout,
                mut info,
                version,
            } = result;
            if version < self.layout_requested_version {
                continue;
            }
            // The builder initializes every node's scroll offset to 0, so a
            // rebuilt tree would otherwise drop the scroll position (e.g. the
            // viewport change on a window resize). Re-apply the offsets of the
            // previous tree before swapping the new one in.
            if let Some((_, old_info)) = self.layout_and_info.as_ref() {
                let mut scroll_offsets = HashMap::new();
                capture_scroll_offsets(old_info, &mut scroll_offsets);
                apply_scroll_offsets(&mut info, &scroll_offsets);
            }

            self.layout_and_info = Some((layout, info));
            self.layout_applied_version = version;
            self.layout_pending = false;
            self.needs_redraw = true;
            // The fresh tree has no geometry yet (positioning normally happens
            // during draws). Position it right away so input events arriving
            // before the next redraw still hit-test against real boxes.
            self.position_layout_if_needed();
        }
    }

    /// Takes completed JS results from the thread and commits them.
    ///
    /// A result that mutated the DOM carries a snapshot of the thread's mirror;
    /// committing it replaces the authoritative tree, re-registers the JS dom
    /// id map and triggers a relayout.
    fn try_apply_js_results(&mut self) {
        let results: Vec<JsTaskResult> = {
            let Some(processor) = self.js_processor.as_ref() else {
                return;
            };
            let mut results = Vec::new();
            while let Some(result) = processor.try_receive() {
                results.push(result);
            }
            results
        };
        for result in results {
            self.pending_js_tasks = self.pending_js_tasks.saturating_sub(1);
            self.pending_js_fetches.extend(result.fetch_requests);
            self.pending_devtools_requests
                .extend(result.devtools_requests);
            self.pending_dynamic_scripts
                .extend(result.dynamic_script_requests);
            self.pending_dynamic_styles
                .extend(result.dynamic_style_requests);
            self.pending_dynamic_images
                .extend(result.dynamic_image_requests);
            self.pending_iframe_fetches
                .extend(result.iframe_fetch_requests);

            if let Some(in_flight) = self.in_flight_timer_version
                && result.version >= in_flight
            {
                // The newest timer poke has been processed (run or superseded),
                // so its snapshot can no longer race the write-back sync.
                self.in_flight_timer_version = None;
            }

            let Some(snapshot) = result.dom else {
                continue;
            };
            let Some(info) = self.docment_info.as_mut() else {
                continue;
            };
            // Commit the thread's mirror as the new authoritative tree. The
            // rebuilt tree starts with a fresh version, so the cached snapshot
            // and live layout references are stale and must be dropped.
            let (tree, dom_ids) = snapshot.into_tree();
            // Retain only iframe content for iframes still present; stale
            // entries from removed iframes are dropped on the next nav anyway.
            self.iframe_content.clear();
            for iframe_doc in result.iframe_documents {
                self.iframe_content
                    .insert(iframe_doc.iframe_dom_id, iframe_doc.content);
            }
            info.dom = Rc::new(tree);
            self.js_dom_ids = dom_ids;
            // Splice committed iframe content under the host <iframe> nodes so
            // layout/paint render it nested.
            graft_iframe_documents(&info.dom, &self.js_dom_ids, &self.iframe_content);
            self.snapshot_cache = None;
            self.layout_dom_refs.clear();

            self.rebuild_styles_and_layout();
            self.needs_redraw = true;
        }
    }

    /// Syncs UI-side DOM mutations (write-backs, `update_page`) to the JS thread.
    ///
    /// Only runs when no JS task is in flight: mid-task the thread's mirror
    /// legitimately diverges from the real tree, and committing an in-flight
    /// snapshot after this sync would clobber the thread's newer mutations.
    /// An in-flight `RunTimers` poke counts as in flight for the same reason:
    /// its callback may mutate the mirror and commit a stale snapshot.
    fn sync_dom_to_worker(&mut self) {
        if !self.js_dom_dirty
            || self.pending_js_tasks != 0
            || self.in_flight_timer_version.is_some()
        {
            return;
        }
        let Some(processor) = self.js_processor.as_ref() else {
            return;
        };
        let Some(doc_info) = self.docment_info.as_ref() else {
            return;
        };
        let snapshot = DomSnapshot::from_mirror(&doc_info.dom.root, &self.js_dom_ids);
        processor.send(JsTask::UpdateDom { snapshot });
        self.pending_js_tasks += 1;
        self.js_dom_dirty = false;
    }

    /// Applies value write-backs reported by text inputs to the live DOM.
    fn drain_write_backs(&mut self) {
        let mut applied = false;
        while let Ok((node_id, value)) = self.write_back_rx.try_recv() {
            if let Some(weak) = self.layout_dom_refs.get(node_id as usize)
                && let Some(node) = weak.upgrade()
            {
                node.borrow_mut().value.set_attr("value", value);
                applied = true;
            }
            self.needs_redraw = true;
        }

        // The DOM mutated, so the cached snapshot is stale and must be rebuilt
        // on the next relayout. Future JS mutations must also bump the version.
        if applied && let Some(doc_info) = self.docment_info.as_mut() {
            doc_info.dom.mark_dirty();
        }
        // The JS thread's mirror must reflect the new input value; synced once
        // the in-flight JS tasks have all been applied.
        if applied {
            self.js_dom_dirty = true;
        }
    }

    pub fn navigate(&mut self) {
        self.reset_for_navigation();
    }

    fn reset_for_navigation(&mut self) {
        if self.phase != PagePhase::Init {
            self.phase = PagePhase::BeforeHtmlParsing;
        }

        self.docment_info = None;
        self.pending_css_urls.clear();
        self.pending_images.clear();
        self.pending_audio.clear();
        self.loaded_css.clear();
        self.linked_css.clear();
        self.images.clear();
        self.audio.clear();
        Arc::make_mut(&mut self.resolved_styles).clear();
        self.layout_and_info = None;

        self.needs_redraw = false;

        self.css_processor = css::processor::CssProcessor::new();
        self.css_results_expected = 0;
        self.css_results_received = 0;

        self.layout_processor = layouter::LayoutProcessor::new();
        self.layout_pending = false;
        self.layout_requested_version = 0;
        self.layout_applied_version = 0;
        self.positioned_layout = None;
        self.layout_dom_refs.clear();
        self.snapshot_cache = None;
        self.js_processor = None;
        self.js_dom_ids.clear();
        self.pending_js_tasks = 0;
        self.js_dom_dirty = false;
        self.in_flight_timer_version = None;
        self.pending_js_fetches.clear();
        self.pending_devtools_requests.clear();
        self.inspector_ids.borrow_mut().clear();
        self.pending_dynamic_scripts.clear();
        self.pending_dynamic_styles.clear();
        self.pending_dynamic_images.clear();
        self.pending_iframe_fetches.clear();
        self.iframe_content.clear();
        self.classic_scripts.clear();
        self.next_script_index = 0;
        self.pending_script_fetches.clear();
        self.non_blocking_scripts_scheduled = false;
        self.deferred_script_results.clear();
        self.next_deferred_script_index = 0;
        self.pending_fragment_scroll = None;
        self.fragment_ready_version = None;
        let (write_back_tx, write_back_rx) = mpsc::channel();
        self.write_back_tx = write_back_tx;
        self.write_back_rx = write_back_rx;
    }

    pub fn set_system_color_scheme(&mut self, scheme: ColorScheme) {
        if self.system_color_scheme == scheme {
            return;
        }
        self.system_color_scheme = scheme;
        self.update_layout();
    }

    pub fn title(&self) -> Option<&String> {
        self.docment_info.as_ref().map(|d| &d.title)
    }

    /// Runs the box-positioning pass over the current layout tree unless it
    /// has already been positioned for the current version + viewport.
    ///
    /// Background layout results arrive unpositioned (geometry is computed on
    /// the main thread), so this must run before the tree is used for anything
    /// geometry-sensitive — drawing, but crucially also hit-testing. Without
    /// the eager call in [`WebView::try_apply_layout_results`], a click landing
    /// between a result being applied and the next draw would walk boxes with
    /// no geometry and find nothing.
    fn position_layout_if_needed(&mut self) {
        let viewport = self.viewport;
        if self.positioned_layout == Some((self.layout_applied_version, viewport)) {
            return;
        }
        let Some((layout, info)) = self.layout_and_info.as_mut() else {
            return;
        };

        ui_layout::LayoutEngine::layout(layout, viewport.0, viewport.1);
        if layouter::constrain_auto_grid_track_items(layout) {
            ui_layout::LayoutEngine::layout(layout, viewport.0, viewport.1);
        }
        layouter::correct_atomic_inline_spacing_with_info(layout, info);
        layouter::align_table_columns(layout, info);
        layouter::refresh_missing_text_layout_results(layout, info, viewport);

        self.positioned_layout = Some((self.layout_applied_version, viewport));
    }

    pub fn relayout(&mut self, viewport: (f32, f32)) {
        if self.viewport != viewport {
            self.viewport = viewport;
            if let Some(processor) = self.js_processor.as_ref() {
                processor.send(JsTask::SetViewport {
                    width: viewport.0,
                    height: viewport.1,
                });
                self.pending_js_tasks += 1;
            }
            self.update_layout();
        }
        self.position_layout_if_needed();

        let fragment_target =
            if fragment_layout_is_ready(self.fragment_ready_version, self.layout_applied_version) {
                self.pending_fragment_scroll
                    .as_deref()
                    .and_then(|fragment| {
                        find_fragment_target_dom_id(&self.layout_dom_refs, fragment)
                    })
            } else {
                None
            };

        let Some((layout, info)) = self.layout_and_info.as_mut() else {
            return;
        };

        if fragment_target
            .is_some_and(|target| apply_fragment_scroll(layout, info, target, viewport.1))
        {
            self.pending_fragment_scroll = None;
            self.needs_redraw = true;
        }

        let layout_metrics =
            collect_js_layout_metrics(layout, info, &self.layout_dom_refs, &self.js_dom_ids);
        if let Some(processor) = self.js_processor.as_ref() {
            processor.send(JsTask::SetLayoutMetrics {
                metrics: layout_metrics,
            });
            self.pending_js_tasks += 1;
        }
    }

    /// 現在描画可能な Layout / Info を返す（なければ None）
    pub fn layout_and_info(&self) -> Option<(&LayoutNode, &InfoNode)> {
        self.layout_and_info.as_ref().map(|(l, i)| (l, i))
    }

    pub fn layout_and_info_mut(&mut self) -> Option<(&LayoutNode, &mut InfoNode)> {
        self.layout_and_info.as_mut().map(|(l, i)| (&*l, i))
    }

    /// Returns document info
    pub fn document_info(&self) -> Option<&DocumentInfo> {
        self.docment_info.as_ref()
    }

    pub fn document_url(&self) -> Option<&Url> {
        self.docment_info.as_ref().map(|info| &info.document_url)
    }

    pub fn base_url(&self) -> Option<&Url> {
        self.docment_info.as_ref().map(|info| &info.base_url)
    }

    pub fn needs_redraw(&self) -> bool {
        self.needs_redraw
            || self
                .layout_and_info
                .as_ref()
                .is_some_and(|(_, info)| crate::engine::input::any_custom_node_needs_repaint(info))
    }

    pub fn clear_redraw_flag(&mut self) {
        self.needs_redraw = false;
    }

    fn resolve_url(&self, url: &str) -> Result<Url, url::ParseError> {
        let base = self
            .docment_info
            .as_ref()
            .map(|info| &info.base_url)
            .ok_or(url::ParseError::RelativeUrlWithoutBase)?;

        Url::parse(url).or_else(|_| base.join(url))
    }
}

/// Builds the geometry snapshot used by DOM measurement APIs from the same
/// layout boxes and scroll offsets consumed by painting and hit testing.
fn collect_js_layout_metrics(
    layout: &LayoutNode,
    info: &InfoNode,
    dom_refs: &[Weak<RefCell<TreeNode<HtmlNodeType>>>],
    js_dom_ids: &HashMap<usize, u64>,
) -> HashMap<u64, JsLayoutMetrics> {
    let mut metrics = HashMap::new();
    collect_js_layout_metrics_inner(
        layout,
        info,
        dom_refs,
        js_dom_ids,
        (0.0, 0.0),
        (0.0, 0.0),
        &mut metrics,
    );
    metrics
}

fn collect_js_layout_metrics_inner(
    layout: &LayoutNode,
    info: &InfoNode,
    dom_refs: &[Weak<RefCell<TreeNode<HtmlNodeType>>>],
    js_dom_ids: &HashMap<usize, u64>,
    parent_content_origin: (f32, f32),
    inherited_scroll: (f32, f32),
    metrics: &mut HashMap<u64, JsLayoutMetrics>,
) {
    let is_fixed = layout.style.position.kind == ui_layout::Position::Fixed;
    let effective_scroll = if is_fixed {
        (0.0, 0.0)
    } else {
        inherited_scroll
    };
    let own_scroll = info.kind.scroll_offsets();
    let child_scroll = if is_fixed {
        own_scroll
    } else {
        (
            inherited_scroll.0 + own_scroll.0,
            inherited_scroll.1 + own_scroll.1,
        )
    };

    let boxes: Vec<_> = layout.layout_box.iter().collect();
    if let Some(first) = boxes.first() {
        let mut page_left = parent_content_origin.0 + first.border_box.x;
        let mut page_top = parent_content_origin.1 + first.border_box.y;
        let mut page_right = page_left + first.border_box.width;
        let mut page_bottom = page_top + first.border_box.height;
        for model in boxes.iter().skip(1) {
            let left = parent_content_origin.0 + model.border_box.x;
            let top = parent_content_origin.1 + model.border_box.y;
            page_left = page_left.min(left);
            page_top = page_top.min(top);
            page_right = page_right.max(left + model.border_box.width);
            page_bottom = page_bottom.max(top + model.border_box.height);
        }

        if let Some(node) = info
            .dom_id
            .and_then(|id| dom_refs.get(id as usize))
            .and_then(Weak::upgrade)
        {
            let node_key = Rc::as_ptr(&node) as usize;
            if let Some(dom_id) = js_dom_ids.get(&node_key).copied() {
                metrics.insert(
                    dom_id,
                    JsLayoutMetrics {
                        offset_left: first.border_box.x as f64,
                        offset_top: first.border_box.y as f64,
                        offset_width: (page_right - page_left) as f64,
                        offset_height: (page_bottom - page_top) as f64,
                        client_width: first.padding_box.width as f64,
                        client_height: first.padding_box.height as f64,
                        rect_left: (page_left - effective_scroll.0) as f64,
                        rect_top: (page_top - effective_scroll.1) as f64,
                        rect_width: (page_right - page_left) as f64,
                        rect_height: (page_bottom - page_top) as f64,
                    },
                );
            }
        }

        // TODO: Apply CSS transforms and sticky-position paint offsets to DOMRect geometry.
        let child_origin = (
            parent_content_origin.0 + first.content_box.x,
            parent_content_origin.1 + first.content_box.y,
        );
        for (child_layout, child_info) in layout.children.iter().zip(&info.children) {
            if let Some(child_layout) = child_layout.node() {
                collect_js_layout_metrics_inner(
                    child_layout,
                    child_info,
                    dom_refs,
                    js_dom_ids,
                    child_origin,
                    child_scroll,
                    metrics,
                );
            }
        }
    }
}

/// Records the nonzero scroll offsets of every scrollable node in `info`,
/// keyed by the node's DOM snapshot id.
///
/// The layout builder initializes each node's `scroll_offset` to 0, so a
/// rebuild would otherwise drop the scroll position (e.g. after a window
/// resize). `dom_id` stays stable across rebuilds while the DOM is unchanged,
/// which makes it a reliable key for restoring state onto the new tree.
fn capture_scroll_offsets(info: &InfoNode, offsets: &mut HashMap<NodeId, (f32, f32)>) {
    let (x, y) = info.kind.scroll_offsets();
    if (x != 0.0 || y != 0.0)
        && let Some(dom_id) = info.dom_id
    {
        offsets.insert(dom_id, (x, y));
    }
    for child in &info.children {
        capture_scroll_offsets(child, offsets);
    }
}

/// Restores scroll offsets captured by [`capture_scroll_offsets`] onto a newly
/// built tree.
///
/// Offsets are copied verbatim for every matching node regardless of its
/// `scroll_x`/`scroll_y` flags: the flags describe whether an axis *can*
/// scroll, not whether a scroll position was captured, so gating on them here
/// would drop positions (e.g. the viewport/page scroll carried by the root).
fn apply_scroll_offsets(info: &mut InfoNode, offsets: &HashMap<NodeId, (f32, f32)>) {
    if let Some((x, y)) = info.dom_id.and_then(|id| offsets.get(&id)) {
        match &mut info.kind {
            NodeKind::Container {
                scroll_offset_x,
                scroll_offset_y,
                ..
            }
            | NodeKind::Custom {
                scroll_offset_x,
                scroll_offset_y,
                ..
            } => {
                *scroll_offset_x = *x;
                *scroll_offset_y = *y;
            }
            _ => {}
        }
    }
    for child in &mut info.children {
        apply_scroll_offsets(child, offsets);
    }
}

fn fragment_layout_is_ready(ready_version: Option<u64>, applied_version: u64) -> bool {
    ready_version.is_some_and(|ready_version| applied_version >= ready_version)
}

fn find_fragment_target_dom_id(
    dom_refs: &[Weak<RefCell<TreeNode<HtmlNodeType>>>],
    fragment: &str,
) -> Option<NodeId> {
    dom_refs.iter().enumerate().find_map(|(index, node)| {
        let node = node.upgrade()?;
        (node.borrow().value.get_attr("id") == Some(fragment)).then_some(index as NodeId)
    })
}

fn apply_fragment_scroll(
    layout: &LayoutNode,
    info: &mut InfoNode,
    target: NodeId,
    viewport_height: f32,
) -> bool {
    let Some(target_y) = fragment_target_y(layout, info, target, 0.0) else {
        return false;
    };
    set_first_vertical_scroll_offset(layout, info, target_y, viewport_height)
}

fn fragment_target_y(
    layout: &LayoutNode,
    info: &InfoNode,
    target: NodeId,
    parent_content_y: f32,
) -> Option<f32> {
    let model = layout.layout_box.iter().next();
    if info.dom_id == Some(target) {
        return model.map(|model| parent_content_y + model.border_box.y);
    }
    let child_content_y =
        parent_content_y + model.as_ref().map_or(0.0, |model| model.content_box.y);
    layout
        .children
        .iter()
        .zip(&info.children)
        .find_map(|(layout_child, info_child)| {
            let LayoutChild::Node(layout_child) = layout_child else {
                return None;
            };
            fragment_target_y(layout_child, info_child, target, child_content_y)
        })
}

fn set_first_vertical_scroll_offset(
    layout: &LayoutNode,
    info: &mut InfoNode,
    target_y: f32,
    viewport_height: f32,
) -> bool {
    if let Some(model) = layout.layout_box.iter().next() {
        let offset = match &mut info.kind {
            NodeKind::Container {
                scroll_y: true,
                scroll_offset_y,
                ..
            }
            | NodeKind::Custom {
                scroll_y: true,
                scroll_offset_y,
                ..
            } => Some(scroll_offset_y),
            _ => None,
        };
        if let Some(offset) = offset {
            let max_scroll = (model.children_box.height
                - model.content_box.height.min(viewport_height))
            .max(0.0);
            *offset = target_y.clamp(0.0, max_scroll);
            return true;
        }
    }

    layout
        .children
        .iter()
        .zip(&mut info.children)
        .any(|(layout_child, info_child)| {
            let LayoutChild::Node(layout_child) = layout_child else {
                return false;
            };
            set_first_vertical_scroll_offset(layout_child, info_child, target_y, viewport_height)
        })
}

fn collect_css_image_sources(css: &str) -> Vec<String> {
    fn collect_value(value: &CssValue, sources: &mut Vec<String>) {
        match value {
            CssValue::Function(name, arguments) if name.eq_ignore_ascii_case("url") => {
                if let Some(source) = arguments
                    .iter()
                    .flatten()
                    .find_map(|argument| match argument {
                        CssValue::String(source) => Some(source.clone()),
                        CssValue::Keyword(source) => Some(source.to_string()),
                        _ => None,
                    })
                    && !source.is_empty()
                    && !sources.contains(&source)
                {
                    sources.push(source);
                }
            }
            CssValue::Function(_, arguments) => {
                for argument in arguments.iter().flatten() {
                    collect_value(argument, sources);
                }
            }
            CssValue::List(arguments) => {
                for argument in arguments {
                    collect_value(argument, sources);
                }
            }
            _ => {}
        }
    }

    fn visit(node: &CssNode, sources: &mut Vec<String>) {
        if let CssNodeType::Declaration { name, value } = node.node()
            && matches!(
                name.to_ascii_lowercase().as_str(),
                "background" | "background-image"
            )
        {
            collect_value(value, sources);
        }
        for child in node.children() {
            visit(child, sources);
        }
    }

    let stylesheet = CssParser::new(css).parse_lossy();
    let mut sources = Vec::new();
    visit(&stylesheet, &mut sources);
    sources
}

fn parse_html(html: &str, document_url: Url, scripting_mode: ScriptingMode) -> ParsedDocument {
    // --- DOM パース ---
    let mut parser = HtmlParser::new(html).with_scripting_mode(scripting_mode);
    let dom = Rc::new(parser.parse());

    // --- base_url ---
    let base_url = dom
        .find_all(|n| n.tag_name() == Some("base"))
        .iter()
        .filter_map(|node_ref| {
            let html_node = &node_ref.borrow().value;
            let href = html_node.get_attr("href")?;
            document_url.join(href).ok()
        })
        .next()
        .unwrap_or_else(|| document_url.clone());

    // --- title 抽出 ---
    let title = dom
        .collect_text_by_tag("title")
        .first()
        .cloned()
        .unwrap_or("".into());

    // --- Style links ---
    // <link rel="stylesheet" href="...">
    let link_nodes = dom.find_all(|n| n.tag_name() == Some("link"));
    let mut style_links = Vec::new();

    for node in link_nodes {
        let (rel, href) = {
            let node_ref = node.borrow();
            let html_node = &node_ref.value;

            let rel = html_node.get_attr("rel").map(|s| s.to_string());
            let href = html_node.get_attr("href").map(|s| s.to_string());
            (rel, href)
        };

        if let (Some(rel), Some(href)) = (rel, href)
            && rel == "stylesheet"
        {
            let css_url = match resolve_url(&base_url, &href) {
                Ok(url) => url,
                Err(_) => continue,
            };
            style_links.push(css_url);
        }
    }

    // --- Inline styles ---
    let inline_styles = dom.collect_text_by_tag("style");

    // --- Classic scripts ---
    let scripts = dom
        .collect_classic_script_descriptors()
        .into_iter()
        .filter_map(|script| match script.source {
            ClassicScriptSource::Inline(source) => Some(ClassicScript::Inline(source)),
            ClassicScriptSource::External(source) => {
                resolve_url(&base_url, &source)
                    .ok()
                    .map(|url| ClassicScript::External {
                        url,
                        execution: script.execution,
                    })
            }
        })
        .collect();

    let image_sources = dom
        .get_elements_by_tag_name("img")
        .into_iter()
        .filter_map(|node| {
            let source = node.borrow().value.get_attr("src")?.to_string();
            let url = resolve_url(&base_url, &source).ok()?;
            Some((source, url))
        })
        .collect();

    let audio_sources = dom
        .get_elements_by_tag_name("audio")
        .into_iter()
        .filter_map(|node| {
            let source = {
                let audio = node.borrow();
                audio.value.get_attr("src").map(str::to_string).or_else(|| {
                    audio.children().iter().find_map(|child| {
                        let child = child.borrow();
                        (child.value.tag_name() == Some("source"))
                            .then(|| child.value.get_attr("src").map(str::to_string))
                            .flatten()
                    })
                })
            }?;
            let url = resolve_url(&base_url, &source).ok()?;
            Some((source, url))
        })
        .collect();

    ParsedDocument {
        document_url,
        base_url,
        dom,
        title,
        style_links,
        inline_styles,
        image_sources,
        audio_sources,
        scripts,
    }
}

pub fn resolve_url(base_url: &Url, path: &str) -> Result<Url, url::ParseError> {
    // absolute URL（scheme を持つ）
    if let Ok(url) = Url::parse(path) {
        return Ok(url);
    }

    // relative URL
    base_url.join(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::layouter::types::{ContainerRole, ContainerStyle};
    use serde_json::{Value, json};
    use std::time::{Duration, Instant};
    use ui_layout::{Length, LengthOrAuto};

    /// Drives `tick()` until `done` holds, or panics after a timeout.
    ///
    /// JS runs on a background thread, so effects (DOM commits, relayouts) are
    /// observed a few ticks after the task that produced them was sent.
    fn pump_until(webview: &mut WebView, mut done: impl FnMut(&mut WebView) -> bool, why: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done(webview) {
            assert!(Instant::now() < deadline, "timed out waiting for {why}");
            webview.tick();
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Drives `tick()` collecting tasks until `done` accepts one, then returns it.
    fn pump_for_task(
        webview: &mut WebView,
        mut done: impl FnMut(&WebViewTask) -> bool,
        why: &str,
    ) -> WebViewTask {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let tasks = webview.tick();
            if let Some(task) = tasks.into_iter().find(&mut done) {
                return task;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {why}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn find_json_by_attribute<'a>(value: &'a Value, name: &str, wanted: &str) -> Option<&'a Value> {
        if value
            .get("attributes")
            .and_then(Value::as_array)
            .is_some_and(|attrs| {
                attrs
                    .iter()
                    .any(|attr| attr[0] == *name && attr[1] == *wanted)
            })
        {
            return Some(value);
        }
        value
            .get("children")
            .and_then(Value::as_array)
            .and_then(|children| {
                children
                    .iter()
                    .find_map(|child| find_json_by_attribute(child, name, wanted))
            })
    }

    fn styles_webview() -> WebView {
        let mut webview = WebView::default();
        // The Init phase injects the user-agent stylesheet into resolved_styles.
        webview.tick();
        webview.on_html_fetched(
            r#"<html><head><style>
                    p { color: red; }
                    .box { color: blue; }
               </style></head>
               <body><p id="t" class="box" style="margin-top: 7px">x</p></body></html>"#
                .to_string(),
            Url::parse("https://example.test/").unwrap(),
        );
        // Resolve the inline <style> block and rebuild snapshot + layout inputs.
        webview.rebuild_styles_and_layout();
        webview
    }

    fn box_model_webview() -> WebView {
        let mut webview = WebView::default();
        webview.tick();
        webview.on_html_fetched(
            r#"<html><head><style>
                    #box {
                        width: 100px;
                        height: 50px;
                        padding: 10px;
                        border: 2px solid red;
                        margin-top: 7px;
                    }
               </style></head>
               <body><div id="box">x</div></body></html>"#
                .to_string(),
            Url::parse("https://example.test/box").unwrap(),
        );
        webview.rebuild_styles_and_layout();
        // The heavy tree build runs on the background thread; wait for it.
        pump_until(
            &mut webview,
            |webview| webview.layout_and_info.is_some(),
            "the first background layout build",
        );
        webview
    }

    fn dom_id_for_attribute(webview: &mut WebView, name: &str, wanted: &str) -> u64 {
        let document = webview.inspect("getDocument", "{}").expect("document");
        find_json_by_attribute(&document, name, wanted).map_or_else(
            || panic!("no element with {name}={wanted}"),
            |node| node["id"].as_u64().unwrap(),
        )
    }

    #[test]
    fn box_model_reports_rings_from_laid_out_geometry() {
        let mut webview = box_model_webview();
        let dom_id = dom_id_for_attribute(&mut webview, "id", "box");
        let params = format!(r#"{{"domId":{dom_id}}}"#);

        let model = webview.inspect("getBoxModel", &params).expect("box model")["model"].clone();

        // Declared margins come through as text, auto stays readable.
        assert_eq!(model["margin"][0], "7");
        assert_eq!(
            model["padding"],
            json!([10.0, 10.0, 10.0, 10.0]),
            "padding ring derives from padding vs content boxes"
        );
        assert_eq!(model["border"], json!([2.0, 2.0, 2.0, 2.0]));
        // Default box-sizing is content-box: content keeps the declared size.
        assert_eq!(model["content"], json!([100.0, 50.0]));
        assert_eq!(model["size"], json!([124.0, 74.0]));

        let info = webview
            .inspect("getLayoutInfo", &params)
            .expect("layout info")["info"]
            .clone();
        assert_eq!(info["width"], "100");
        assert_eq!(info["height"], "50");
        assert_eq!(info["scroll"], json!([0.0, 0.0]));
    }

    #[test]
    fn box_model_rejects_ids_outside_the_current_layout() {
        let mut webview = box_model_webview();
        let error = webview
            .inspect("getBoxModel", r#"{"domId":99999}"#)
            .expect_err("unknown id must fail");
        assert!(error.contains("unknown domId"), "unexpected error: {error}");
    }

    #[test]
    fn applied_layout_results_are_positioned_before_any_draw() {
        // Regression: background layout results replaced the tree unpositioned
        // (geometry was only computed during draws), so a click landing between
        // an application and the next redraw hit-tested against boxes without
        // geometry and found nothing.
        let webview = box_model_webview();
        let (layout, info) = webview.layout_and_info().expect("layout applied");

        let path = crate::engine::input::hit_test(layout, info, 50.0, 25.0);
        assert!(
            crate::engine::input::hit_dom_id(&path).is_some(),
            "boxes must carry geometry as soon as a background result lands"
        );
    }

    #[test]
    fn matched_rules_report_winners_overrides_and_inline_styles() {
        let mut webview = styles_webview();

        let document = webview.inspect("getDocument", "{}").expect("document");
        let paragraph = find_json_by_attribute(&document, "id", "t").expect("<p id=t>");
        let dom_id = paragraph["id"].as_u64().unwrap();

        let rules = webview
            .inspect("getMatchedRules", &format!(r#"{{"domId":{dom_id}}}"#))
            .expect("matched rules");
        let rules = rules["rules"].as_array().unwrap();

        let inline = rules
            .iter()
            .find(|rule| rule["inline"] == Value::Bool(true))
            .expect("inline entry");
        assert_eq!(inline["selector"], "element.style");
        assert_eq!(inline["declarations"][0]["name"], "margin-top");
        assert_eq!(inline["declarations"][0]["value"], "7px");
        assert_eq!(inline["declarations"][0]["applied"], true);

        let class_rule = rules
            .iter()
            .find(|rule| rule["selector"] == ".box")
            .expect(".box rule");
        assert_eq!(class_rule["origin"], "author");
        assert_eq!(class_rule["declarations"][0]["applied"], true);

        // The user-agent sheet also styles `p`; pick the author rule and
        // check its color declaration specifically.
        let tag_rule = rules
            .iter()
            .find(|rule| rule["selector"] == "p" && rule["origin"] == "author")
            .expect("author p rule");
        let color = tag_rule["declarations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|declaration| declaration["name"] == "color")
            .unwrap();
        assert_eq!(color["applied"], false, ".box must override the p color");

        // User-agent rules participate in the report too.
        assert!(
            rules.iter().any(|rule| rule["origin"] == "user-agent"),
            "user-agent origin rules must be reported"
        );
    }

    #[test]
    fn computed_style_lists_winning_declarations_sorted_by_name() {
        let mut webview = styles_webview();

        let document = webview.inspect("getDocument", "{}").expect("document");
        let paragraph = find_json_by_attribute(&document, "id", "t").expect("<p id=t>");
        let dom_id = paragraph["id"].as_u64().unwrap();

        let computed = webview
            .inspect("getComputedStyle", &format!(r#"{{"domId":{dom_id}}}"#))
            .expect("computed style");
        let properties = computed["properties"].as_array().unwrap();

        let names: Vec<&str> = properties
            .iter()
            .map(|property| property["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, {
            let mut sorted = names.clone();
            sorted.sort();
            sorted
        });

        let winner = |name: &str| {
            properties
                .iter()
                .find(|property| property["name"] == *name)
                .map(|property| property["value"].as_str().unwrap().to_string())
        };
        assert_eq!(winner("color").as_deref(), Some("blue"));
        assert_eq!(winner("margin-top").as_deref(), Some("7px"));
    }

    #[test]
    fn collects_background_images_without_treating_fonts_as_page_images() {
        let sources = collect_css_image_sources(
            r#"
            @font-face { src: url("/fonts/scratch.woff2"); }
            .logo { background: white url('/images/logo.svg') no-repeat center; }
            .hero { background-image: url("../images/hero.png"); }
            "#,
        );
        assert_eq!(
            sources,
            vec![
                "/images/logo.svg".to_string(),
                "../images/hero.png".to_string()
            ]
        );
    }

    fn scrollable_info(dom_id: Option<NodeId>, scroll_y: bool, offset_y: f32) -> InfoNode {
        InfoNode {
            kind: NodeKind::Container {
                scroll_x: false,
                scroll_y,
                scroll_offset_x: 0.0,
                scroll_offset_y: offset_y,
                style: ContainerStyle::default(),
                role: ContainerRole::Normal,
            },
            children: Vec::new(),
            dom_id,
        }
    }

    fn scroll_offsets(info: &InfoNode) -> (f32, f32) {
        info.kind.scroll_offsets()
    }

    fn set_root_scroll_offset(info: &mut InfoNode, offset: f32) {
        match &mut info.kind {
            NodeKind::Container {
                scroll_offset_y, ..
            }
            | NodeKind::Custom {
                scroll_offset_y, ..
            } => *scroll_offset_y = offset,
            _ => panic!("expected a container root"),
        }
    }

    fn root_scroll_offset(info: &InfoNode) -> Option<f32> {
        match &info.kind {
            NodeKind::Container {
                scroll_offset_y, ..
            }
            | NodeKind::Custom {
                scroll_offset_y, ..
            } => Some(*scroll_offset_y),
            _ => None,
        }
    }

    fn find_scrollable_offset(info: &InfoNode) -> Option<f32> {
        if let NodeKind::Container {
            scroll_y: true,
            scroll_offset_y,
            ..
        } = &info.kind
        {
            return Some(*scroll_offset_y);
        }
        info.children.iter().find_map(find_scrollable_offset)
    }

    fn set_first_scrollable_offset(info: &mut InfoNode, offset: f32) -> bool {
        if let NodeKind::Container {
            scroll_y: true,
            scroll_offset_y,
            ..
        } = &mut info.kind
        {
            *scroll_offset_y = offset;
            return true;
        }
        info.children
            .iter_mut()
            .any(|c| set_first_scrollable_offset(c, offset))
    }

    fn layout_box(y: f32, height: f32, children_height: f32) -> ui_layout::LayoutBox {
        let rect = |y, height| ui_layout::Rect {
            x: 0.0,
            y,
            width: 800.0,
            height,
        };
        ui_layout::LayoutBox::BlockBox(ui_layout::BoxModel {
            sticky_edges: None,
            border_box: rect(y, height),
            padding_box: rect(y, height),
            content_box: rect(y, height),
            children_box: rect(y, children_height),
        })
    }

    #[test]
    fn dom_layout_metrics_follow_box_geometry_and_scroll_offsets() {
        let mut parser = HtmlParser::new(r#"<div id="target"></div>"#);
        let dom = Rc::new(parser.parse());
        let target = dom.get_element_by_id("target").unwrap();
        let dom_refs = vec![Rc::downgrade(&target)];

        let mut child = LayoutNode::new(ui_layout::Style::default());
        child.layout_box = ui_layout::LayoutBox::BlockBox(ui_layout::BoxModel {
            sticky_edges: None,
            border_box: ui_layout::Rect {
                x: 30.0,
                y: 40.0,
                width: 120.0,
                height: 80.0,
            },
            padding_box: ui_layout::Rect {
                x: 32.0,
                y: 42.0,
                width: 116.0,
                height: 76.0,
            },
            content_box: ui_layout::Rect {
                x: 36.0,
                y: 46.0,
                width: 108.0,
                height: 68.0,
            },
            children_box: ui_layout::Rect::default(),
        });
        let mut root = LayoutNode::with_children(ui_layout::Style::default(), [child]);
        root.layout_box = ui_layout::LayoutBox::BlockBox(ui_layout::BoxModel {
            sticky_edges: None,
            border_box: ui_layout::Rect {
                width: 800.0,
                height: 600.0,
                ..Default::default()
            },
            padding_box: ui_layout::Rect {
                width: 800.0,
                height: 600.0,
                ..Default::default()
            },
            content_box: ui_layout::Rect {
                x: 10.0,
                y: 20.0,
                width: 780.0,
                height: 580.0,
            },
            children_box: ui_layout::Rect::default(),
        });

        let mut root_info = scrollable_info(None, true, 7.0);
        if let NodeKind::Container {
            scroll_offset_x, ..
        } = &mut root_info.kind
        {
            *scroll_offset_x = 5.0;
        }
        root_info
            .children
            .push(scrollable_info(Some(0), false, 0.0));

        let js_dom_ids = HashMap::from([(Rc::as_ptr(&target) as usize, 42)]);
        let measurements = collect_js_layout_metrics(&root, &root_info, &dom_refs, &js_dom_ids);
        assert_eq!(
            measurements.get(&42),
            Some(&JsLayoutMetrics {
                offset_left: 30.0,
                offset_top: 40.0,
                offset_width: 120.0,
                offset_height: 80.0,
                client_width: 116.0,
                client_height: 76.0,
                rect_left: 35.0,
                rect_top: 53.0,
                rect_width: 120.0,
                rect_height: 80.0,
            })
        );
    }

    #[test]
    fn fragment_waits_for_the_styled_layout_generation() {
        assert!(!fragment_layout_is_ready(None, 5));
        assert!(!fragment_layout_is_ready(Some(5), 4));
        assert!(fragment_layout_is_ready(Some(5), 5));
        assert!(fragment_layout_is_ready(Some(5), 6));
    }

    #[test]
    fn fragment_target_scrolls_the_page_to_its_border_box() {
        let mut target_layout = LayoutNode::new(ui_layout::Style::default());
        target_layout.layout_box = layout_box(900.0, 100.0, 100.0);
        let mut root_layout =
            LayoutNode::with_children(ui_layout::Style::default(), [target_layout]);
        root_layout.layout_box = layout_box(0.0, 600.0, 2000.0);

        let mut root_info = scrollable_info(Some(1), true, 0.0);
        root_info
            .children
            .push(scrollable_info(Some(7), false, 0.0));

        assert!(apply_fragment_scroll(
            &root_layout,
            &mut root_info,
            7,
            600.0
        ));
        assert_eq!(root_scroll_offset(&root_info), Some(900.0));
    }

    #[test]
    fn resize_preserves_scroll_offset() {
        // A scroll container needs a constrained height so its content_box
        // stays smaller than children_box (auto-height boxes stretch to their
        // content in this engine and are never scrollable).
        let html = r#"<html><body><div style="height: 300px; overflow-y: auto;"><div style="height: 3000px;"></div></div></body></html>"#;
        let mut wv = WebView::new(ColorScheme::Light, JsPolicy::default());
        wv.tick();
        wv.on_html_fetched(
            html.to_string(),
            Url::parse("https://example.test/").unwrap(),
        );

        for _ in 0..500 {
            wv.tick();
            if !wv.layout_pending && wv.layout_and_info().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        {
            let (_, info) = wv.layout_and_info_mut().expect("layout not ready");
            assert!(
                set_first_scrollable_offset(info, 500.0),
                "expected a scrollable container"
            );
        }

        wv.relayout((1000.0, 700.0));

        for _ in 0..500 {
            wv.tick();
            if !wv.layout_pending {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        let (_, info) = wv.layout_and_info().expect("layout not ready after resize");
        assert_eq!(find_scrollable_offset(info), Some(500.0));
    }

    #[test]
    fn resize_preserves_page_scroll_on_root() {
        // A plain page without overflow rules: the wheel handler stores the
        // page scroll on the root InfoNode, which has no scroll flags set.
        let html = r#"<html><body><div style="height: 3000px;"></div></body></html>"#;
        let mut wv = WebView::new(ColorScheme::Light, JsPolicy::default());
        wv.tick();
        wv.on_html_fetched(
            html.to_string(),
            Url::parse("https://example.test/").unwrap(),
        );

        for _ in 0..500 {
            wv.tick();
            if !wv.layout_pending && wv.layout_and_info().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        {
            let (_, info) = wv.layout_and_info_mut().expect("layout not ready");
            set_root_scroll_offset(info, 500.0);
        }

        wv.relayout((1000.0, 700.0));

        for _ in 0..500 {
            wv.tick();
            if !wv.layout_pending {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        let (_, info) = wv.layout_and_info().expect("layout not ready after resize");
        assert_eq!(root_scroll_offset(info), Some(500.0));
    }

    #[test]
    fn captures_and_restores_scroll_offsets_across_rebuild() {
        // Old tree: parent scrolled to 100, child to 50.
        let mut old_info = scrollable_info(Some(1), true, 100.0);
        old_info.children.push(scrollable_info(Some(2), true, 50.0));

        let mut offsets = HashMap::new();
        capture_scroll_offsets(&old_info, &mut offsets);
        assert_eq!(
            offsets,
            HashMap::from([(1u32, (0.0, 100.0)), (2u32, (0.0, 50.0))])
        );

        // New tree: same DOM, offsets reset to 0 by the builder.
        let mut new_info = scrollable_info(Some(1), true, 0.0);
        new_info.children.push(scrollable_info(Some(2), true, 0.0));

        apply_scroll_offsets(&mut new_info, &offsets);

        assert_eq!(scroll_offsets(&new_info), (0.0, 100.0));
        assert_eq!(scroll_offsets(&new_info.children[0]), (0.0, 50.0));
    }

    #[test]
    fn offsets_are_restored_verbatim_even_on_non_scrollable_axes() {
        // Old tree: the x axis was scrollable and scrolled to 50.
        let old_info = InfoNode {
            kind: NodeKind::Container {
                scroll_x: true,
                scroll_y: false,
                scroll_offset_x: 50.0,
                scroll_offset_y: 0.0,
                style: ContainerStyle::default(),
                role: ContainerRole::Normal,
            },
            children: Vec::new(),
            dom_id: Some(3),
        };

        let mut offsets = HashMap::new();
        capture_scroll_offsets(&old_info, &mut offsets);
        assert_eq!(offsets, HashMap::from([(3u32, (50.0, 0.0))]));

        // New tree: the x axis no longer scrolls. apply restores the captured
        // position verbatim; range enforcement is clamp's job, so the flag
        // change alone must not drop the offset.
        let mut new_info = InfoNode {
            kind: NodeKind::Container {
                scroll_x: false,
                scroll_y: false,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                style: ContainerStyle::default(),
                role: ContainerRole::Normal,
            },
            children: Vec::new(),
            dom_id: Some(3),
        };

        apply_scroll_offsets(&mut new_info, &offsets);

        assert_eq!(scroll_offsets(&new_info), (50.0, 0.0));
    }

    #[test]
    fn inline_styles_recover_after_an_unsupported_rule() {
        let mut webview = WebView::default();
        webview.on_html_fetched(
            r#"<style>@media { @broken } .valid { color: green; }</style><div class="valid">ok</div>"#
                .to_string(),
            Url::parse("https://example.test/").unwrap(),
        );

        assert!(webview.resolved_styles.iter().any(|declaration| {
            declaration.name == "color"
                && declaration
                    .selector
                    .parts
                    .iter()
                    .any(|part| part.selector.classes.iter().any(|class| class == "valid"))
        }));
    }

    #[test]
    fn javascript_inserted_style_elements_are_resolved() {
        let mut webview = WebView::default();
        webview.on_html_fetched(
            r#"<html><body><div class="dynamic">ok</div></body></html>"#.to_string(),
            Url::parse("https://example.test/").unwrap(),
        );
        webview.send_script(
            r#"
            const style = document.createElement("style");
            style.textContent = ".dynamic { color: red; }";
            document.documentElement.appendChild(style);
            "#,
        );

        pump_until(
            &mut webview,
            |wv| {
                wv.resolved_styles.iter().any(|declaration| {
                    declaration.name == "color"
                        && declaration.selector.parts.iter().any(|part| {
                            part.selector.classes.iter().any(|class| class == "dynamic")
                        })
                })
            },
            "JS-inserted style element to be resolved",
        );
    }

    /// Concatenates every text node under an info subtree.
    fn collect_text(info: &InfoNode) -> String {
        let mut text = match &info.kind {
            NodeKind::Text { text, .. } => text.clone(),
            _ => String::new(),
        };
        for child in &info.children {
            text.push_str(&collect_text(child));
        }
        text
    }

    /// Whether any box in the layout is sized 300×150 — the content-box size
    /// the builder gives an `<iframe>` by default (the border box is larger
    /// because the UA stylesheet adds a 2px border).
    fn has_300x150_box(node: &LayoutNode) -> bool {
        let sized = matches!(node.style.size.width, LengthOrAuto::Length(Length::Px(w)) if (w - 300.0).abs() < 0.001)
            && matches!(node.style.size.height, LengthOrAuto::Length(Length::Px(h)) if (h - 150.0).abs() < 0.001);
        sized
            || node
                .children
                .iter()
                .filter_map(LayoutChild::node)
                .any(has_300x150_box)
    }

    /// Whether some dual-axis scroll container (the `<iframe>` box, or the
    /// nested `<html>` document root grafted into it) holds only the given
    /// nested text and none of the host page's text.
    fn iframe_holds_grafted_content(info: &InfoNode) -> bool {
        if matches!(
            info.kind,
            NodeKind::Container {
                scroll_x: true,
                scroll_y: true,
                ..
            }
        ) {
            let text = collect_text(info);
            if text.contains("grafted inner paragraph") && !text.contains("host paragraph") {
                return true;
            }
        }
        info.children.iter().any(iframe_holds_grafted_content)
    }

    /// Runs the committed layout through draw-command generation and returns
    /// the whitespace-stripped text payload of every `DrawText` command — i.e.
    /// the text that would actually be rasterized on screen.
    fn paint_text(webview: &WebView) -> String {
        let Some((layout, info)) = webview.layout_and_info() else {
            return String::new();
        };
        let mut commands = Vec::new();
        crate::engine::renderer_model::generate_draw_commands(
            &mut commands,
            layout,
            info,
            (800.0, 600.0),
        );
        let mut text = String::new();
        for command in &commands {
            if let crate::engine::renderer_model::DrawCommand::DrawText { text: run, .. } = command
            {
                text.push_str(run);
            }
        }
        text.chars().filter(|c| !c.is_whitespace()).collect()
    }

    #[test]
    fn iframe_content_documents_are_fetched_grafted_and_laid_out() {
        let mut webview = WebView::default();
        webview.tick();
        webview.on_html_fetched(
            r#"<html><body><p>host paragraph</p></body></html>"#.to_string(),
            Url::parse("https://example.test/inside-host.html").unwrap(),
        );
        // Create the iframe from JS so the src attribute is set on a live node
        // (the same path the acid3 harness exercises).
        webview.send_script(
            r#"
            const frame = document.createElement("iframe");
            document.body.appendChild(frame);
            frame.src = "https://example.test/inside.html";
            "#,
        );

        // The JS thread reports the iframe's src as a fetch request.
        let task = pump_for_task(
            &mut webview,
            |task| {
                matches!(
                    task,
                    WebViewTask::Fetch {
                        kind: FetchKind::Iframe { .. },
                        ..
                    }
                )
            },
            "iframe fetch request",
        );
        let (url, dom_id) = match task {
            WebViewTask::Fetch {
                url,
                kind: FetchKind::Iframe { dom_id },
            } => (url, dom_id),
            _ => unreachable!("pump_for_task only returns matching tasks"),
        };
        assert_eq!(url.as_str(), "https://example.test/inside.html");

        // The fetched HTML is parsed on the JS thread and installed as the
        // iframe's content document; the committed result must carry it back so
        // the browser can graft it under the host <iframe> node.
        webview.on_iframe_fetched(
            dom_id,
            r#"<html><body><p>grafted inner paragraph</p></body></html>"#.to_string(),
        );

        pump_until(
            &mut webview,
            |wv| {
                let Some((layout, info)) = wv.layout_and_info() else {
                    return false;
                };
                iframe_holds_grafted_content(info)
                    && collect_text(info).contains("host paragraph")
                    && has_300x150_box(layout)
            },
            "grafted iframe content to reach the layout",
        );

        // The nested content must be reachable by the paint pass, not just the
        // layout tree.
        let painted = paint_text(&webview);
        assert!(
            painted.contains("hostparagraph"),
            "host page text must be painted"
        );
        assert!(
            painted.contains("graftedinnerparagraph"),
            "iframe content text must be painted"
        );
    }

    #[test]
    fn markup_declared_iframes_load_content_without_javascript() {
        let mut webview = WebView::default();
        webview.tick();
        // A plain `<iframe src>` in the parsed HTML must load like any other
        // subresource: real pages declare frames in markup, and nothing sets
        // their `src` property from JavaScript.
        webview.on_html_fetched(
            r#"<html><body><p>host paragraph</p><iframe src="https://example.test/inside.html"></iframe></body></html>"#.to_string(),
            Url::parse("https://example.test/inside-host.html").unwrap(),
        );

        let task = pump_for_task(
            &mut webview,
            |task| {
                matches!(
                    task,
                    WebViewTask::Fetch {
                        kind: FetchKind::Iframe { .. },
                        ..
                    }
                )
            },
            "fetch request for a markup-declared iframe",
        );
        let (url, dom_id) = match task {
            WebViewTask::Fetch {
                url,
                kind: FetchKind::Iframe { dom_id },
            } => (url, dom_id),
            _ => unreachable!("pump_for_task only returns matching tasks"),
        };
        assert_eq!(url.as_str(), "https://example.test/inside.html");

        webview.on_iframe_fetched(
            dom_id,
            r#"<html><body><p>grafted inner paragraph</p></body></html>"#.to_string(),
        );
        pump_until(
            &mut webview,
            |wv| {
                let Some((layout, info)) = wv.layout_and_info() else {
                    return false;
                };
                iframe_holds_grafted_content(info)
                    && collect_text(info).contains("host paragraph")
                    && has_300x150_box(layout)
            },
            "markup-declared iframe content to reach the layout",
        );

        let painted = paint_text(&webview);
        assert!(
            painted.contains("hostparagraph"),
            "host page text must be painted"
        );
        assert!(
            painted.contains("graftedinnerparagraph"),
            "iframe content text must be painted"
        );
    }

    #[test]
    fn parse_html_resolves_image_sources_against_base_url() {
        let parsed = parse_html(
            r#"<base href="https://cdn.example/assets/"><img src="logo.png"><img>"#,
            Url::parse("https://example.test/page/index.html").unwrap(),
            ScriptingMode::Enabled,
        );

        assert_eq!(parsed.image_sources.len(), 1);
        assert_eq!(parsed.image_sources[0].0, "logo.png");
        assert_eq!(
            parsed.image_sources[0].1.as_str(),
            "https://cdn.example/assets/logo.png"
        );
    }

    #[test]
    fn parse_html_resolves_audio_and_child_source_urls() {
        let parsed = parse_html(
            r#"<base href="https://cdn.example/media/"><audio src="one.mp3"></audio><audio><source src="two.ogg"></audio>"#,
            Url::parse("https://example.test/index.html").unwrap(),
            ScriptingMode::Enabled,
        );

        assert_eq!(
            parsed.audio_sources,
            [
                (
                    "one.mp3".to_string(),
                    Url::parse("https://cdn.example/media/one.mp3").unwrap()
                ),
                (
                    "two.ogg".to_string(),
                    Url::parse("https://cdn.example/media/two.ogg").unwrap()
                ),
            ]
        );
    }

    #[test]
    fn parse_html_resolves_external_classic_scripts_in_document_order() {
        let parsed = parse_html(
            r#"<base href="https://cdn.example/js/"><script>let a = 1;</script><script src="one.js"></script><script src="/two.js"></script>"#,
            Url::parse("https://example.test/page/index.html").unwrap(),
            ScriptingMode::Enabled,
        );

        assert_eq!(
            parsed.scripts,
            [
                ClassicScript::Inline("let a = 1;".to_string()),
                ClassicScript::External {
                    url: Url::parse("https://cdn.example/js/one.js").unwrap(),
                    execution: ClassicScriptExecution::Default,
                },
                ClassicScript::External {
                    url: Url::parse("https://cdn.example/two.js").unwrap(),
                    execution: ClassicScriptExecution::Default,
                },
            ]
        );
    }

    #[test]
    fn external_classic_scripts_fetch_and_execute_in_document_order() {
        let mut webview = WebView::default();
        webview.on_html_fetched(
            r#"
                <div id="result"></div>
                <script>let order = "a";</script>
                <script src="one.js"></script>
                <script>order = order + "c";</script>
                <script src="two.js"></script>
            "#
            .to_string(),
            Url::parse("https://example.test/path/index.html").unwrap(),
        );

        assert!(webview.tick().is_empty());
        let first_tasks = webview.tick();
        assert_eq!(first_tasks.len(), 1);
        match &first_tasks[0] {
            WebViewTask::Fetch {
                url,
                kind: FetchKind::Script { index },
            } => {
                assert_eq!(*index, 1);
                assert_eq!(url.as_str(), "https://example.test/path/one.js");
            }
            _ => panic!("expected first external classic script fetch"),
        }

        webview.on_script_fetched(1, r#"order = order + "b";"#.to_string());
        let second_tasks = webview.tick();
        assert_eq!(second_tasks.len(), 1);
        match &second_tasks[0] {
            WebViewTask::Fetch {
                url,
                kind: FetchKind::Script { index },
            } => {
                assert_eq!(*index, 3);
                assert_eq!(url.as_str(), "https://example.test/path/two.js");
            }
            _ => panic!("expected second external classic script fetch"),
        }

        let result = webview
            .document_info()
            .unwrap()
            .dom
            .get_element_by_id("result")
            .unwrap();
        assert_eq!(result.borrow().value.get_attr("data-order"), None);

        webview.on_script_fetched(
            3,
            r#"document.getElementById("result").setAttribute("data-order", order + "d");"#
                .to_string(),
        );
        pump_until(
            &mut webview,
            |wv| {
                wv.document_info()
                    .unwrap()
                    .dom
                    .get_element_by_id("result")
                    .is_some_and(|node| node.borrow().value.get_attr("data-order").is_some())
            },
            "final classic script result to be committed",
        );
        let result = webview
            .document_info()
            .unwrap()
            .dom
            .get_element_by_id("result")
            .unwrap();
        assert_eq!(result.borrow().value.get_attr("data-order"), Some("abcd"));
        assert_eq!(webview.phase, PagePhase::ScriptApplied);
    }

    #[test]
    fn failed_external_classic_script_does_not_block_later_scripts() {
        let mut webview = WebView::default();
        webview.on_html_fetched(
            r#"
                <div id="result"></div>
                <script src="missing.js"></script>
                <script>document.getElementById("result").setAttribute("data-ran", "yes");</script>
            "#
            .to_string(),
            Url::parse("https://example.test/index.html").unwrap(),
        );

        assert!(webview.tick().is_empty());
        let tasks = webview.tick();
        assert!(matches!(
            tasks.as_slice(),
            [WebViewTask::Fetch {
                kind: FetchKind::Script { index: 0 },
                ..
            }]
        ));

        webview.on_script_fetch_failed(0);
        pump_until(
            &mut webview,
            |wv| {
                wv.document_info()
                    .unwrap()
                    .dom
                    .get_element_by_id("result")
                    .is_some_and(|node| node.borrow().value.get_attr("data-ran").is_some())
            },
            "inline script after a failed external script",
        );
        let result = webview
            .document_info()
            .unwrap()
            .dom
            .get_element_by_id("result")
            .unwrap();
        assert_eq!(result.borrow().value.get_attr("data-ran"), Some("yes"));
    }

    #[test]
    fn dom_content_loaded_fires_after_external_classic_scripts_finish() {
        let mut webview = WebView::default();
        webview.on_html_fetched(
            r#"
                <div id="result"></div>
                <script>
                    document.addEventListener("DOMContentLoaded", function () {
                        const result = document.getElementById("result");
                        result.setAttribute("data-ready", result.getAttribute("data-external"));
                    });
                </script>
                <script src="setup.js"></script>
            "#
            .to_string(),
            Url::parse("https://example.test/index.html").unwrap(),
        );

        assert!(webview.tick().is_empty());
        let tasks = webview.tick();
        assert!(matches!(
            tasks.as_slice(),
            [WebViewTask::Fetch {
                kind: FetchKind::Script { index: 1 },
                ..
            }]
        ));

        let result = webview
            .document_info()
            .unwrap()
            .dom
            .get_element_by_id("result")
            .unwrap();
        assert_eq!(result.borrow().value.get_attr("data-ready"), None);

        webview.on_script_fetched(
            1,
            r#"document.getElementById("result").setAttribute("data-external", "yes");"#
                .to_string(),
        );
        assert_eq!(result.borrow().value.get_attr("data-ready"), None);

        pump_until(
            &mut webview,
            |wv| {
                wv.document_info()
                    .unwrap()
                    .dom
                    .get_element_by_id("result")
                    .is_some_and(|node| node.borrow().value.get_attr("data-ready").is_some())
            },
            "DOMContentLoaded listener to run",
        );
        let result = webview
            .document_info()
            .unwrap()
            .dom
            .get_element_by_id("result")
            .unwrap();
        assert_eq!(result.borrow().value.get_attr("data-ready"), Some("yes"));
        assert_eq!(webview.phase, PagePhase::ScriptApplied);
    }

    #[test]
    fn window_onload_fires_after_the_page_stabilizes() {
        let mut webview = WebView::default();
        webview.on_html_fetched(
            r#"
                <div id="result"></div>
                <script>
                    window.onload = function () {
                        document.getElementById("result").setAttribute("data-loaded", "yes");
                    };
                </script>
            "#
            .to_string(),
            Url::parse("https://example.test/index.html").unwrap(),
        );

        pump_until(
            &mut webview,
            |wv| {
                wv.document_info()
                    .unwrap()
                    .dom
                    .get_element_by_id("result")
                    .is_some_and(|node| node.borrow().value.get_attr("data-loaded").is_some())
            },
            "window.onload to run",
        );
        let result = webview
            .document_info()
            .unwrap()
            .dom
            .get_element_by_id("result")
            .unwrap();
        assert_eq!(result.borrow().value.get_attr("data-loaded"), Some("yes"));
        assert_eq!(webview.phase, PagePhase::ScriptApplied);
    }

    #[test]
    fn deferred_scripts_fetch_in_parallel_and_execute_in_document_order() {
        let mut webview = WebView::default();
        webview.on_html_fetched(
            r#"
                <div id="result"></div>
                <script>let order = "inline";</script>
                <script defer src="first.js"></script>
                <script defer src="second.js"></script>
                <script>
                    document.addEventListener("DOMContentLoaded", function () {
                        document.getElementById("result").setAttribute("data-order", order);
                    });
                </script>
            "#
            .to_string(),
            Url::parse("https://example.test/index.html").unwrap(),
        );

        assert!(webview.tick().is_empty());
        let tasks = webview.tick();
        assert_eq!(tasks.len(), 2);
        assert!(matches!(
            tasks[0],
            WebViewTask::Fetch {
                kind: FetchKind::Script { index: 1 },
                ..
            }
        ));
        assert!(matches!(
            tasks[1],
            WebViewTask::Fetch {
                kind: FetchKind::Script { index: 2 },
                ..
            }
        ));

        webview.on_script_fetched(2, r#"order = order + " > second";"#.to_string());
        assert!(webview.tick().is_empty());
        assert_ne!(webview.phase, PagePhase::ScriptApplied);

        webview.on_script_fetched(1, r#"order = order + " > first";"#.to_string());
        pump_until(
            &mut webview,
            |wv| {
                wv.document_info()
                    .unwrap()
                    .dom
                    .get_element_by_id("result")
                    .is_some_and(|node| node.borrow().value.get_attr("data-order").is_some())
            },
            "deferred scripts and DOMContentLoaded to run",
        );
        let result = webview
            .document_info()
            .unwrap()
            .dom
            .get_element_by_id("result")
            .unwrap();
        assert_eq!(
            result.borrow().value.get_attr("data-order"),
            Some("inline > first > second")
        );
        assert_eq!(webview.phase, PagePhase::ScriptApplied);
    }

    #[test]
    fn async_script_executes_on_arrival_without_blocking_dom_content_loaded() {
        let mut webview = WebView::default();
        webview.on_html_fetched(
            r#"
                <div id="result"></div>
                <script async src="async.js"></script>
                <script>
                    document.addEventListener("DOMContentLoaded", function () {
                        document.getElementById("result").setAttribute("data-ready", "yes");
                    });
                </script>
            "#
            .to_string(),
            Url::parse("https://example.test/index.html").unwrap(),
        );

        assert!(webview.tick().is_empty());
        let tasks = webview.tick();
        assert!(matches!(
            tasks.as_slice(),
            [WebViewTask::Fetch {
                kind: FetchKind::Script { index: 0 },
                ..
            }]
        ));

        pump_until(
            &mut webview,
            |wv| {
                wv.document_info()
                    .unwrap()
                    .dom
                    .get_element_by_id("result")
                    .is_some_and(|node| node.borrow().value.get_attr("data-ready").is_some())
            },
            "DOMContentLoaded before the async script arrives",
        );
        let result = webview
            .document_info()
            .unwrap()
            .dom
            .get_element_by_id("result")
            .unwrap();
        assert_eq!(result.borrow().value.get_attr("data-ready"), Some("yes"));
        assert_eq!(result.borrow().value.get_attr("data-async"), None);
        assert_eq!(webview.phase, PagePhase::ScriptApplied);

        webview.on_script_fetched(
            0,
            r#"document.getElementById("result").setAttribute("data-async", "yes");"#.to_string(),
        );
        pump_until(
            &mut webview,
            |wv| {
                wv.document_info()
                    .unwrap()
                    .dom
                    .get_element_by_id("result")
                    .is_some_and(|node| node.borrow().value.get_attr("data-async").is_some())
            },
            "async script to run after DOMContentLoaded",
        );
        let result = webview
            .document_info()
            .unwrap()
            .dom
            .get_element_by_id("result")
            .unwrap();
        assert_eq!(result.borrow().value.get_attr("data-async"), Some("yes"));
    }

    #[test]
    fn javascript_fetch_uses_document_url_and_resolves_response() {
        let mut webview = WebView::default();
        webview.on_html_fetched(
            r#"
                <div id="result"></div>
                <script>
                    fetch("../message.txt")
                        .then(response => response.text())
                        .then(text => {
                            document.getElementById("result").setAttribute("data-text", text);
                        });
                </script>
            "#
            .to_string(),
            Url::parse("https://example.test/path/index.html").unwrap(),
        );

        let fetch_task = pump_for_task(
            &mut webview,
            |task| {
                matches!(
                    task,
                    WebViewTask::Fetch {
                        kind: FetchKind::JavaScript { .. },
                        ..
                    }
                )
            },
            "the page's fetch() to be dispatched",
        );
        let request_id = match fetch_task {
            WebViewTask::Fetch {
                url,
                kind: FetchKind::JavaScript { request_id, .. },
            } => {
                assert_eq!(url.as_str(), "https://example.test/message.txt");
                request_id
            }
            _ => unreachable!("guarded by pump_for_task"),
        };

        webview.on_js_fetch_succeeded(
            request_id,
            JsFetchResponse {
                url: "https://example.test/message.txt".to_string(),
                status: 200,
                status_text: "OK".to_string(),
                redirected: false,
                body: b"hello from fetch".to_vec(),
                headers: Vec::new(),
            },
        );

        pump_until(
            &mut webview,
            |wv| {
                wv.document_info()
                    .unwrap()
                    .dom
                    .get_element_by_id("result")
                    .is_some_and(|node| node.borrow().value.get_attr("data-text").is_some())
            },
            "the fetch response microtask to run",
        );
        let result = webview
            .document_info()
            .unwrap()
            .dom
            .get_element_by_id("result")
            .unwrap();
        assert_eq!(
            result.borrow().value.get_attr("data-text"),
            Some("hello from fetch")
        );
    }

    #[test]
    fn failed_javascript_fetch_rejects_without_navigating() {
        let mut webview = WebView::default();
        webview.on_html_fetched(
            r#"
                <div id="result"></div>
                <script>
                    fetch("missing.txt").catch(error => {
                        document.getElementById("result").setAttribute("data-error", error);
                    });
                </script>
            "#
            .to_string(),
            Url::parse("https://example.test/index.html").unwrap(),
        );

        let fetch_task = pump_for_task(
            &mut webview,
            |task| {
                matches!(
                    task,
                    WebViewTask::Fetch {
                        kind: FetchKind::JavaScript { .. },
                        ..
                    }
                )
            },
            "the page's fetch() to be dispatched",
        );
        let request_id = match fetch_task {
            WebViewTask::Fetch {
                kind: FetchKind::JavaScript { request_id, .. },
                ..
            } => request_id,
            _ => unreachable!("guarded by pump_for_task"),
        };

        webview.on_js_fetch_failed(request_id, "network error".to_string());

        pump_until(
            &mut webview,
            |wv| {
                wv.document_info()
                    .unwrap()
                    .dom
                    .get_element_by_id("result")
                    .is_some_and(|node| node.borrow().value.get_attr("data-error").is_some())
            },
            "the fetch rejection to run",
        );
        let result = webview
            .document_info()
            .unwrap()
            .dom
            .get_element_by_id("result")
            .unwrap();
        assert_eq!(
            result.borrow().value.get_attr("data-error"),
            Some("network error")
        );
        assert_eq!(
            webview.document_info().unwrap().base_url.as_str(),
            "https://example.test/index.html"
        );
    }

    #[test]
    fn document_origin_is_exposed_to_page_scripts() {
        let mut webview = WebView::default();
        webview.on_html_fetched(
            r#"
                <div id="result"></div>
                <script>
                    document.getElementById("result").setAttribute(
                        "data-origin",
                        location.origin + ":" + window.origin + ":" + document.origin
                    );
                </script>
            "#
            .to_string(),
            Url::parse("https://example.test/path/index.html").unwrap(),
        );

        pump_until(
            &mut webview,
            |wv| {
                wv.document_info()
                    .unwrap()
                    .dom
                    .get_element_by_id("result")
                    .is_some_and(|node| node.borrow().value.get_attr("data-origin").is_some())
            },
            "the origin-handling script to run",
        );
        let result = webview
            .document_info()
            .unwrap()
            .dom
            .get_element_by_id("result")
            .unwrap();
        assert_eq!(
            result.borrow().value.get_attr("data-origin"),
            Some("https://example.test:https://example.test:https://example.test")
        );
    }

    #[test]
    fn internal_document_reports_null_origin() {
        let mut webview = WebView::default();
        webview.on_html_fetched(
            r#"
                <div id="result"></div>
                <script>
                    document.getElementById("result").setAttribute(
                        "data-origin",
                        location.origin + ":" + window.origin + ":" + document.origin
                    );
                </script>
            "#
            .to_string(),
            Url::parse("resource:///devtools/index.html").unwrap(),
        );

        pump_until(
            &mut webview,
            |wv| {
                wv.document_info()
                    .unwrap()
                    .dom
                    .get_element_by_id("result")
                    .is_some_and(|node| node.borrow().value.get_attr("data-origin").is_some())
            },
            "the origin-handling script to run",
        );
        let result = webview
            .document_info()
            .unwrap()
            .dom
            .get_element_by_id("result")
            .unwrap();
        assert_eq!(
            result.borrow().value.get_attr("data-origin"),
            Some("null:null:null")
        );
    }

    #[test]
    fn zero_delay_timer_runs_from_webview_tick_and_updates_dom() {
        let mut webview = WebView::default();
        webview.on_html_fetched(
            r##"
                <div id="result"></div>
                <script>
                    setTimeout(function () {
                        document.querySelector("#result").setAttribute("data-timer", "ran");
                    }, 0);
                </script>
            "##
            .to_string(),
            Url::parse("https://example.test/index.html").unwrap(),
        );

        assert!(webview.tick().is_empty());
        assert!(webview.tick().is_empty());
        pump_until(
            &mut webview,
            |wv| {
                wv.document_info()
                    .unwrap()
                    .dom
                    .get_element_by_id("result")
                    .is_some_and(|node| node.borrow().value.get_attr("data-timer").is_some())
            },
            "the zero-delay timer callback to run",
        );
        let result = webview
            .document_info()
            .unwrap()
            .dom
            .get_element_by_id("result")
            .unwrap();
        assert_eq!(result.borrow().value.get_attr("data-timer"), Some("ran"));
        assert!(webview.needs_redraw());
    }

    #[test]
    fn devtools_request_round_trips_through_inspection_and_back() {
        let mut webview = WebView::default();
        webview.on_html_fetched(
            r##"
                <div id="probe"></div>
                <script>
                    __orinium_devtools("getVersion").then(function (json) {
                        const envelope = JSON.parse(json);
                        document.getElementById("probe")
                            .setAttribute("data-ok", envelope.ok ? "yes" : "no");
                    });
                </script>
            "##
            .to_string(),
            Url::parse("https://example.test/index.html").unwrap(),
        );

        let task = pump_for_task(
            &mut webview,
            |task| matches!(task, WebViewTask::DevToolsRequest { .. }),
            "the page's DevTools inspection request",
        );
        let WebViewTask::DevToolsRequest { id, method, params } = task else {
            unreachable!("pump_for_task matched this variant");
        };
        assert_eq!(method, "getVersion");
        assert_eq!(params, "{}");

        let data = webview
            .inspect(&method, &params)
            .expect("inspection answer");
        webview.on_devtools_response(
            id,
            serde_json::json!({ "ok": true, "data": data }).to_string(),
        );

        pump_until(
            &mut webview,
            |wv| {
                wv.document_info()
                    .unwrap()
                    .dom
                    .get_element_by_id("probe")
                    .is_some_and(|node| node.borrow().value.get_attr("data-ok").is_some())
            },
            "the resolved promise callback to mark the probe element",
        );
        let probe = webview
            .document_info()
            .unwrap()
            .dom
            .get_element_by_id("probe")
            .unwrap();
        assert_eq!(probe.borrow().value.get_attr("data-ok"), Some("yes"));
    }
}
