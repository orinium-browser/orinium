//! CSS border values
//! (border-width, border-style, border-color, etc.)

use super::color::Color;
use super::length::Length;

/// CSSのborder-styleプロパティで使われる値
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderStyle {
    #[default]
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

/// 単一の辺のborder定義（top, right, bottom, left）
#[derive(Debug, Copy, Clone)]
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
#[derive(Debug, Copy, Clone, Default)]
pub struct Border {
    pub top: BorderSide,
    pub right: BorderSide,
    pub bottom: BorderSide,
    pub left: BorderSide,
}
