//! CSS Length types.
//!
//! This module provides a type-safe representation of CSS length values such as
//! `10px`, `50%`, or keyword-based values like `auto`.
//!
//! The core idea is to model a CSS length as either:
//! - a numeric value with a unit, or
//! - a keyword.

use std::fmt;

/// CSS length units such as `px`, `em`, `%`, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthUnit {
    Px,
    Em,
    Rem,
    Vw,
    Vh,
    Percent,
}

impl LengthUnit {
    /// Returns the CSS string for this unit.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Px => "px",
            Self::Em => "em",
            Self::Rem => "rem",
            Self::Vw => "vw",
            Self::Vh => "vh",
            Self::Percent => "%",
        }
    }
}

/// Keyword-based CSS length values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthKeyword {
    /// The `auto` keyword.
    Auto,
    /// The `min-content` keyword.
    MinContent,
    /// The `max-content` keyword.
    MaxContent,
    /// The `fit-content` keyword.
    FitContent,
}

/// A CSS length value.
///
/// This type represents either:
/// - a numeric value with a unit (e.g. `10px`, `2.5em`, `100%`), or
/// - a keyword (e.g. `auto`).
///
/// This mirrors how CSS defines length values in specifications.
#[derive(Debug, Clone, PartialEq, Copy)]
pub enum Length {
    /// A numeric value with a unit.
    Value(f32, LengthUnit),
    /// A keyword-based length.
    Keyword(LengthKeyword),
}

impl Length {
    /// Creates a `px` length.
    pub fn px(value: f32) -> Self {
        Self::Value(value, LengthUnit::Px)
    }

    /// Creates an `em` length.
    pub fn em(value: f32) -> Self {
        Self::Value(value, LengthUnit::Em)
    }

    /// Creates a `rem` length.
    pub fn rem(value: f32) -> Self {
        Self::Value(value, LengthUnit::Rem)
    }

    /// Creates a `vw` length.
    pub fn vw(value: f32) -> Self {
        Self::Value(value, LengthUnit::Vw)
    }

    /// Creates a `vh` length.
    pub fn vh(value: f32) -> Self {
        Self::Value(value, LengthUnit::Vh)
    }

    /// Creates a percentage length.
    pub fn percent(value: f32) -> Self {
        Self::Value(value, LengthUnit::Percent)
    }

    /// Creates the `auto` keyword length.
    pub fn auto() -> Self {
        Self::Keyword(LengthKeyword::Auto)
    }

    /// Returns true if this value is a keyword.
    pub fn is_keyword(&self) -> bool {
        matches!(self, Self::Keyword(_))
    }

    /// Returns the numeric value and unit if this is a numeric length.
    pub fn as_value(&self) -> Option<(f32, LengthUnit)> {
        match self {
            Self::Value(v, u) => Some((*v, *u)),
            _ => None,
        }
    }

    /// Parses a single CSS length token.
    ///
    /// Examples:
    /// - `"10px"`
    /// - `"2.5em"`
    /// - `"100%"`
    /// - `"auto"`
    pub fn from_css(input: &str) -> Option<Self> {
        let s = input.trim();

        // --- keywords ---
        let keyword = match s {
            "auto" => Some(LengthKeyword::Auto),
            "min-content" => Some(LengthKeyword::MinContent),
            "max-content" => Some(LengthKeyword::MaxContent),
            "fit-content" => Some(LengthKeyword::FitContent),
            _ => None,
        };

        if let Some(k) = keyword {
            return Some(Self::Keyword(k));
        }

        // --- number + unit ---
        // Split numeric part and unit part
        let split = s
            .chars()
            .position(|c| !matches!(c, '0'..='9' | '.' | '-' | '+'));

        let idx = split?;
        let (num_str, unit_str) = s.split_at(idx);

        let value: f32 = num_str.parse().ok()?;

        let unit = match unit_str {
            "px" => LengthUnit::Px,
            "em" => LengthUnit::Em,
            "rem" => LengthUnit::Rem,
            "vw" => LengthUnit::Vw,
            "vh" => LengthUnit::Vh,
            "%" => LengthUnit::Percent,
            _ => return None,
        };

        Some(Self::Value(value, unit))
    }
}

impl fmt::Display for Length {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Length::Value(value, unit) => write!(f, "{}{}", value, unit.as_str()),
            Length::Keyword(keyword) => {
                let s = match keyword {
                    LengthKeyword::Auto => "auto",
                    LengthKeyword::MinContent => "min-content",
                    LengthKeyword::MaxContent => "max-content",
                    LengthKeyword::FitContent => "fit-content",
                };
                write!(f, "{}", s)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_numeric_lengths() {
        assert_eq!(Length::px(10.0).to_string(), "10px");
        assert_eq!(Length::em(2.5).to_string(), "2.5em");
        assert_eq!(Length::percent(50.0).to_string(), "50%");
    }

    #[test]
    fn display_keyword_lengths() {
        assert_eq!(Length::auto().to_string(), "auto");
        assert_eq!(
            Length::Keyword(LengthKeyword::MinContent).to_string(),
            "min-content"
        );
    }
}
