//! 計算済みスタイル（ComputedStyle）
//! CSSの継承・初期値などを反映した最終スタイル

use std::cell::RefCell;
use std::rc::Rc;

use crate::engine::css::cssom::{CssNodeType, CssValue};
use crate::engine::html::HtmlNodeType;

#[derive(Debug)]
pub struct ComputedNode {
    pub html_node: Rc<RefCell<HtmlNodeType>>,
    pub style: ComputedStyle,
}

#[derive(Debug, Clone, Default)]
pub struct ComputedStyle {
    pub display: Option<String>,
    pub color: Option<String>,
    pub background_color: Option<String>,
    // ここにどんどん増やす
}

impl ComputedStyle {
    pub fn from_html(
        node: Rc<RefCell<HtmlNodeType>>,
        cssoms: &[crate::engine::tree::Tree<CssNodeType>],
    ) -> Self {
        // TODO: selectorマッチ → cascade → inheritance
        // まずはデフォルトを返す簡易実装
        ComputedStyle {
            display: Some("block".into()),
            color: Some("black".into()),
            background_color: Some("transparent".into()),
        }
    }
}
