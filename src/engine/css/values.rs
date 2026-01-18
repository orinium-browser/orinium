#[derive(Debug, Clone, PartialEq)]
pub enum Unit {
    Px,
    Em,
    Rem,
    Percent,
    Vw,
    Vh,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CssValue {
    Keyword(String),                 // e.g. auto, none
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
