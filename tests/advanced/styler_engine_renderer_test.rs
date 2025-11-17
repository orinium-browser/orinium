//! Stylerとengine::rendererの挙動確認に為に作りました

mod utils;

use orinium_browser::engine::styler::style_tree::StyleTree;

#[test]
fn test_style_to_be_computed() {
    let dom = utils::test_dom();
    println!("{}", dom);
    let mut style_tree = StyleTree::transform(&dom);
    println!("{}", style_tree);
    style_tree = style_tree.style(&[]);
    println!("{}", style_tree);
    let computed_tree = style_tree.compute();
    println!("{}", computed_tree);
}
