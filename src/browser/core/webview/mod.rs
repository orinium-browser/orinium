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

const USER_AGENT_CSS: &str = include_str!("../../../../resource/user-agent.css");

pub enum WebViewTask {
    Fetch { url: String, kind: FetchKind },
}

pub enum FetchKind {
    Html,
    Css,
}

enum PagePhase {
    BeforeHtmlParsing,
    HtmlParsed,
    CssPending,
    CssApplied,
}

pub struct WebView {
    phase: PagePhase,

    dom: Option<DomTree>,

    title: Option<String>,

    pending_css_urls: Vec<String>,
    loaded_css: Vec<String>,

    resolved_styles: layouter::css_resolver::ResolvedStyles,
    layout_and_info: Option<(LayoutNode, InfoNode)>,

    needs_redraw: bool,
}

struct ParsedDocument {
    dom: DomTree,
    title: Option<String>,
    style_links: Vec<String>,
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
            phase: PagePhase::BeforeHtmlParsing,

            dom: None,

            title: None,

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
            PagePhase::BeforeHtmlParsing => {
                self.resolved_styles
                    .extend(layouter::css_resolver::CssResolver::resolve(
                        &CssParser::new(USER_AGENT_CSS).parse().unwrap(),
                    ));
            }

            PagePhase::HtmlParsed => {
                // Phase 1: UA.css only layout
                let measurer = PlatformTextMeasurer::new().unwrap();

                self.update_layout_and_info(measurer);

                // CSS fetch を要求
                for url in &self.pending_css_urls {
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

    pub fn on_html_fetched(&mut self, html: String, base_url: &str) {
        let parsed = parse_html(&html, base_url);

        self.dom = Some(parsed.dom);
        self.pending_css_urls = parsed.style_links;
        self.title = parsed.title;
        self.resolved_styles
            .extend(resolve_all_css(&parsed.inline_styles));

        self.phase = PagePhase::HtmlParsed;
    }

    pub fn on_css_fetched(&mut self, css: String) {
        self.loaded_css.push(css);

        if self.loaded_css.len() == self.pending_css_urls.len() {
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
            &self.dom.as_ref().unwrap().root,
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

    pub fn title(&self) -> Option<&String> {
        self.title.as_ref()
    }

    /// 現在描画可能な Layout / Info を返す（なければ None）
    pub fn layout_and_info(&self) -> Option<&(LayoutNode, InfoNode)> {
        self.layout_and_info.as_ref()
    }

    pub fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }

    pub fn clear_redraw_flag(&mut self) {
        self.needs_redraw = false;
    }
}

fn parse_html(html: &str, base_url: &str) -> ParsedDocument {
    // --- DOM パース ---
    let mut parser = HtmlParser::new(&html);
    let dom = parser.parse();

    // --- title 抽出 ---
    let title = dom.collect_text_by_tag("title").first().cloned();

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
            let css_url = resolve_url(base_url, &href);
            style_links.push(css_url);
        }
    }

    // --- Inline styles ---
    let inline_styles = dom.collect_text_by_tag("style");

    ParsedDocument {
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

fn resolve_url(base: &str, path: &str) -> String {
    if path.starts_with("http://")
        || path.starts_with("https://")
        || path.starts_with("resource:///")
    {
        return path.to_string();
    }
    let base_url = url::Url::parse(base).unwrap();
    base_url.join(path).unwrap().to_string()
}
