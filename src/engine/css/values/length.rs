//! CSS length values (px, em, %, etc.)
//! Used in margin, padding, border-width, etc.

use std::fmt;

/// CSSの長さ単位を表す列挙型
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Length {
    /// 絶対値 (px)
    Px(f32),
    /// 相対値 (em, rem, etc.)
    Em(f32),
    /// パーセンテージ (%)
    Percent(f32),
    /// 自動 (auto)
    Auto,
    /// 未指定
    None,
}

impl Length {
    /// ピクセル値として強制評価
    fn to_px_unwprap(self, base: f32) -> f32 {
        match self {
            Length::Px(px) => px,
            Length::Em(em) => em * base,
            Length::Percent(p) => base * (p / 100.0),
            Length::Auto => {
                panic!("`Length::to_px_unwprap()` should'n be called to `Length::Auto`")
            }
            Length::None => 0.0,
        }
    }

    /// ピクセル値として評価
    /// Autoの場合、Noneを返す
    pub fn to_px(&self, base: f32) -> Option<f32> {
        match self {
            Length::Auto => None,
            _ => Some(self.to_px_unwprap(base)),
        }
    }

    /// CSS文字列からLength
    pub fn from_css(value: &str) -> Option<Length> {
        let value = value.trim();
        if value.eq_ignore_ascii_case("auto") {
            return Some(Length::Auto);
        } else if let Some(num_str) = value.strip_suffix("px") {
            if let Ok(num) = num_str.parse::<f32>() {
                return Some(Length::Px(num));
            }
        } else if let Some(num_str) = value.strip_suffix("em") {
            if let Ok(num) = num_str.parse::<f32>() {
                return Some(Length::Em(num));
            }
        } else if let Some(num_str) = value.strip_suffix('%')
            && let Ok(num) = num_str.parse::<f32>()
        {
            return Some(Length::Percent(num));
        }
        None
    }

    pub fn from_number_and_unit(value: f32, unit: &str) -> Option<Length> {
        match unit {
            "px" => Some(Length::Px(value)),
            "em" => Some(Length::Em(value)),
            "%" => Some(Length::Percent(value)),
            _ => None,
        }
    }
}

impl Default for Length {
    fn default() -> Self {
        Length::Px(0.0)
    }
}

impl fmt::Display for Length {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Length::Px(v) => write!(f, "{v}px"),
            Length::Em(v) => write!(f, "{v}em"),
            Length::Percent(v) => write!(f, "{v}%"),
            Length::Auto => write!(f, "auto"),
            Length::None => write!(f, "none"),
        }
    }
}
