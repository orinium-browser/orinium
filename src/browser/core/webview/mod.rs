//! ブラウザのwebview機能。タスクとレンダリング情報の管理を行う。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::{Arc, mpsc};

use crate::engine::layouter::types::ColorScheme;
use crate::engine::{
    css::{self, parser::Parser as CssParser},
    html::HtmlNodeType,
    html::parser::{DomTree, Parser as HtmlParser},
    js::JsRuntime,
    layouter::{
        self, InheritedCss,
        dom_snapshot::DomSnapshot,
        types::{InfoNode, TextStyle},
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
    Image { source: String },
    Audio { source: String },
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
    images: HashMap<String, Image>,
    audio: HashMap<String, Arc<[u8]>>,

    resolved_styles: Arc<layouter::css_resolver::ResolvedStyles>,
    layout_and_info: Option<(LayoutNode, InfoNode)>,

    needs_redraw: bool,

    text_measurer: Option<Arc<PlatformTextMeasurer>>,

    system_color_scheme: ColorScheme,

    css_processor: css::processor::CssProcessor,
    css_strategy: CssApplicationStrategy,
    css_results_expected: usize,
    css_results_received: usize,

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
    /// JS runtime sharing the current document's DOM, run once after CSS.
    js_runtime: Option<JsRuntime>,
    /// Inline `<script>` sources collected at parse time, run after CSS applied.
    pending_scripts: Vec<String>,
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
    scripts: Vec<String>,
}

impl Default for WebView {
    fn default() -> Self {
        Self::new(ColorScheme::default())
    }
}

impl WebView {
    pub fn new(system_color_scheme: ColorScheme) -> Self {
        let (write_back_tx, write_back_rx) = mpsc::channel();
        Self {
            phase: PagePhase::Init,

            docment_info: None,

            pending_css_urls: Vec::new(),
            pending_images: Vec::new(),
            pending_audio: Vec::new(),
            loaded_css: Vec::new(),
            images: HashMap::new(),
            audio: HashMap::new(),

            resolved_styles: Arc::new(layouter::css_resolver::ResolvedStyles::default()),
            layout_and_info: None,

            needs_redraw: false,

            text_measurer: None,

            system_color_scheme,

            css_processor: css::processor::CssProcessor::new(),
            css_strategy: CssApplicationStrategy::Incremental,
            css_results_expected: 0,
            css_results_received: 0,

            layout_processor: layouter::LayoutProcessor::new(),
            layout_pending: false,
            layout_dom_refs: Vec::new(),
            snapshot_cache: None,
            write_back_tx,
            write_back_rx,
            js_runtime: None,
            pending_scripts: Vec::new(),
        }
    }

    /// Set the CSS application strategy.
    ///
    /// Default is `Incremental`.
    pub fn set_css_strategy(&mut self, strategy: CssApplicationStrategy) {
        self.css_strategy = strategy;
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
                // Run inline scripts once, then settle into a stable state.
                self.run_scripts_once();
                self.phase = PagePhase::ScriptApplied;
            }

            PagePhase::ScriptApplied => {
                // 安定状態
            }
        }

        self.try_apply_layout_results();
        self.drain_write_backs();

        tasks
    }

    pub fn on_html_fetched(&mut self, html: String, document_url: Url) {
        log::info!("Fetched HTML: {}", document_url);
        let parsed = parse_html(&html, document_url);

        self.pending_css_urls = parsed.style_links;
        self.pending_images = parsed.image_sources;
        self.pending_audio = parsed.audio_sources;
        self.pending_scripts = parsed.scripts;
        self.css_results_expected = self.pending_css_urls.len();

        self.js_runtime = Some(JsRuntime::new(Rc::clone(&parsed.dom)));

        let docment_info = DocumentInfo {
            document_url: parsed.document_url,
            base_url: parsed.base_url,
            dom: parsed.dom,
            title: parsed.title,
        };
        self.docment_info = Some(docment_info);
        self.snapshot_cache = None;

        for inline_css in &parsed.inline_styles {
            if let Ok(sheet) = CssParser::new(inline_css).parse() {
                layouter::css_resolver::append_resolved_styles(
                    Arc::make_mut(&mut self.resolved_styles),
                    layouter::css_resolver::CssResolver::resolve(&sheet),
                );
            }
        }

        self.phase = PagePhase::HtmlParsed;
    }

    pub fn on_css_fetched(&mut self, css: String) {
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

    /// Runs the document's inline scripts once (after CSS is applied).
    ///
    /// If a script mutated the DOM, rebuilds the layout and flags a redraw.
    fn run_scripts_once(&mut self) {
        let scripts = std::mem::take(&mut self.pending_scripts);
        if scripts.is_empty() {
            return;
        }

        let needs_redraw = {
            let Some(js_runtime) = self.js_runtime.as_mut() else {
                return;
            };
            for script in &scripts {
                js_runtime.run_script(script);
            }
            js_runtime.take_needs_redraw()
        };

        if needs_redraw {
            self.update_layout();
            self.needs_redraw = true;
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
            self.update_layout();
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

        let task = layouter::LayoutTask {
            snapshot,
            root,
            resolved_styles: Arc::clone(&self.resolved_styles),
            measurer: self.text_measurer.clone().unwrap(),
            system_color_scheme: self.system_color_scheme,
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
            self.layout_and_info = Some((result.layout, result.info));
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
        self.pending_scripts.clear();
        let (write_back_tx, write_back_rx) = mpsc::channel();
        self.write_back_tx = write_back_tx;
        self.write_back_rx = write_back_rx;
    }

    pub fn set_system_color_scheme(&mut self, scheme: ColorScheme) {
        self.system_color_scheme = scheme;
    }

    pub fn title(&self) -> Option<&String> {
        self.docment_info.as_ref().map(|d| &d.title)
    }

    pub fn relayout(&mut self, viewport: (f32, f32)) {
        let Some((layout, _info)) = self.layout_and_info.as_mut() else {
            return;
        };

        ui_layout::LayoutEngine::layout(layout, viewport.0, viewport.1);
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

fn parse_html(html: &str, document_url: Url) -> ParsedDocument {
    // --- DOM パース ---
    let mut parser = HtmlParser::new(html);
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

    // --- Inline scripts ---
    let scripts = dom.collect_inline_scripts();

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

    #[test]
    fn parse_html_resolves_image_sources_against_base_url() {
        let parsed = parse_html(
            r#"<base href="https://cdn.example/assets/"><img src="logo.png"><img>"#,
            Url::parse("https://example.test/page/index.html").unwrap(),
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
}
