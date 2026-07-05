//! Layout tree node types. Styles, text, and container definitions.

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
        /// Full text content.
        text: String,
        style: TextStyle,
        /// Unique ID linking to [`TextFlowLayouter`] for position data.
        text_id: usize,
    },
    LineBreak,
}

impl NodeKind {
    pub fn is_container_with_transparent_bg(&self) -> bool {
        matches!(self, NodeKind::Container { style, .. }
            if style.background == Background::Color(Color(0, 0, 0, 0)))
    }

    pub fn container_bg(&self) -> Option<&Background> {
        if let NodeKind::Container { style, .. } = self {
            Some(&style.background)
        } else {
            None
        }
    }
}

// =========================
//          Color
// =========================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub u8, pub u8, pub u8, pub u8);

impl Color {
    /// Convert sRGB (0..255) to linear RGB (0..1)
    pub fn to_linear_f32_array(&self) -> [f32; 4] {
        let r = Self::srgb_to_linear(self.0 as f32 / 255.0);
        let g = Self::srgb_to_linear(self.1 as f32 / 255.0);
        let b = Self::srgb_to_linear(self.2 as f32 / 255.0);
        let a = self.3 as f32 / 255.0; // alpha is linear
        [r, g, b, a]
    }

    /// Convert linear RGB (0..1) to sRGB Color (0..255)
    pub fn from_linear_f32_array(rgba: [f32; 4]) -> Self {
        Color(
            (Self::linear_to_srgb(rgba[0]).clamp(0.0, 1.0) * 255.0).round() as u8,
            (Self::linear_to_srgb(rgba[1]).clamp(0.0, 1.0) * 255.0).round() as u8,
            (Self::linear_to_srgb(rgba[2]).clamp(0.0, 1.0) * 255.0).round() as u8,
            (rgba[3].clamp(0.0, 1.0) * 255.0).round() as u8,
        )
    }

    /// Convert sRGB (0..1) to linear RGB (0..1)
    pub fn srgb_to_linear(c: f32) -> f32 {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    /// Convert linear RGB (0..1) to sRGB (0..1)
    pub fn linear_to_srgb(c: f32) -> f32 {
        if c <= 0.0031308 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        }
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
//        Background
// =========================

#[derive(Debug, Clone, PartialEq)]
pub enum Background {
    Color(Color),
    Gradient(Gradient),
}

impl Default for Background {
    fn default() -> Self {
        Self::Color(Color(0, 0, 0, 0))
    }
}

/// CSS gradient definition.
#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    pub kind: GradientKind,
    pub stops: Vec<ColorStop>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GradientKind {
    Linear {
        angle: f32,
    },
    Radial {
        shape: RadialShape,
        size: RadialSizeKind,
        position: (f32, f32),
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RadialShape {
    Circle,
    Ellipse,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RadialSizeKind {
    ClosestSide,
    FarthestSide,
    ClosestCorner,
    FarthestCorner,
}

impl Default for RadialSizeKind {
    fn default() -> Self {
        Self::FarthestCorner
    }
}

/// A single color stop in a gradient.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorStop {
    pub color: Color,
    /// Normalized position (0.0–1.0). None means the position is auto-distributed.
    pub position: Option<f32>,
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
    pub background: Background,
    pub border_color: BorderColor,
    pub border_style: BorderStyles,
}

impl Default for ContainerStyle {
    fn default() -> Self {
        Self {
            background: Background::default(),
            border_color: BorderColor::default(),
            border_style: BorderStyles::default(),
        }
    }
}

// =========================
//           Text
// =========================

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerticalAlign {
    #[default]
    Baseline,
    Sub,
    Super,
    Top,
    Bottom,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextTransform {
    #[default]
    None,
    Uppercase,
    Lowercase,
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

/// Line-height representation that preserves raw values for inheritance.
///
/// - `Normal` — keyword `normal`, resolved per element's font_size
/// - `Number(f)` — unitless factor (e.g. 1.5), re-resolved per child's font_size
/// - `Px(px)` — absolute pixel value (from `<length>`, `<percentage>`, `calc()`)
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub enum LineHeight {
    #[default]
    Normal,
    Number(f32),
    Px(f32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub font_size: f32,
    pub text_align: TextAlign,
    pub text_decoration: TextDecoration,
    pub text_transform: TextTransform,
    pub font_style: FontStyle,
    pub font_weight: FontWeight,
    pub color: Color,
    pub line_height: LineHeight,
    pub vertical_align: VerticalAlign,
    /// Override color for text-decoration lines.
    /// `None` means use `color` (currentColor).
    pub text_decoration_color: Option<Color>,
    /// Ordered list of font family names (CSS `font-family`).
    /// The first available font is used as primary; glyphs missing from it
    /// fall back to subsequent families. Generic families (e.g. "sans-serif")
    /// are resolved by the text engine.
    pub font_families: Vec<String>,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_size: 16.0,
            text_align: TextAlign::default(),
            text_decoration: TextDecoration::default(),
            text_transform: TextTransform::default(),
            font_style: FontStyle::default(),
            font_weight: FontWeight::default(),
            color: Color::default(),
            line_height: LineHeight::default(),
            vertical_align: VerticalAlign::default(),
            text_decoration_color: None,
            font_families: vec!["sans-serif".to_string()],
        }
    }
}
