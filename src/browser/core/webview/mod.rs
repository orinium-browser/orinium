use std::sync::Arc;

use crate::engine::{
    css::cssom::Parser as CssParser,
    html::parser::{DomTree, Parser as HtmlParser},
    renderer::RenderTree,
    styler::style_tree::StyleTree,
};

use crate::platform::network::NetworkCore;

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
    pub style: Option<StyleTree>,
    pub render: Option<RenderTree>,

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
            style: None,
            render: None,
            scroll_x: 0.0,
            scroll_y: 0.0,
            needs_redraw: true,
        }
    }

    /// ロード → DOM/CSS/Style/Render のフルパイプライン
    ///
    /// TODO:
    /// - dom_tree をクローンするコストを削減
    /// - cssソースの適応
    /// - cssをstyle element から適応
    pub fn load_from_raw_source(&mut self, html_source: &str, _css_source: Option<&str>) {
        // DOM
        let mut parser = HtmlParser::new(html_source);
        let dom_tree = parser.parse();
        self.dom = Some(dom_tree.clone());

        self.title = dom_tree.collect_text_by_tag("title").first().cloned();

        // Style Tree
        let mut style_tree = StyleTree::transform(&dom_tree);
        style_tree.style(&[]);
        let computed_tree = style_tree.compute();

        let measurer = crate::platform::renderer::text_measurer::PlatformTextMeasurer::new();
        let render_tree = match measurer {
            Ok(measurer) => computed_tree.layout_with_measurer(&measurer, 800.0, 600.0),
            Err(e) => {
                println!("Failed to create PlatformTextMeasurer: {}", e);
                computed_tree.layout_with_fallback(800.0, 600.0)
            }
        };
        
        // Scrollable でラップ
        let render_tree = render_tree.wrap_in_scrollable(0.0, 0.0, 800.0, 600.0);

        self.render = Some(render_tree);

        self.needs_redraw = true;
    }

    /// URL を使った本格的なページロード
    ///
    /// - HTML を取得
    /// - DOM パース
    /// - `<link rel="stylesheet">` を解決して CSS を取得
    /// - Style Tree を構築
    /// - Render Tree を構築
    pub async fn load_from_url(&mut self, url: &str, net: Arc<NetworkCore>) -> anyhow::Result<()> {
        // --- HTML をロード ---
        let html_bytes = net
            .fetch_url(url)
            .await
            .map_err(|e| anyhow::Error::msg(e.to_string()))?;

        let html_source = String::from_utf8_lossy(&html_bytes.body).to_string();

        // --- DOM パース ---
        let mut parser = HtmlParser::new(&html_source);
        let dom_tree = parser.parse();
        self.dom = Some(dom_tree.clone());

        // --- title 抽出 ---
        self.title = dom_tree.collect_text_by_tag("title").first().cloned();

        // --- CSS リンクを解決 ---
        let mut css_sources: Vec<String> = Vec::new();

        // <link rel="stylesheet" href="...">
        let link_nodes: Vec<_> = {
            let root = dom_tree.root.borrow();
            root.find_children_by(|n| n.tag_name() == Some("link".to_string()))
                .into_iter()
                .collect()
        };

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

                if let Ok(res) = net.fetch_url(&css_url).await {
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
        let mut cssoms = vec![];
        for css_text in css_sources {
            let mut css_parser = CssParser::new(&css_text);
            let cssom = css_parser.parse()?;
            cssoms.push(cssom);
        }

        // --- Style Tree を構築 ---
        let mut style_tree = StyleTree::transform(&dom_tree);
        style_tree.style(&cssoms);
        let computed_tree = style_tree.compute();

        // --- Render Tree ---
        let measurer = crate::platform::renderer::text_measurer::PlatformTextMeasurer::new();
        let render_tree = match measurer {
            Ok(measurer) => computed_tree.layout_with_measurer(&measurer, 800.0, 600.0),
            Err(e) => {
                println!("Failed to create PlatformTextMeasurer: {}", e);
                computed_tree.layout_with_fallback(800.0, 600.0)
            }
        };

        // Scrollable でラップ
        let render_tree = render_tree.wrap_in_scrollable(0.0, 0.0, 800.0, 600.0);

        self.render = Some(render_tree);

        self.needs_redraw = true;

        Ok(())
    }

    pub fn scroll_page(&mut self, delta_x: f32, delta_y: f32) {
        self.scroll_x += delta_x;
        self.scroll_y += delta_y;
        fn scroll_scrollable(
            node: &std::rc::Rc<
                std::cell::RefCell<
                    crate::engine::tree::TreeNode<crate::engine::renderer::RenderNode>,
                >,
            >,
            delta_x: f32,
            delta_y: f32,
        ) {
            let mut node_borrow = node.borrow_mut();
            if let crate::engine::renderer::NodeKind::Scrollable {
                scroll_offset_x,
                scroll_offset_y,
                ..
            } = &mut node_borrow.value.kind
            {
                *scroll_offset_x += delta_x;
                *scroll_offset_y += delta_y;
            } else {
                panic!("scroll_page called on non-scrollable node; this should not happen");
            }
        }
        if let Some(render_tree) = &self.render {
            scroll_scrollable(&render_tree.root, delta_x, delta_y);
        }
        self.needs_redraw = true;
    }
}

fn resolve_url(base: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }
    let base_url = url::Url::parse(base).unwrap();
    base_url.join(path).unwrap().to_string()
}
