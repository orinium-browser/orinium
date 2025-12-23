use std::cell::RefCell;
use std::rc::{Rc, Weak};

use super::computed_tree::{ComputedStyle, ComputedStyleNode};
use super::ua::default_style_for;
use log;

use super::matcher::selector_matches_on_node;
use crate::engine::css::cssom::{CssNodeType, CssValue};
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
    pub fn style(&mut self, cssoms: &[Tree<CssNodeType>]) {
        self.traverse(&mut |node: &Rc<RefCell<TreeNode<StyleNode>>>| {
            let mut node = node.borrow_mut();
            let node_value = node.value.clone();
            let html_weak = node_value.html.clone();
            let html_rc: Rc<RefCell<TreeNode<HtmlNodeType>>> = html_weak.upgrade().unwrap();
            let html = html_rc.borrow().value.clone();

            // 1. UA デフォルトスタイル
            let mut style = default_style_for(&html);
            log::debug!(target: "Styler::StyleTree", "UA default style for node={:?}: {:?}", html, style);

            // 2. 親スタイルを取得して継承
            let parent_node = node.parent();

            if let Some(parent_rc) = parent_node.clone() {
                inherit_from_parent_from_node(&mut style, &parent_rc);
                log::debug!(target: "Styler::StyleTree", "After inheriting from parent for node={:?}: {:?}", html, style);
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

            log::debug!(target: "Styler::StyleTree", "Before applying user styles for node={:?}: {:?}", html, style);
            // 3. User stylesheets (cssoms) を走査してルールを適用
            for css in cssoms {
                // ルート直下や再帰的に Rule ノードが存在するので traverse で探す
                css.traverse(&mut |css_node_rc| {
                    let css_node = css_node_rc.borrow();
                    match &css_node.value {
                        CssNodeType::Rule { selectors } => {
                            // この rule の宣言（子ノード）を見て適用する
                            for sel in selectors {
                                if selector_matches_on_node(sel.as_str(), &html_rc) {
                                    // 要素ノードのみ処理する
                                    match &html {
                                        HtmlNodeType::Element { .. } | HtmlNodeType::Document => {
                                            log::debug!(target: "Styler::StyleTree::CSS", "Selector matched '{}' on node={:?}", sel, html);
                                            // この rule applies -> 子の Declaration を走査して適用
                                            for child in css_node_rc.borrow().children().iter() {
                                                let child_b = child.borrow();
                                                if let CssNodeType::Declaration { name, value } = &child_b.value {
                                                    match name.as_str() {
                                                        "color" => {
                                                            if let CssValue::Color(c) = value {
                                                                let old = style.color;
                                                                style.color = Some(*c);
                                                                log::debug!(target: "Styler::StyleTree::CSS", "Applied 'color': {:?} -> {:?} (node={:?})", old, style.color, html);
                                                            }
                                                        }
                                                        "background-color" => {
                                                            if let CssValue::Color(c) = value {
                                                                let old = style.background_color;
                                                                style.background_color = Some(*c);
                                                                log::debug!(target: "Styler::StyleTree::CSS", "Applied 'background-color': {:?} -> {:?} (node={:?})", old, style.background_color, html);
                                                            }
                                                        }
                                                        "width" => {
                                                            if let CssValue::Length(l) = value {
                                                                let old = style.width;
                                                                style.width = Some(*l);
                                                                log::debug!(target: "Styler::StyleTree::CSS", "Applied 'width': {:?} -> {:?} (node={:?})", old, style.width, html);
                                                            }
                                                        }
                                                        "height" => {
                                                            if let CssValue::Length(l) = value {
                                                                let old = style.height;
                                                                style.height = Some(*l);
                                                                log::debug!(target: "Styler::StyleTree::CSS", "Applied 'height': {:?} -> {:?} (node={:?})", old, style.height, html);
                                                            }
                                                        }
                                                        "display" => {
                                                            if let CssValue::Keyword(k) = value {
                                                                let old = style.display;
                                                                match k.as_str() {
                                                                    "block" => { style.display = Some(Display::Block) }
                                                                    "inline" => { style.display = Some(Display::Inline) }
                                                                    "none" => { style.display = Some(Display::None) }
                                                                    _ => {}
                                                                }
                                                                log::debug!(target: "Styler::StyleTree::CSS", "Applied 'display': {:?} -> {:?} (node={:?})", old, style.display, html);
                                                            }
                                                        }
                                                        "font-size" => {
                                                            if let CssValue::Length(l) = value {
                                                                let old = style.font_size;
                                                                style.font_size = Some(*l);
                                                                log::debug!(target: "Styler::StyleTree::CSS", "Applied 'font-size': {:?} -> {:?} (node={:?})", old, style.font_size, html);
                                                            }
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            }
                                        }
                                        _ => {
                                            // 非要素ノード（Text など）は無視
                                        }
                                    }
                                }
                            }
                        }
                        CssNodeType::AtRule { .. } => {}
                        _ => {}
                    }
                });
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

// 親ノードから継承する（font-size は計算済み px 値で継承する）
fn inherit_from_parent_from_node(
    child: &mut Style,
    parent_node: &Rc<RefCell<TreeNode<StyleNode>>>,
) {
    // color はそのまま継承
    if child.color.is_none()
        && let Some(parent_style) = parent_node.borrow().value.style.clone()
    {
        child.color = parent_style.color;
    }

    // font-size は "computed" として px に解決して継承する
    if child.font_size.is_none() {
        // 親ノードのフォントサイズを px で解決して設定
        let resolved = resolve_font_size_px_from_node(parent_node);
        child.font_size = Some(Length::Px(resolved));
    }
}

// ノードから font-size を再帰的に解決して px を返す（見つからなければ 16px フォールバック）
fn resolve_font_size_px_from_node(node: &Rc<RefCell<TreeNode<StyleNode>>>) -> f32 {
    // デフォルトベース
    const DEFAULT_FONT_PX: f32 = 16.0;

    // まずこのノードの style に font_size があるか確認
    if let Some(style) = node.borrow().value.style.clone()
        && let Some(length) = style.font_size
    {
        match length {
            Length::Px(px) => return px,
            Length::Em(em) => {
                // base を親から解決
                if let Some(parent) = node.borrow().parent() {
                    let base = resolve_font_size_px_from_node(&parent);
                    return Length::Em(em).to_px(base);
                } else {
                    return Length::Em(em).to_px(DEFAULT_FONT_PX);
                }
            }
            Length::Percent(p) => {
                if let Some(parent) = node.borrow().parent() {
                    let base = resolve_font_size_px_from_node(&parent);
                    return Length::Percent(p).to_px(base);
                } else {
                    return Length::Percent(p).to_px(DEFAULT_FONT_PX);
                }
            }
            _ => {}
        }
    }

    // 見つからなければ祖先を辿る
    if let Some(parent) = node.borrow().parent() {
        return resolve_font_size_px_from_node(&parent);
    }

    DEFAULT_FONT_PX
}
