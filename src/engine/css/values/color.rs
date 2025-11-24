#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Color {
    Rgba(u8, u8, u8, f32), // RGBA (0–255, 0–1)
    CurrentColor,          // CSS の currentColor
    Transparent,           // 透明
}

impl Default for Color {
    fn default() -> Self {
        Color::Rgba(0, 0, 0, 1.0) // デフォルトは不透明な黒
    }
}

impl Color {
    pub const BLACK: Color = Color::Rgba(0, 0, 0, 1.0);
    pub const WHITE: Color = Color::Rgba(255, 255, 255, 1.0);
    pub const TRANSPARENT: Color = Color::Rgba(0, 0, 0, 0.0);

    pub fn to_rgba_tuple(&self, current: Option<&Color>) -> (u8, u8, u8, f32) {
        match self {
            Color::Rgba(r, g, b, a) => (*r, *g, *b, *a),
            Color::Transparent => (0, 0, 0, 0.0),
            Color::CurrentColor => {
                if let Some(c) = current {
                    c.to_rgba_tuple(None)
                } else {
                    (0, 0, 0, 1.0)
                }
            }
        }
    }

    pub fn from_named(name: &str) -> Option<Color> {
        match name.to_ascii_lowercase().as_str() {
            "black" => Some(Color::BLACK),
            "white" => Some(Color::WHITE),
            "red" => Some(Color::Rgba(255, 0, 0, 1.0)),
            "green" => Some(Color::Rgba(0, 128, 0, 1.0)),
            "blue" => Some(Color::Rgba(0, 0, 255, 1.0)),
            "transparent" => Some(Color::TRANSPARENT),
            _ => None,
        }
    }

    pub fn from_hex(hex: &str) -> Option<Color> {
        let hex = hex.trim_start_matches('#');
        match hex.len() {
            3 => {
                let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
                let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
                let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
                Some(Color::Rgba(r, g, b, 1.0))
            }
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some(Color::Rgba(r, g, b, 1.0))
            }
            _ => None,
        }
    }

    pub fn from_hsl(h: f32, s: f32, l: f32, a: f32) -> Color {
        // （簡易実装：CSS Color Module Level 3準拠）
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
        let m = l - c / 2.0;
        let (r, g, b) = match h as i32 {
            0..=59 => (c, x, 0.0),
            60..=119 => (x, c, 0.0),
            120..=179 => (0.0, c, x),
            180..=239 => (0.0, x, c),
            240..=299 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        Color::Rgba(
            ((r + m) * 255.0) as u8,
            ((g + m) * 255.0) as u8,
            ((b + m) * 255.0) as u8,
            a,
        )
    }

    pub fn as_f32_array(&self) -> [f32; 4] {
        let (r, g, b, a) = self.to_rgba_tuple(None);
        [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a]
    }
}
