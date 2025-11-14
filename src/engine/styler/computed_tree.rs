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
