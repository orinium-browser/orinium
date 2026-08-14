//! Layout tree node types. Styles, text, and container definitions.

use std::sync::Arc;

use crate::engine::renderer_model::Image;
use crate::engine::ui::custom_node::CustomNode;

use super::dom_snapshot::NodeId;

/// InfoNode represents a node in the layout tree.
/// It can be either a Container or Text node, each with its own properties and styles.
#[derive(Debug, Clone)]
pub struct InfoNode {
    pub kind: NodeKind,
    pub children: Vec<InfoNode>,
    /// Pre-order DOM snapshot node id this node was built from, when any.
    ///
    /// Lets click hit-testing map a layout node back to its live DOM node via
    /// the snapshot's `dom_refs`.
    pub dom_id: Option<NodeId>,
}

/// Semantic role used by interaction and post-layout processing.
#[derive(Debug, Clone, PartialEq)]
pub enum ContainerRole {
    Normal,
    Link { href: String },
    Table,
    TableRowGroup,
    TableRow,
    TableCell,
    TableCaption,
}

/// CSS `overflow` scrollability resolved per axis.
///
/// Each flag is `true` when the axis participates in scrolling
/// (`overflow: hidden` / `scroll` / `auto`). Populated from the CSS
/// `overflow` / `overflow-x` / `overflow-y` properties and consumed when
/// routing wheel input and applying scroll offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Overflow {
    /// Whether horizontal overflow scrolls.
    pub x: bool,
    /// Whether vertical overflow scrolls.
    pub y: bool,
}

/// Node kind of InfoNode
///
/// - Container: A container node that can hold other nodes and has scrolling capabilities.
/// - Text: A text node that contains text content and styling information.
/// - Custom: A custom node that generates its own draw commands via [`CustomNode::draw`].
#[derive(Debug, Clone)]
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
        flow_style: TextFlowStyle,
        /// Unique ID linking to [`TextFlowLayouter`] for position data.
        text_id: usize,
    },
    LineBreak,
    Custom {
        node: Arc<dyn CustomNode>,
        scroll_x: bool,
        scroll_y: bool,
        scroll_offset_x: f32,
        scroll_offset_y: f32,
        style: ContainerStyle,
        /// Resolved `ui_layout::Style` for CSS sizing at render time.
        layout_style: ui_layout::Style,
        text_style: TextStyle,
        text_flow_style: TextFlowStyle,
    },
}

impl NodeKind {
    pub fn z_index(&self) -> i32 {
        match self {
            NodeKind::Container { style, .. } | NodeKind::Custom { style, .. } => {
                style.z_index.unwrap_or(0)
            }
            _ => 0,
        }
    }

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

    pub fn custom_style(&self) -> Option<&ContainerStyle> {
        if let NodeKind::Custom { style, .. } = self {
            Some(style)
        } else {
            None
        }
    }

    pub fn scroll_offsets(&self) -> (f32, f32) {
        match self {
            NodeKind::Container {
                scroll_offset_x,
                scroll_offset_y,
                ..
            }
            | NodeKind::Custom {
                scroll_offset_x,
                scroll_offset_y,
                ..
            } => (*scroll_offset_x, *scroll_offset_y),
            _ => (0.0, 0.0),
        }
    }
}

// =========================
//          Color
// =========================

/// Color scheme used to resolve `light-dark()` values and system colors.
///
/// Mirrors the system preference (`prefers-color-scheme`) and the computed
/// `color-scheme` property of each element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorScheme {
    #[default]
    Light,
    Dark,
}

impl From<dark_light::Mode> for ColorScheme {
    fn from(value: dark_light::Mode) -> Self {
        match value {
            dark_light::Mode::Dark => ColorScheme::Dark,
            dark_light::Mode::Light => ColorScheme::Light,
            dark_light::Mode::Unspecified => ColorScheme::default(),
        }
    }
}

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
    Image {
        source: String,
        image: Option<Image>,
        color: Color,
    },
}

impl Default for Background {
    fn default() -> Self {
        Self::Color(Color(0, 0, 0, 0))
    }
}

