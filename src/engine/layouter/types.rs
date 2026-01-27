/// InfoNode represents a node in the layout tree.
/// It can be either a Container or Text node, each with its own properties and styles.
#[derive(Debug, Clone)]
pub struct InfoNode {
    pub kind: NodeKind,
    pub children: Vec<InfoNode>,
}

/// Role of Container
///
/// - Normal: A standard container with no special role.
/// - Link: A container that acts as a hyperlink, containing a URL.
#[derive(Debug, Clone, PartialEq)]
pub enum ContainerRole {
    Normal,
    Link { href: String },
}

/// Node kind of InfoNode
///
/// - Container: A container node that can hold other nodes and has scrolling capabilities.
/// - Text: A text node that contains text content and styling information.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Container {
        scroll_x: bool,
        scroll_y: bool,
        scroll_offset_x: f32,
        scroll_offset_y: f32,
        style: ContainerStyle,
        role: ContainerRole,
    },
    Text {
        text: String,
        style: TextStyle,
        measured: Option<MeasureCache>,
    },
}

// =========================
//          Color
// =========================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub u8, pub u8, pub u8, pub u8);

impl Color {
    /// u8 RGBA -> [f32; 4] RGBA (0.0~1.0)
    pub fn to_f32_array(&self) -> [f32; 4] {
        [
            self.0 as f32 / 255.0,
            self.1 as f32 / 255.0,
            self.2 as f32 / 255.0,
            self.3 as f32 / 255.0,
        ]
    }
}

impl Default for Color {
    fn default() -> Self {
        Self(0, 0, 0, 255)
    }
}

impl TryFrom<(u8, u8, u8, f32)> for Color {
    type Error = ();

    fn try_from((r, g, b, a): (u8, u8, u8, f32)) -> Result<Self, Self::Error> {
        if !(0.0..=1.0).contains(&a) {
            return Err(());
        }
        Ok(Color(r, g, b, (a * 255.0).round() as u8))
    }
}

// =========================
//        Cantainer
// =========================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderStyle {
    #[default]
    None,
    Solid,
    Dashed,
    Dotted,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BorderColor {
    pub top: Color,
    pub right: Color,
    pub bottom: Color,
    pub left: Color,
}

impl Default for BorderColor {
    fn default() -> Self {
        let c = Color(0, 0, 0, 255);
        Self {
            top: c,
            right: c,
            bottom: c,
            left: c,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BorderStyles {
    pub top: BorderStyle,
    pub right: BorderStyle,
    pub bottom: BorderStyle,
    pub left: BorderStyle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContainerStyle {
    pub background_color: Color,
    pub border_color: BorderColor,
    pub border_style: BorderStyles,
}

impl Default for ContainerStyle {
    fn default() -> Self {
        Self {
            background_color: Color(0, 0, 0, 0),
            border_color: BorderColor::default(),
            border_style: BorderStyles::default(),
        }
    }
}

// =========================
//           Text
// =========================

/// TODO
/// - Add cache logic
#[allow(dead_code)]
#[derive(Hash)]
struct TextMeasureHashKey<'a> {
    text: &'a str,
    font_size: u32,
    font_weight: u16,
    font_style: FontStyle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeasureCache {
    pub hash: u64,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextDecoration {
    #[default]
    None,
    Underline,
    LineThrough,
    Overline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontWeight(pub u16);

impl FontWeight {
    pub const THIN: Self = Self(100);
    pub const NORMAL: Self = Self(400);
    pub const BOLD: Self = Self(700);
    pub const BLACK: Self = Self(900);
}

impl Default for FontWeight {
    fn default() -> Self {
        Self::NORMAL
    }
}

#[derive(Copy, Debug, Clone, Default, PartialEq)]
pub struct TextStyle {
    pub font_size: f32,
    pub text_align: TextAlign,
    pub text_decoration: TextDecoration,
    pub font_style: FontStyle,
    pub font_weight: FontWeight,
    pub color: Color,
}
