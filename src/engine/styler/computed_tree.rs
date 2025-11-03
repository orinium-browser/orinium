//! DOMツリーを元に、ComputedStyle付きのツリーを構築する
//!
//! ここではCSSマッチング・継承・初期値適用を行い、
//! 各ノードに `ComputedStyle` を対応付ける

use std::cell::RefCell;
use std::rc::Rc;

use crate::engine::css::cssom::{CssNodeType, CssValue};
use crate::engine::html::HtmlNodeType;
use crate::engine::tree::{Tree, TreeNode};

use super::computed::ComputedStyle;
use super::matcher;

/// ComputedStyle付きのノード。
#[derive(Debug, Clone)]
pub struct ComputedNode {
    pub html_node: HtmlNodeType, // 元HTMLノード（複製して保持）
    pub style: ComputedStyle,    // 計算済みスタイル
}

/// DOMツリーを解析し、ComputedTreeを生成する。
pub fn compute_styles(
    dom: &Tree<HtmlNodeType>,
    cssoms: &[Tree<CssNodeType>],
) -> Tree<ComputedNode> {
    fn build_node(
        node: &Rc<RefCell<TreeNode<HtmlNodeType>>>,
        cssoms: &[Tree<CssNodeType>],
    ) -> Rc<RefCell<TreeNode<ComputedNode>>> {
        let html_node = node.borrow().value.clone();

        // TODO: matcherモジュールを使って CSS 適用ルールを計算
        let computed_style = ComputedStyle::from_html(&html_node, cssoms);

        let new_node = TreeNode::new(ComputedNode {
            html_node,
            style: computed_style,
        });

        // 再帰的に子ノードを処理
        for child in &node.borrow().children {
            let new_child = build_node(child, cssoms);
            TreeNode::add_child(&new_node, new_child);
        }

        new_node
    }

    Tree {
        root: build_node(&dom.root, cssoms),
    }
}
