use std::cell::RefCell;
use std::rc::{Rc, Weak};

use super::computed_tree::{ComputedStyle, ComputedStyleNode};
use crate::engine::css::cssom::CssNodeType;
use crate::engine::css::values::{Color, Display, Length, Border};
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


#[derive(Debug, Clone, Default)]
pub struct Style {
    pub display: Option<Display>,
    pub width: Option<Length>,
    pub height: Option<Length>,

    pub margin_top: Option<Length>,
    pub margin_right: Option<Length>,
    pub margin_bottom: Option<Length>,
    pub margin_left: Option<Length>,

    pub padding_top: Option<Length>,
    pub padding_right: Option<Length>,
    pub padding_bottom: Option<Length>,
    pub padding_left: Option<Length>,

    pub color: Option<Color>,
    pub background_color: Option<Color>,

    pub border: Option<Border>,
}

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
        self.map(&|node: &StyleNode| {
            let html = node.html();
            let style = Style::default();

            StyleNode {
                html,
                style: Some(style),
            }
        })
    }

    pub fn compute(&mut self) -> Tree<ComputedStyleNode> {
        self.map(&|node: &StyleNode| ComputedStyleNode {
            html: node.html(),
            computed: Some(ComputedStyle::compute(node.style.clone().unwrap())),
        })
    }
}
