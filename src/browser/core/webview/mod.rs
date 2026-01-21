use std::sync::Arc;

use crate::engine::{
    css::parser::Parser as CssParser,
    html::parser::{DomTree, Parser as HtmlParser},
    layouter::{
        self,
        types::{InfoNode, TextStyle},
    },
};
use ui_layout::LayoutNode;

use super::resource_loader::BrowserResourceLoader;

const USER_AGENT_CSS: &str = include_str!("../../../../resource/user-agent.css");

/// WebView は 1 つのウェブページの表示・レイアウト・描画を管理する構造体です。
///
/// 主に以下の責務を持ちます:
/// - HTML の読み込み・DOM ツリーの構築
/// - CSS の適用・Style Tree の生成
/// - レンダーツリー(RenderTree)の構築とレイアウト計算
/// - DrawCommand の生成による描画準備
/// - スクロールやビューポート等のページ固有の状態管理
///
/// WebView はタブ(Tab)の内部に持たれ、BrowserApp から更新・描画処理が呼ばれます。
/// 1 WebView = 1 ページ(ドキュメント) と対応します。
pub struct WebView {
    pub url: Option<String>,

    pub title: Option<String>,

    // Core trees
    pub dom: Option<DomTree>,
    pub layout_and_info: Option<(LayoutNode, InfoNode)>,

    // Viewport
    pub scroll_x: f32,
    pub scroll_y: f32,

    pub needs_redraw: bool,
}

impl Default for WebView {
    fn default() -> Self {
        Self::new()
    }
}

impl WebView {
    pub fn new() -> Self {
        Self {
            url: None,
            title: None,
            dom: None,
            layout_and_info: None,
            scroll_x: 0.0,
            scroll_y: 0.0,
            needs_redraw: true,
        }
    }

    /// URL を使った本格的なページロード
    ///
    /// - HTML を取得
    /// - DOM パース
    /// - `<link rel="stylesheet">` を解決して CSS を取得
    /// - Style Tree を構築
    /// - Render Tree を構築
    pub fn load_from_url(
        &mut self,
        url: &str,
        net: Arc<BrowserResourceLoader>,
    ) -> anyhow::Result<()> {
        let resp = net
            .fetch(url)
            .map_err(|e| anyhow::Error::msg(e.to_string()))?;

        self.url = Some(dbg!(resp.url));

        // --- HTML をロード ---
        let html_source = String::from_utf8_lossy(&resp.body).to_string();

        // --- DOM パース ---
        let mut parser = HtmlParser::new(&html_source);
        let dom_tree = parser.parse();
        self.dom = Some(dom_tree.clone());

        // --- title 抽出 ---
        self.title = dom_tree.collect_text_by_tag("title").first().cloned();

        // --- CSS リンクを解決 ---
        let mut css_sources: Vec<String> = Vec::new();

        // <link rel="stylesheet" href="...">
        let link_nodes = dom_tree.find_all(|n| n.tag_name() == Some("link".to_string()));

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
                let css_url = resolve_url(url, &href);

                if let Ok(res) = net.fetch(&css_url) {
                    let bytes = res.body;
                    if let Ok(text) = String::from_utf8(bytes) {
                        css_sources.push(text);
                    }
                }
            }
        }

        // style タグから読み取る
        for css_text in dom_tree.collect_text_by_tag("style") {
            css_sources.push(css_text);
        }

        // --- CSSOM を構築 ---
        let mut cssoms = Vec::with_capacity(css_sources.len() + 1);
        cssoms.push(CssParser::new(USER_AGENT_CSS).parse()?);
        for css_text in css_sources {
            let mut css_parser = CssParser::new(&css_text);
            let cssom = css_parser.parse()?;
            cssoms.push(cssom);
        }

        let mut resolved_styles = layouter::css_resolver::ResolvedStyles::new();
        for cssom in cssoms {
            resolved_styles.extend(layouter::css_resolver::CssResolver::resolve(&cssom));
        }

        let measurer = crate::platform::renderer::text_measurer::PlatformTextMeasurer::new();

        // Layout and Info
        self.layout_and_info = Some(layouter::build_layout_and_info(
            &dom_tree.root,
            &resolved_styles,
            &measurer.unwrap(),
            TextStyle {
                font_size: 16.0,
                ..Default::default()
            },
            Vec::new(),
        ));

        log::debug!(
            target: "WebView::LayoutInfo",
            "Layouted the page:\n {:?}",
            self.layout_and_info.as_ref().unwrap()
        );

        self.needs_redraw = true;

        Ok(())
    }

    pub fn move_to(&mut self, href: &str, net: Arc<BrowserResourceLoader>) -> anyhow::Result<()> {
        let base = self
            .url
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("current URL is not set"))?;
        let url = resolve_url(base, href);
        self.load_from_url(&url, net)
    }
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
