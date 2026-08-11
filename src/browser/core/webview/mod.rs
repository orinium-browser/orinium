//! ブラウザのwebview機能。タスクとレンダリング情報の管理を行う。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::{Arc, mpsc};

use crate::engine::layouter::types::ColorScheme;
use crate::engine::{
    css::{self, parser::Parser as CssParser},
    html::HtmlNodeType,
    html::parser::{
        ClassicScriptExecution, ClassicScriptSource, DomTree, Parser as HtmlParser, ScriptingMode,
    },
    js::{JsFetchResponse, JsRuntime},
    layouter::{
        self, InheritedCss, LayoutResult, NodeId,
        dom_snapshot::DomSnapshot,
        types::{InfoNode, NodeKind, TextStyle},
    },
    renderer_model::Image,
    tree::TreeNode,
};
use crate::platform::renderer::text_measurer::PlatformTextMeasurer;
use ui_layout::LayoutNode;
use url::Url;

const USER_AGENT_CSS: &str = include_str!("../../../../resource/user-agent.css");

pub enum WebViewTask {
    AskTabHtml,
    Fetch { url: Url, kind: FetchKind },
}

/// TODO:
/// - Root Document fetch
/// - Image fetch
/// - JS fetch
/// - その他リソース fetch
pub enum FetchKind {
    Html,
    Css,
    Script {
        index: usize,
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
    audio: HashMap<String, Arc<[u8]>>,

    resolved_styles: Arc<layouter::css_resolver::ResolvedStyles>,
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
    /// Live DOM references for the latest snapshot, used to apply write-backs.
    layout_dom_refs: Vec<Weak<RefCell<TreeNode<HtmlNodeType>>>>,
    /// Cached DOM snapshot, reused while the tree's mutation version is
    /// unchanged so that CSS/image-driven relayouts skip the full clone.
    snapshot_cache: Option<SnapshotCache>,
    /// Channel on which text inputs report value write-backs (received here).
    write_back_tx: mpsc::Sender<(u32, String)>,
    write_back_rx: mpsc::Receiver<(u32, String)>,
    /// JS runtime sharing the current document's DOM.
    js_runtime: Option<JsRuntime>,
    /// Classic scripts in document order. Execution starts after CSS is applied.
    classic_scripts: Vec<ClassicScript>,
    next_script_index: usize,
    pending_script_fetches: HashMap<usize, ClassicScriptExecution>,
    non_blocking_scripts_scheduled: bool,
    deferred_script_results: HashMap<usize, Option<String>>,
    next_deferred_script_index: usize,
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
/// - title: The title of the document.
/// - dom: The DOM tree of the document.
#[derive(Debug)]
pub struct DocumentInfo {
    document_url: Url,
    base_url: Url,
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
            audio: HashMap::new(),

            resolved_styles: Arc::new(layouter::css_resolver::ResolvedStyles::default()),
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
            layout_dom_refs: Vec::new(),
            snapshot_cache: None,
            write_back_tx,
            write_back_rx,
            js_runtime: None,
            classic_scripts: Vec::new(),
            next_script_index: 0,
            pending_script_fetches: HashMap::new(),
            non_blocking_scripts_scheduled: false,
            deferred_script_results: HashMap::new(),
            next_deferred_script_index: 0,
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
        } else if self.js_runtime.is_none()
            && let Some(dom) = self.docment_info.as_ref().map(|info| Rc::clone(&info.dom))
        {
            // Re-enabling: install a runtime so DOM APIs work again, without
            // replaying scripts that were skipped while disabled.
            self.js_runtime = Some(JsRuntime::new(dom));
        }
    }

    /// Returns the current JavaScript execution policy.
    pub fn js_policy(&self) -> JsPolicy {
        self.js_policy
    }

    /// Stops script execution immediately, dropping the runtime and any
    /// pending script work.
    fn teardown_script_execution(&mut self) {
        self.js_runtime = None;
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

        self.schedule_js_fetches(&mut tasks);
        self.run_due_js_timers();
        self.try_apply_layout_results();
        self.drain_write_backs();

        tasks
    }

