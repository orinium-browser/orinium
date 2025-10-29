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
    /// ピクセル値として評価（計算済みスタイルで使用）
    pub fn to_px(&self, base: f32) -> f32 {
        match *self {
            Length::Px(px) => px,
            Length::Em(em) => em * base,
            Length::Percent(p) => base * (p / 100.0),
            Length::Auto => base, // 仮の挙動（layout時に解釈）
            Length::None => 0.0,
        }
    }

    /// CSS文字列からLength
    pub fn from_css(value: &str) -> Option<Length> {
        let value = value.trim();
        if value.eq_ignore_ascii_case("auto") {
            return Some(Length::Auto);
        } else if value.ends_with("px") {
            let num_str = &value[..value.len() - 2];
            if let Ok(num) = num_str.parse::<f32>() {
                return Some(Length::Px(num));
            }
        } else if value.ends_with("em") {
            let num_str = &value[..value.len() - 2];
            if let Ok(num) = num_str.parse::<f32>() {
                return Some(Length::Em(num));
            }
        } else if value.ends_with('%') {
            let num_str = &value[..value.len() - 1];
            if let Ok(num) = num_str.parse::<f32>() {
                return Some(Length::Percent(num));
            }
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
            Length::Px(v) => write!(f, "{}px", v),
            Length::Em(v) => write!(f, "{}em", v),
            Length::Percent(v) => write!(f, "{}%", v),
            Length::Auto => write!(f, "auto"),
            Length::None => write!(f, "none"),
        }
    }
}