/// How a CSS background image is repeated inside its painting area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackgroundRepeat {
    #[default]
    Repeat,
    RepeatX,
    RepeatY,
    NoRepeat,
}

/// A component of `background-size`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BackgroundDimension {
    #[default]
    Auto,
    Length(f32),
    Percent(f32),
}

/// The resolved syntax of `background-size`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BackgroundSize {
    #[default]
    Auto,
    Contain,
    Cover,
    Explicit {
        width: BackgroundDimension,
        height: BackgroundDimension,
    },
}

/// An offset following an edge keyword in `background-position`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BackgroundOffset {
    #[default]
    Zero,
    Length(f32),
    Percent(f32),
}

/// One horizontal or vertical component of `background-position`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BackgroundPositionAxis {
    Start(BackgroundOffset),
    Center(BackgroundOffset),
    End(BackgroundOffset),
}

impl Default for BackgroundPositionAxis {
    fn default() -> Self {
        Self::Start(BackgroundOffset::Zero)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BackgroundPosition {
    pub x: BackgroundPositionAxis,
    pub y: BackgroundPositionAxis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CssFloat {
    #[default]
    None,
    Left,
    Right,
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
    Conic {
        /// Start angle in degrees (default 0).
        angle: f32,
        /// Center position as normalized (0..1) coordinates (default 0.5, 0.5).
        position: (f32, f32),
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RadialShape {
    Circle,
    Ellipse,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum RadialSizeKind {
    ClosestSide,
    FarthestSide,
    ClosestCorner,
    #[default]
    FarthestCorner,
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

/// Border radius of a single corner.
///
/// `x` is resolved against the border-box width and `y` against the border-box
/// height (so percentages resolve per-axis per CSS).
#[derive(Debug, Clone, PartialEq)]
pub struct CornerRadius {
    pub x: ui_layout::Length,
    pub y: ui_layout::Length,
}

impl Default for CornerRadius {
    fn default() -> Self {
        Self {
            x: ui_layout::Length::Px(0.0),
            y: ui_layout::Length::Px(0.0),
        }
    }
}

/// Rounded-corner radii of a box. Values are ordered top-left, top-right,
/// bottom-right, bottom-left (matching CSS clockwise order).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BorderRadius {
    pub top_left: CornerRadius,
    pub top_right: CornerRadius,
    pub bottom_right: CornerRadius,
    pub bottom_left: CornerRadius,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ContainerStyle {
    pub background: Background,
    pub background_repeat: BackgroundRepeat,
    pub background_size: BackgroundSize,
    pub background_position: BackgroundPosition,
    /// Integer stacking level. `None` represents CSS `auto`.
    pub z_index: Option<i32>,
    pub css_float: CssFloat,
    pub border_color: BorderColor,
    pub border_style: BorderStyles,
    pub border_radius: BorderRadius,
}

// =========================
//           Text
// =========================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextDecoration {
    #[default]
    None,
    Underline,
    LineThrough,
    Overline,
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

#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub text_decoration: TextDecoration,
    pub text_transform: TextTransform,
    pub font_style: FontStyle,
    pub font_weight: FontWeight,
    pub color: Color,
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
            text_decoration: TextDecoration::default(),
            text_transform: TextTransform::default(),
            font_style: FontStyle::default(),
            font_weight: FontWeight::default(),
            color: Color::default(),
            text_decoration_color: None,
            font_families: vec!["sans-serif".to_string()],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
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
pub enum WhiteSpace {
    #[default]
    Normal,
    Nowrap,
    Pre,
    PreWrap,
    PreLine,
    BreakSpaces,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct TextFlowStyle {
    pub font_size: f32,
    pub text_align: TextAlign,
    pub line_height: LineHeight,
    pub vertical_align: VerticalAlign,
    pub white_space: WhiteSpace,
}

impl Default for TextFlowStyle {
    fn default() -> Self {
        Self {
            font_size: 16.0,
            text_align: Default::default(),
            line_height: Default::default(),
            vertical_align: Default::default(),
            white_space: Default::default(),
        }
    }
}