    pub fn on_html_fetched(&mut self, html: String, document_url: Url) {
        log::info!("Fetched HTML: {}", document_url);
        let parsed = parse_html(&html, document_url, self.js_policy.into());

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

        self.js_runtime = if self.js_policy == JsPolicy::Enabled {
            Some(JsRuntime::new(Rc::clone(&parsed.dom)))
        } else {
            None
        };

        let docment_info = DocumentInfo {
            document_url: parsed.document_url,
            base_url: parsed.base_url,
            dom: parsed.dom,
            title: parsed.title,
        };
        self.docment_info = Some(docment_info);
        self.snapshot_cache = None;

        for inline_css in &parsed.inline_styles {
            let sheet = CssParser::new(inline_css).parse_lossy();
            layouter::css_resolver::append_resolved_styles(
                Arc::make_mut(&mut self.resolved_styles),
                layouter::css_resolver::CssResolver::resolve(&sheet),
            );
        }
        self.phase = PagePhase::HtmlParsed;
    }

    pub fn on_css_fetched(&mut self, css: String) {
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
        let image = Image::decode(bytes)?;
        self.images.insert(source, image);
        self.update_layout();
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
                self.run_script_source(&source);
            }
            ClassicScriptExecution::Defer => {
                self.deferred_script_results.insert(index, Some(source));
            }
            ClassicScriptExecution::Async => self.run_script_source(&source),
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

    /// Resolves a JavaScript `fetch()` request with a network response.
    pub fn on_js_fetch_succeeded(
        &mut self,
        request_id: u64,
        url: String,
        status: u16,
        status_text: String,
        redirected: bool,
        body: Vec<u8>,
        headers: Vec<(String, String)>,
    ) {
        let needs_redraw = self.js_runtime.as_mut().is_some_and(|runtime| {
            runtime.resolve_fetch(
                request_id,
                JsFetchResponse {
                    url,
                    status,
                    status_text,
                    redirected,
                    body,
                    headers,
                },
            );
            runtime.take_needs_redraw()
        });
        if needs_redraw {
            self.rebuild_styles_and_layout();
            self.needs_redraw = true;
        }
    }

    /// Rejects a JavaScript `fetch()` request after a network failure.
    pub fn on_js_fetch_failed(&mut self, request_id: u64, reason: String) {
        let needs_redraw = self.js_runtime.as_mut().is_some_and(|runtime| {
            runtime.reject_fetch(request_id, reason);
            runtime.take_needs_redraw()
        });
        if needs_redraw {
            self.rebuild_styles_and_layout();
            self.needs_redraw = true;
        }
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
    }

    fn apply_resolved_styles_and_relayout(
        &mut self,
        resolved: layouter::css_resolver::ResolvedStyles,
    ) {
        layouter::css_resolver::append_resolved_styles(
            Arc::make_mut(&mut self.resolved_styles),
            resolved,
        );
        self.update_layout();
    }

    fn rebuild_styles_and_layout(&mut self) {
        let Some(document) = self.docment_info.as_ref() else {
            self.update_layout();
            return;
        };
        let mut resolved = layouter::css_resolver::CssResolver::resolve_with_origin(
            &CssParser::new(USER_AGENT_CSS).parse().unwrap(),
            layouter::css_resolver::StyleOrigin::UserAgent,
        );
        for source in &self.linked_css {
            let sheet = CssParser::new(source).parse_lossy();
            layouter::css_resolver::append_resolved_styles(
                &mut resolved,
                layouter::css_resolver::CssResolver::resolve(&sheet),
            );
        }
        for source in document.dom.collect_text_by_tag("style") {
            let sheet = CssParser::new(&source).parse_lossy();
            layouter::css_resolver::append_resolved_styles(
                &mut resolved,
                layouter::css_resolver::CssResolver::resolve(&sheet),
            );
        }
        self.resolved_styles = Arc::new(resolved);
        self.update_layout();
    }

