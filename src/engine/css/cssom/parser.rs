use std::cell::RefCell;
use std::rc::Rc;

use crate::engine::tree::{Tree, TreeNode};

#[derive(Debug, Clone)]
pub struct CssNodeType {
    // CSSノードの種類をここに定義
}

#[allow(dead_code)]
pub struct Parser<'a> {
    tokenizer: crate::engine::css::cssom::tokenizer::Tokenizer<'a>,
    tree: Tree<CssNodeType>,
    stack: Vec<Rc<RefCell<TreeNode<CssNodeType>>>>,
}
