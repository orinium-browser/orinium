use std::cell::RefCell;
use std::rc::{Rc, Weak};

use super::style_tree::Style;
use crate::engine::tree::*;
use crate::html::HtmlNodeType;

#[derive(Debug, Clone)]
pub struct ComputedStyleNode {
    pub html: Weak<RefCell<TreeNode<HtmlNodeType>>>,
    pub computed: Option<ComputedStyle>,
}

#[derive(Debug, Clone)]
pub struct ComputedStyle;

impl ComputedStyle {
    /// スタイルを計算
    pub fn compute(_style: Style) -> Self {
        Self
    }
}
