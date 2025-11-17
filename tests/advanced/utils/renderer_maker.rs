//! render::new までやるツール
//! DOMツリーの構築、スタイル計算、レンダーツリーの構築までを行うユーティリティ

use orinium_browser::engine::html::parser::Parser as HtmlParser;
use orinium_browser::engine::css::cssom::parser::Parser as CssParser;
use orinium_browser::engine::styler::style_tree::StyleTree;
use orinium_browser::engine::renderer::Renderer;

pub fn renderer_maker(html: &str, css: &str) -> orinium::engine::renderer::Renderer {
    // HTMLをパース
    let mut html_parser = HtmlParser::new(html);
    let dom_tree = html_parser.parse();

    // CSSをパース
    let mut css_parser = CssParser::new(css);
    let cssom = css_parser.parse().expect("Failed to parse CSS");

    // スタイルツリーを構築
    let mut style_tree = StyleTree::transform(&dom_tree);
    style_tree = style_tree.style(&cssom.rules);

    // レンダーツリーを構築
    let render_tree = orinium::engine::renderer::Renderer::new(&dom_tree, &style_tree);

    // rendererを作成
    let renderer = Renderer::new(800.0, 600.0);

    renderer
}
