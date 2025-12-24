//! Style 型のみを定義
use crate::engine::css::values::{Border, Color, Display, Length};

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

pub trait UADefault {
    /// HTML ノードに対するデフォルト Style を返す
    fn default_style_for(node: &crate::engine::html::parser::HtmlNodeType) -> Self;
}
