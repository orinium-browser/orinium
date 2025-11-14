//! Stylerの挙動確認に為に作りました

mod utils;

use orinium_browser::engine::styler::style_tree::Style;
use utils::test_dom;

#[test]
fn test_dom_to_style() {
    let dom = test_dom();
    let style_tree = Style::transform(&dom);
    println!("{}", style_tree);
}
