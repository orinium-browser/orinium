//! CSS border values
//! (border-width, border-style, border-color, etc.)

use super::color::Color;
use super::length::Length;

/// CSSのborder-styleプロパティで使われる値
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderStyle {
    None,
    Solid,
    Dashed,
    Dotted,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
}

impl Default for BorderStyle {
    fn default() -> Self {
        BorderStyle::None
    }
}

/// 単一の辺のborder定義（top, right, bottom, left）
#[derive(Debug, Clone)]
pub struct BorderSide {
    pub width: Length,
    pub style: BorderStyle,
    pub color: Color,
}

impl Default for BorderSide {
    fn default() -> Self {
        Self {
            width: Length::Px(0.0),
            style: BorderStyle::None,
            color: Color::BLACK,
        }
    }
}

/// 全体のborderプロパティを表す構造体
#[derive(Debug, Clone, Default)]
pub struct Border {
    pub top: BorderSide,
    pub right: BorderSide,
    pub bottom: BorderSide,
    pub left: BorderSide,
}
