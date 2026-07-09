//! ブラウザのwebview機能。タスクとレンダリング情報の管理を行う。

use crate::engine::{
    css::{self, parser::Parser as CssParser},
    html::parser::{DomTree, Parser as HtmlParser},
    layouter::{
        self, InheritedCss,
        types::{InfoNode, TextStyle},
    },
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
}

#[derive(Debug)]
pub struct WebView {
    phase: PagePhase,

    docment_info: Option<DocumentInfo>,

    pending_css_urls: Vec<Url>,
    loaded_css: Vec<String>,

    resolved_styles: layouter::css_resolver::ResolvedStyles,
    layout_and_info: Option<(LayoutNode, InfoNode)>,

    needs_redraw: bool,

    text_measurer: Option<PlatformTextMeasurer>,

    css_processor: css::processor::CssProcessor,
    css_strategy: CssApplicationStrategy,
    css_results_expected: usize,
    css_results_received: usize,
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
    pub dom: DomTree,
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
struct ParsedDocument {
    document_url: Url,
    base_url: Url,
    dom: DomTree,
    title: String,
    style_links: Vec<Url>,
    inline_styles: Vec<String>,
}

impl Default for WebView {
    fn default() -> Self {
        Self::new()
    }
}

impl WebView {
    pub fn new() -> Self {
        Self {
            phase: PagePhase::Init,

            docment_info: None,

            pending_css_urls: Vec::new(),
            loaded_css: Vec::new(),

            resolved_styles: layouter::css_resolver::ResolvedStyles::default(),
            layout_and_info: None,

            needs_redraw: false,

            text_measurer: None,

            css_processor: css::processor::CssProcessor::new(),
            css_strategy: CssApplicationStrategy::Incremental,
            css_results_expected: 0,
            css_results_received: 0,
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
                self.resolved_styles
                    .extend(layouter::css_resolver::CssResolver::resolve(
                        &CssParser::new(USER_AGENT_CSS).parse().unwrap(),
                    ));

                tasks.push(WebViewTask::AskTabHtml);

                self.phase = PagePhase::BeforeHtmlParsing;
            }

            PagePhase::BeforeHtmlParsing => {}

            PagePhase::HtmlParsed => {
                // Phase 1: UA.css only layout
                self.ensure_text_measurer();
                self.update_layout();

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
                // 安定状態
            }
        }

        tasks
    }

    pub fn on_html_fetched(&mut self, html: String, document_url: Url) {
        log::info!("Fetched HTML: {}", document_url);
        let parsed = parse_html(&html, document_url);

        self.pending_css_urls = parsed.style_links;
        self.css_results_expected = self.pending_css_urls.len();

        let docment_info = DocumentInfo {
            document_url: parsed.document_url,
            base_url: parsed.base_url,
            dom: parsed.dom,
            title: parsed.title,
        };
        self.docment_info = Some(docment_info);

        for inline_css in &parsed.inline_styles {
            if let Ok(sheet) = CssParser::new(inline_css).parse() {
                self.resolved_styles
                    .extend(layouter::css_resolver::CssResolver::resolve(&sheet));
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

    /// Update page (e.g. DOM changed)
    ///
    /// This is a stub method for now.
    pub fn update_page(&mut self) {
        self.ensure_text_measurer();
        self.update_layout();
    }

    fn apply_resolved_styles_and_relayout(
        &mut self,
        resolved: layouter::css_resolver::ResolvedStyles,
    ) {
        self.resolved_styles.extend(resolved);
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

    fn ensure_text_measurer(&mut self) {
        if self.text_measurer.is_none() {
            self.text_measurer = Some(PlatformTextMeasurer::new().unwrap());
        }
    }

    fn build_layout(
        docment_info: &DocumentInfo,
        resolved_styles: &layouter::css_resolver::ResolvedStyles,
        measurer: &PlatformTextMeasurer,
    ) -> (LayoutNode, InfoNode) {
        layouter::build_layout_and_info(
            &docment_info.dom.root,
            resolved_styles,
            measurer,
            InheritedCss {
                text_style: TextStyle {
                    font_size: 16.0,
                    ..Default::default()
                },
            },
            Vec::new(),
        )
    }

    fn update_layout(&mut self) {
        let doc_info = match self.docment_info.as_ref() {
            Some(d) => d,
            None => return,
        };

        self.layout_and_info = Some(Self::build_layout(
            doc_info,
            &self.resolved_styles,
            self.text_measurer.as_ref().unwrap(),
        ));
        self.needs_redraw = true;
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
        self.loaded_css.clear();
        self.resolved_styles.clear();
        self.layout_and_info = None;

        self.needs_redraw = false;

        self.css_processor = css::processor::CssProcessor::new();
        self.css_results_expected = 0;
        self.css_results_received = 0;
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
    }

    pub fn clear_redraw_flag(&mut self) {
        self.needs_redraw = false;
    }
}

fn parse_html(html: &str, document_url: Url) -> ParsedDocument {
    // --- DOM パース ---
    let mut parser = HtmlParser::new(html);
    let dom = parser.parse();

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

    ParsedDocument {
        document_url,
        base_url,
        dom,
        title,
        style_links,
        inline_styles,
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