    fn try_apply_css_results(&mut self) {
        while let Some(resolved) = self.css_processor.try_receive() {
            self.css_results_received += 1;
            self.apply_resolved_styles_and_relayout(resolved);
            self.needs_redraw = true;

            if self.css_results_received >= self.css_results_expected {
                self.phase = PagePhase::CssApplied;
            }
        }
    }

    fn try_apply_batch_result(&mut self) {
        if let Some(resolved) = self.css_processor.try_receive() {
            self.css_results_received += 1;
            self.apply_resolved_styles_and_relayout(resolved);
            self.needs_redraw = true;
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
                    self.run_script_source(&source);
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
                self.run_script_source(&source);
            }
        }
    }

    fn run_script_source(&mut self, source: &str) {
        let needs_redraw = self.js_runtime.as_mut().is_some_and(|runtime| {
            runtime.run_script(source);
            runtime.take_needs_redraw()
        });

        if needs_redraw {
            self.rebuild_styles_and_layout();
            self.needs_redraw = true;
        }
    }

    fn dispatch_dom_content_loaded(&mut self) {
        let needs_redraw = self.js_runtime.as_mut().is_some_and(|runtime| {
            runtime.dispatch_dom_content_loaded();
            runtime.take_needs_redraw()
        });

        if needs_redraw {
            self.rebuild_styles_and_layout();
            self.needs_redraw = true;
        }
    }

    fn run_due_js_timers(&mut self) {
        let needs_redraw = self.js_runtime.as_mut().is_some_and(|runtime| {
            runtime.run_due_timers();
            runtime.take_needs_redraw()
        });

        if needs_redraw {
            self.rebuild_styles_and_layout();
            self.needs_redraw = true;
        }
    }

    fn schedule_js_fetches(&mut self, tasks: &mut Vec<WebViewTask>) {
        let requests = self
            .js_runtime
            .as_mut()
            .map(JsRuntime::take_fetch_requests)
            .unwrap_or_default();
        let base_url = self.docment_info.as_ref().map(|info| info.base_url.clone());

        for request in requests {
            let url = Url::parse(&request.url).or_else(|_| {
                base_url
                    .as_ref()
                    .ok_or(url::ParseError::RelativeUrlWithoutBase)?
                    .join(&request.url)
            });
            match url {
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

    /// Dispatches a click on the given DOM snapshot node id to the page's JS.
    ///
    /// Resolves the live DOM node behind the snapshot id, runs its `onclick`
    /// handler if any, and relayouts + requests a redraw when the handler
    /// mutated the DOM. Returns whether a redraw is needed.
    pub fn on_js_click(&mut self, dom_id: u32) -> bool {
        let Some(js_runtime) = self.js_runtime.as_mut() else {
            return false;
        };
        let Some(node) = self
            .layout_dom_refs
            .get(dom_id as usize)
            .and_then(|weak| weak.upgrade())
        else {
            return false;
        };
        js_runtime.click(&node);
        if js_runtime.take_needs_redraw() {
            self.rebuild_styles_and_layout();
            self.needs_redraw = true;
            true
        } else {
            false
        }
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
        let (snapshot, dom_refs) = match &self.snapshot_cache {
            // The DOM is unchanged since the last snapshot: reuse it instead of
            // re-cloning the whole tree (CSS/image relayouts dominate).
            Some(cache) if cache.dom_version == dom_version => {
                (Arc::clone(&cache.snapshot), cache.dom_refs.clone())
            }
            _ => {
                let (snapshot, dom_refs) = DomSnapshot::from_tree(&doc_info.dom.root);
                let snapshot = Arc::new(snapshot);
                self.snapshot_cache = Some(SnapshotCache {
                    dom_version,
                    snapshot: Arc::clone(&snapshot),
                    dom_refs: dom_refs.clone(),
                });
                (snapshot, dom_refs)
            }
        };
        let root = snapshot.roots()[0];

        let media_environment =
            layouter::css_resolver::MediaEnvironment::new(self.viewport, self.system_color_scheme);
        let resolved_styles =
            layouter::css_resolver::filter_media(&self.resolved_styles, &media_environment);
        let task = layouter::LayoutTask {
            snapshot,
            root,
            resolved_styles: Arc::new(resolved_styles),
            measurer: self.text_measurer.clone().unwrap(),
            system_color_scheme: self.system_color_scheme,
            scripting_mode: self.js_policy.into(),
            images: self.images.clone(),
            audio: self.audio.clone(),
            parent: InheritedCss {
                text_style: TextStyle {
                    font_size: 16.0,
                    ..Default::default()
                },
                color_scheme: Default::default(),
            },
            chain: Vec::new(),
            write_back_sender: Some(self.write_back_tx.clone()),
            version: 0,
        };
        self.layout_dom_refs = dom_refs;
        self.layout_processor.send(task);
        self.layout_pending = true;
    }

    /// Takes completed layout results from the worker and makes them drawable.
    fn try_apply_layout_results(&mut self) {
        while let Some(result) = self.layout_processor.try_receive() {
            let LayoutResult { layout, mut info } = result;

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
            self.layout_pending = false;
            self.needs_redraw = true;
        }
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
        self.layout_dom_refs.clear();
        self.snapshot_cache = None;
        self.js_runtime = None;
        self.classic_scripts.clear();
        self.next_script_index = 0;
        self.pending_script_fetches.clear();
        self.non_blocking_scripts_scheduled = false;
        self.deferred_script_results.clear();
        self.next_deferred_script_index = 0;
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

    pub fn relayout(&mut self, viewport: (f32, f32)) {
        if self.viewport != viewport {
            self.viewport = viewport;
            self.update_layout();
        }

        let Some((layout, info)) = self.layout_and_info.as_mut() else {
            return;
        };

        ui_layout::LayoutEngine::layout(layout, viewport.0, viewport.1);
        layouter::align_table_columns(layout, info);
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
        webview.run_script_source(
            r#"
            const style = document.createElement("style");
            style.textContent = ".dynamic { color: red; }";
            document.documentElement.appendChild(style);
            "#,
        );

        assert!(webview.resolved_styles.iter().any(|declaration| {
            declaration.name == "color"
                && declaration
                    .selector
                    .parts
                    .iter()
                    .any(|part| part.selector.classes.iter().any(|class| class == "dynamic"))
        }));
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
        assert_eq!(result.borrow().value.get_attr("data-order"), Some("abcd"));
        assert!(webview.tick().is_empty());
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
        assert!(webview.tick().is_empty());
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

        assert!(webview.tick().is_empty());
        assert_eq!(result.borrow().value.get_attr("data-ready"), Some("yes"));
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
        assert!(webview.tick().is_empty());
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

        assert!(webview.tick().is_empty());
        let tasks = webview.tick();
        let request_id = match tasks.as_slice() {
            [
                WebViewTask::Fetch {
                    url,
                    kind: FetchKind::JavaScript { request_id, .. },
                },
            ] => {
                assert_eq!(url.as_str(), "https://example.test/message.txt");
                *request_id
            }
            _ => panic!("expected JavaScript fetch request"),
        };

        webview.on_js_fetch_succeeded(
            request_id,
            "https://example.test/message.txt".to_string(),
            200,
            "OK".to_string(),
            false,
            b"hello from fetch".to_vec(),
            Vec::new(),
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

        assert!(webview.tick().is_empty());
        let tasks = webview.tick();
        let request_id = match tasks.as_slice() {
            [
                WebViewTask::Fetch {
                    kind: FetchKind::JavaScript { request_id, .. },
                    ..
                },
            ] => *request_id,
            _ => panic!("expected JavaScript fetch request"),
        };

        webview.on_js_fetch_failed(request_id, "network error".to_string());

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
        let result = webview
            .document_info()
            .unwrap()
            .dom
            .get_element_by_id("result")
            .unwrap();
        assert_eq!(result.borrow().value.get_attr("data-timer"), Some("ran"));
        assert!(webview.needs_redraw());
    }
}
