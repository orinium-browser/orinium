use std::cell::RefCell;
use std::rc::{Rc, Weak};

use crate::engine::tree::*;
use crate::html::HtmlNodeType;

#[derive(Debug, Clone)]
pub struct StyleNode {
    html: Weak<RefCell<TreeNode<HtmlNodeType>>>,
    style: Option<Style>,
}

#[derive(Debug, Clone)]
pub struct Style;

impl Style {
    pub fn transform(dom: &Tree<HtmlNodeType>) -> Tree<StyleNode> {
        dom.map_with_node(&|node| StyleNode {
            html: Rc::downgrade(node),
            style: None,
        })
    }

    pub fn compute(&mut self) {}
}
