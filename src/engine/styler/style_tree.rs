use std::cell::RefCell;
use std::rc::{Rc, Weak};

use super::computed_tree::{ComputedStyle, ComputedStyleNode};
use super::ua::default_style_for;

use crate::engine::css::cssom::CssNodeType;
use crate::engine::css::values::{Border, Color, Display, Length};
use crate::engine::tree::*;
use crate::html::{HtmlNodeType, util as html_util};

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

    pub font_size: Option<Length>,
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
    pub fn style(&mut self, _cssoms: &[Tree<CssNodeType>]) {
        self.traverse(&mut |node: &Rc<RefCell<TreeNode<StyleNode>>>| {
            let mut node = node.borrow_mut();
            let node_value = node.value.clone();
            let html_weak = node_value.html.clone();
            let html_rc: Rc<RefCell<TreeNode<HtmlNodeType>>> = html_weak.upgrade().unwrap();
            let html = html_rc.borrow().value.clone();

            // 1. UA デフォルトスタイル
            let mut style = default_style_for(&html);

            // 2. 親スタイルを取得して継承
            let parent_style = node.parent().and_then(|p| p.borrow().value.style.clone());

            if let Some(parent) = parent_style {
                inherit_from_parent(&mut style, &parent);
            }

            if let Some(font_size) = style.font_size {
                style.height = Some(font_size);
            } else {
                style.height = Some(Length::Px(16.0)); // 仮の高さ設定
            }
            style.width = Some(Length::Px(100.0)); // 仮の幅設定

            if let HtmlNodeType::Element { tag_name, .. } = &html {
                match tag_name.as_str() {
                    _ if html_util::is_block_level_element(tag_name) => {
                        style.display = Some(Display::Block);
                    }
                    _ if html_util::is_inline_element(tag_name) => {
                        style.display = Some(Display::Inline);
                    }
                    _ => {}
                }
            }

            if let Some(style_display) = &style.display {
                match style_display {
                    Display::Block => {}
                    Display::Inline => {}
                    Display::None => {}
                }
            }

            node.value.style = Some(style);
        })
    }

    pub fn compute(&mut self) -> Tree<ComputedStyleNode> {
        self.map(&|node: &StyleNode| {
            let style = node.style.clone().unwrap();

            let computed = ComputedStyle::compute(style);

            ComputedStyleNode {
                html: node.html(),
                computed: Some(computed),
            }
        })
    }
}

/// 親スタイルから継承可能プロパティだけをコピー
fn inherit_from_parent(child: &mut Style, parent: &Style) {
    // --- 代表的な継承プロパティ ---
    if child.font_size.is_none() {
        child.font_size = parent.font_size;
    }

    if child.color.is_none() {
        child.color = parent.color;
    }

    // 余白やパディングは継承しない
}
