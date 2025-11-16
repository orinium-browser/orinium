use std::cell::RefCell;
use std::rc::{Rc, Weak};

use super::computed_tree::{ComputedStyle, ComputedStyleNode};
use crate::engine::css::cssom::CssNodeType;
use crate::engine::tree::*;
use crate::html::HtmlNodeType;

#[derive(Debug, Clone)]
pub struct StyleNode {
    html: Weak<RefCell<TreeNode<HtmlNodeType>>>,
    pub style: Option<Style>,
}

impl StyleNode {
    pub fn html(&self) -> Weak<RefCell<TreeNode<HtmlNodeType>>> {
        self.html.clone()
    }
}

#[derive(Debug, Clone)]
pub struct Style;

pub type StyleTree = Tree<StyleNode>;

impl StyleTree {
    pub fn transform(dom: &Tree<HtmlNodeType>) -> StyleTree {
        dom.map_with_node(&|node| StyleNode {
            html: Rc::downgrade(node),
            style: None,
        })
    }

    /// styleを適応させる
    pub fn style(&mut self, _cssoms: &[Tree<CssNodeType>]) -> Self {
        // 仮実装
        self.map(&|node: &StyleNode| StyleNode {
            html: node.html(),
            style: Some(Style),
        })
    }

    pub fn compute(&mut self) -> Tree<ComputedStyleNode> {
        self.map(&|node: &StyleNode| ComputedStyleNode {
            html: node.html(),
            computed: Some(ComputedStyle::compute(node.style.clone().unwrap())),
        })
    }
}
