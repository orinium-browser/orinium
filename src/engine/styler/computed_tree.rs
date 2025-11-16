use super::style_tree::Style;

use crate::engine::css::values::{Border, Color, Display, Length};
use crate::engine::tree::*;
use crate::html::HtmlNodeType;
use std::cell::RefCell;
use std::rc::Weak;

/// 計算済みスタイルを持つノード
#[derive(Debug, Clone)]
pub struct ComputedStyleNode {
    pub html: Weak<RefCell<TreeNode<HtmlNodeType>>>,
    pub computed: Option<ComputedStyle>,
}

/// 計算済みスタイル
#[derive(Debug, Clone)]
pub struct ComputedStyle {
    pub display: Display,
    pub width: Option<Length>,
    pub height: Option<Length>,

    pub margin_top: Length,
    pub margin_right: Length,
    pub margin_bottom: Length,
    pub margin_left: Length,

    pub padding_top: Length,
    pub padding_right: Length,
    pub padding_bottom: Length,
    pub padding_left: Length,

    pub color: Option<Color>,
    pub background_color: Option<Color>,

    pub border: Option<Border>,
}

impl ComputedStyle {
    /// Style から計算済みスタイルを作る
    pub fn compute(style: Style) -> Self {
        Self {
            display: style.display.unwrap_or(Display::Inline),
            width: style.width,
            height: style.height,

            // margin/padding は None の場合 0 にフォールバック
            margin_top: style.margin_top.unwrap_or(Length::Px(0.0)),
            margin_right: style.margin_right.unwrap_or(Length::Px(0.0)),
            margin_bottom: style.margin_bottom.unwrap_or(Length::Px(0.0)),
            margin_left: style.margin_left.unwrap_or(Length::Px(0.0)),

            padding_top: style.padding_top.unwrap_or(Length::Px(0.0)),
            padding_right: style.padding_right.unwrap_or(Length::Px(0.0)),
            padding_bottom: style.padding_bottom.unwrap_or(Length::Px(0.0)),
            padding_left: style.padding_left.unwrap_or(Length::Px(0.0)),

            color: style.color,
            background_color: style.background_color,

            border: style.border,
        }
    }
}
