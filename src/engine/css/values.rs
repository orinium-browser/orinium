//! CSSの値を表す構造体と列挙型

pub type CssIdent = smol_str::SmolStr;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Unit {
    Px,
    Em,
    Rem,
    Percent,
    Vw,
    Vh,
    Deg,
    Fr,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CssValue {
    Keyword(CssIdent),               // e.g. auto, none
    Length(f32, Unit),               // e.g. 10px
    Number(f32),                     // e.g. 1.5
    String(String),                  // e.g. "http"
    Color(String),                   // e.g. #fff, #1f1f11
    Function(String, Vec<CssValue>), // e.g. rgb(255,0,0)
    List(Vec<CssValue>),             // e.g. 100px auto
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
    fn as_str(self) -> &'static str {
        match self {
            Unit::Px => "px",
            Unit::Em => "em",
            Unit::Rem => "rem",
            Unit::Percent => "%",
            Unit::Vw => "vw",
            Unit::Vh => "vh",
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
                    .map(CssValue::to_string)
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
