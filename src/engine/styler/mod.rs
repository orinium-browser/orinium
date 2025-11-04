//! DOM/CSSOM を統合し、各ノードの最終スタイル（ComputedStyle）を決定する。

pub mod matcher;

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use crate::engine::tree::*;
use crate::html::HtmlNodeType;

#[derive(Debug, Clone)]
pub struct ComputedStyleNode {
    html: Weak<RefCell<TreeNode<HtmlNodeType>>>,
    style: Option<ComputedStyle>,
}

#[derive(Debug, Clone)]
pub struct ComputedStyle;

impl ComputedStyleNode {
    pub fn transform(dom: &Tree<HtmlNodeType>) -> Tree<ComputedStyleNode> {
        dom.map_with_node(&|node| ComputedStyleNode {
            html: Rc::downgrade(node),
            style: None,
        })
    }
}
