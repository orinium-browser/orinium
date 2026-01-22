use crate::engine::{
    css::parser::Parser as CssParser,
    html::parser::{DomTree, Parser as HtmlParser},
    layouter::{
        self,
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

#[derive(Debug, PartialEq)]
enum PagePhase {
    Init,
    BeforeHtmlParsing,
    HtmlParsed,
    CssPending,
    CssApplied,
}

pub struct WebView {
    phase: PagePhase,

    docment_info: Option<DocumentInfo>,

    pending_css_urls: Vec<Url>,
    loaded_css: Vec<String>,

    resolved_styles: layouter::css_resolver::ResolvedStyles,
    layout_and_info: Option<(LayoutNode, InfoNode)>,

    needs_redraw: bool,
}

struct DocumentInfo {
    document_url: Url,
    base_url: Url,
    title: String,
    dom: DomTree,
}

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
        }
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
                let measurer = PlatformTextMeasurer::new().unwrap();

                self.update_layout_and_info(measurer);

                // CSS fetch を要求
                for url in &self.pending_css_urls {
                    println!("Fetch requested in WebView: url={}", url);
                    tasks.push(WebViewTask::Fetch {
                        url: url.clone(),
                        kind: FetchKind::Css,
                    });
                }

                self.phase = PagePhase::CssPending;
            }

            PagePhase::CssPending => {
                // CSS が揃うまで待つ
            }

            PagePhase::CssApplied => {
                // 安定状態
            }
        }

        tasks
    }

    pub fn on_html_fetched(&mut self, html: String, document_url: Url) {
        println!("Fetched HTML: {}", document_url);
        let parsed = parse_html(&html, document_url);

        self.pending_css_urls = parsed.style_links;

        let docment_info = DocumentInfo {
            document_url: parsed.document_url,
            base_url: parsed.base_url,
            dom: parsed.dom,
            title: parsed.title,
        };
        self.docment_info = Some(docment_info);

        self.resolved_styles
            .extend(resolve_all_css(&parsed.inline_styles));

        self.phase = PagePhase::HtmlParsed;
    }

    pub fn on_css_fetched(&mut self, css: String) {
        self.loaded_css.push(css);

        if self.loaded_css.len() == self.pending_css_urls.len() {
            print!("Apply");
            self.apply_css_and_relayout();
            self.phase = PagePhase::CssApplied;
            self.needs_redraw = true;
        }
    }

    fn apply_css_and_relayout(&mut self) {
        self.resolved_styles
            .extend(resolve_all_css(&self.loaded_css));

        let measurer = PlatformTextMeasurer::new().unwrap();

        self.update_layout_and_info(measurer);
    }

    fn update_layout_and_info(&mut self, measurer: PlatformTextMeasurer) {
        self.layout_and_info = Some(layouter::build_layout_and_info(
            &self.docment_info.as_ref().unwrap().dom.root,
            &self.resolved_styles,
            &measurer,
            TextStyle {
                font_size: 16.0,
                ..Default::default()
            },
            Vec::new(),
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
        .find_all(|n| n.tag_name() == Some("base".to_string()))
        .iter()
        .filter_map(|node_ref| {
            let html_node = &node_ref.borrow().value;
            let href = html_node.get_attr("href")?;
            document_url.join(&href).ok()
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
    let link_nodes = dom.find_all(|n| n.tag_name() == Some("link".to_string()));
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

fn resolve_all_css(css_sources: &[String]) -> layouter::css_resolver::ResolvedStyles {
    let mut resolved = layouter::css_resolver::ResolvedStyles::default();

    for css in css_sources {
        let sheet = CssParser::new(css).parse().unwrap();

        resolved.extend(layouter::css_resolver::CssResolver::resolve(&sheet));
    }

    resolved
}

pub fn resolve_url(base_url: &Url, path: &str) -> Result<Url, url::ParseError> {
    // absolute URL（scheme を持つ）
    if let Ok(url) = Url::parse(path) {
        return Ok(url);
    }

    // relative URL
    base_url.join(path)
}
