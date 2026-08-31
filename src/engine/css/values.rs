//! CSSの値を表す構造体と列挙型

pub type CssIdent = smol_str::SmolStr;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Unit {
    Px,
    Cm,
    Mm,
    In,
    Pt,
    Pc,

    Em,
    Rem,

    Percent,

    Vw,
    Vh,
    Vmin,
    Vmax,

    Deg,
    Fr,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CssValue {
    Keyword(CssIdent), // e.g. auto, none
    Length(f32, Unit), // e.g. 10px
    Number(f32),       // e.g. 1.5
    String(String),    // e.g. "http"
    Color(String),     // e.g. #fff, #1f1f11
    /// e.g. `rgb(255,0,0)`.
    ///
    /// Arguments are comma-separated; within each argument, whitespace
    /// separates individual components. The result is a two-level structure:
    /// the outer `Vec` holds comma-separated arguments, and each inner `Vec`
    /// holds the whitespace-separated components of that argument.
    Function(String, Vec<Vec<CssValue>>),
    List(Vec<CssValue>), // e.g. 100px auto
}

impl CssValue {
    /// Colorの文字列からRGBAタプルを返す
    pub fn to_rgba_tuple(&self) -> Option<(u8, u8, u8, u8)> {
        match self {
            CssValue::Color(s) => parse_color(&format!("#{}", s)),
            _ => None,
        }
    }
}

impl Unit {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Unit::Px => "px",
            Unit::Cm => "cm",
            Unit::Mm => "mm",
            Unit::In => "in",
            Unit::Pt => "pt",
            Unit::Pc => "pc",

            Unit::Em => "em",
            Unit::Rem => "rem",

            Unit::Percent => "%",

            Unit::Vw => "vw",
            Unit::Vh => "vh",
            Unit::Vmin => "vmin",
            Unit::Vmax => "vmax",

            Unit::Deg => "deg",
            Unit::Fr => "fr",
        }
    }
}

/// Renders values back to their CSS source text, e.g. `10px`, `rgb(1, 2, 3)`
/// or `100px auto`. Used by the DevTools style panels.
impl std::fmt::Display for CssValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CssValue::Keyword(keyword) => f.write_str(keyword),
            CssValue::Length(value, unit) => write!(f, "{value}{}", unit.as_str()),
            CssValue::Number(value) => write!(f, "{value}"),
            CssValue::String(value) => write!(f, "\"{value}\""),
            CssValue::Color(value) => write!(f, "#{value}"),
            CssValue::Function(name, arguments) => {
                let arguments = arguments
                    .iter()
                    .map(|argument| {
                        argument
                            .iter()
                            .map(CssValue::to_string)
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "{name}({arguments})")
            }
            CssValue::List(values) => {
                let values = values
                    .iter()
                    .map(CssValue::to_string)
                    .collect::<Vec<_>>()
                    .join(" ");
                f.write_str(&values)
            }
        }
    }
}

/// 簡易カラー文字列パーサ
fn parse_color(s: &str) -> Option<(u8, u8, u8, u8)> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        match hex.len() {
            3 => {
                // #RGB
                let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
                let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
                let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
                Some((r, g, b, 255))
            }
            4 => {
                // #RGBA
                let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
                let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
                let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
                let a = u8::from_str_radix(&hex[3..4].repeat(2), 16).ok()?;
                Some((r, g, b, a))
            }
            6 => {
                // #RRGGBB
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some((r, g, b, 255))
            }
            8 => {
                // #RRGGBBAA
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
                Some((r, g, b, a))
            }
            _ => None,
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_invalid_returns_none() {
        // Wrong length
        assert_eq!(CssValue::Color("".into()).to_rgba_tuple(), None);
        assert_eq!(CssValue::Color("a".into()).to_rgba_tuple(), None);
        assert_eq!(CssValue::Color("aa".into()).to_rgba_tuple(), None);
        assert_eq!(CssValue::Color("aaaaa".into()).to_rgba_tuple(), None);
        assert_eq!(CssValue::Color("aaaaaaa".into()).to_rgba_tuple(), None);
        assert_eq!(CssValue::Color("aaaaaaaaa".into()).to_rgba_tuple(), None);
        // Non-hex characters (parse_color uses from_str_radix with base 16)
        assert_eq!(CssValue::Color("zzz".into()).to_rgba_tuple(), None);
        assert_eq!(CssValue::Color("gggggg".into()).to_rgba_tuple(), None);
    }

    #[test]
    fn to_rgba_tuple_non_color_returns_none() {
        assert_eq!(CssValue::Keyword("red".into()).to_rgba_tuple(), None);
        assert_eq!(CssValue::Length(10.0, Unit::Px).to_rgba_tuple(), None);
        assert_eq!(CssValue::Number(1.0).to_rgba_tuple(), None);
        assert_eq!(CssValue::String("#fff".into()).to_rgba_tuple(), None);
        assert_eq!(
            CssValue::Function("rgb".into(), vec![]).to_rgba_tuple(),
            None
        );
    }

    #[test]
    fn display_renders_css_source_text() {
        assert_eq!(CssValue::Length(10.0, Unit::Px).to_string(), "10px");
        assert_eq!(CssValue::Length(1.5, Unit::Percent).to_string(), "1.5%");
        assert_eq!(CssValue::Number(0.75).to_string(), "0.75");
        assert_eq!(CssValue::Keyword("auto".into()).to_string(), "auto");
        assert_eq!(CssValue::Color("fff".into()).to_string(), "#fff");
        assert_eq!(CssValue::String("a b".into()).to_string(), "\"a b\"");
        assert_eq!(
            CssValue::Function(
                "rgb".into(),
                vec![
                    vec![CssValue::Number(255.0)],
                    vec![CssValue::Number(0.0)],
                    vec![CssValue::Number(0.0)],
                ]
            )
            .to_string(),
            "rgb(255, 0, 0)"
        );
        assert_eq!(
            CssValue::List(vec![
                CssValue::Length(100.0, Unit::Px),
                CssValue::Keyword("auto".into())
            ])
            .to_string(),
            "100px auto"
        );
    }
}
