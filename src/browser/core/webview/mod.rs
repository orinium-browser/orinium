use crate::engine::{
    html::parser::{DomTree, Parser as HtmlParser},
    // css::cssom::Parser as CssParser,
    renderer::RenderTree,
    styler::style_tree::StyleTree,
};

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
    pub fn load(&mut self, html_source: &str, _css_source: Option<&str>) {
        // DOM
        let mut parser = HtmlParser::new(html_source);
        let dom_tree = parser.parse();
        self.dom = Some(dom_tree.clone());

        self.title = dom_tree.collect_text_by_tag("title").first().cloned();

        // Style Tree
        let mut style_tree = StyleTree::transform(&dom_tree);
        style_tree.style(&[]);
        let computed_tree = style_tree.compute();

        // Render Tree
        let render_tree = RenderTree::from_computed_tree(&computed_tree);
        self.render = Some(render_tree);

        self.needs_redraw = true;
    }
}
