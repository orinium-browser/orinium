//! DOM/CSSOM を統合し、各ノードの最終スタイル（ComputedStyle）を決定する。

pub mod matcher;
pub mod style_tree;

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use self::style_tree::*;
use crate::engine::tree::*;
use crate::html::HtmlNodeType;

#[derive(Debug, Clone)]
pub struct ComputedStyleNode {
    html: Weak<RefCell<TreeNode<HtmlNodeType>>>,
    style: Option<ComputedStyle>,
}

#[derive(Debug, Clone)]
pub struct ComputedStyle;
