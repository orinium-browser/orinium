#[derive(Debug, Clone)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const BLACK: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    pub const WHITE: Color = Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
    

    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub fn from_rgba_tuple(rgba: (u8, u8, u8, f32)) -> Self {
        Self {
            r: rgba.0 as f32 / 255.0,
            g: rgba.1 as f32 / 255.0,
            b: rgba.2 as f32 / 255.0,
            a: rgba.3,
        }
    }
}
