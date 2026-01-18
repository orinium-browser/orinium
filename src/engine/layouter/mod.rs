pub mod css_resolver;

use anyhow::Result;

use crate::engine::css::matcher::{ElementChain, ElementInfo};
use css_resolver::ResolvedStyles;

use crate::engine::bridge::text;
use crate::engine::css::values::{CssValue, Unit};
use crate::engine::tree::TreeNode;
use crate::html::HtmlNodeType;
use std::cell::RefCell;
use std::rc::Rc;
use ui_layout::{Display, FlexDirection, LayoutNode, Length, Style};

#[derive(Debug, Clone)]
pub struct InfoNode {
    pub kind: NodeKind,
    pub children: Vec<InfoNode>,
}

#[derive(Debug, Clone)]
pub enum NodeKind {
    Container {
        scroll_x: bool,
        scroll_y: bool,
        scroll_offset_x: f32,
        scroll_offset_y: f32,
        style: ContainerStyle,
    },
    Text {
        text: String,
        style: TextStyle,
    },
}

#[derive(Debug, Clone, Copy)]
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

#[derive(Debug, Clone)]
pub struct ContainerStyle {
    pub background_color: Color,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(Copy, Debug, Clone, Default)]
pub struct TextStyle {
    pub font_size: f32,
    pub text_align: TextAlign,
    pub text_decoration: TextDecoration,
    pub font_style: FontStyle,
    pub font_weight: FontWeight,
    pub color: Color,
}

/// Builds a layout tree (`LayoutNode`) and a render info tree (`InfoNode`) from the DOM.
///
/// # Overview
/// - Recursively traverses the HTML DOM
/// - Applies resolved CSS declarations
/// - Computes layout-related styles
/// - Collects render-time information (color, font size, text)
///
/// # Style resolution order (low → high priority)
///
/// 1. **Inherited values from parent**
///    - `text_style`
///
/// 2. **Resolved CSS declarations**
///    - Overrides inherited values when specified
///
/// 3. **HTML defaults / semantics**
///    - `display` (block, inline, etc.)
///    - Text measurement for text nodes
///
/// # Inherited properties
///
/// Only the following properties are inherited explicitly:
///
/// - `text_style`
///
/// All other style fields are initialized per node and are **not inherited**.
///
/// # Parameters
///
/// - `parent_text_style`
///
/// These values must be passed from the computed result of the parent when
/// calling this function recursively.
///
/// # Returns
///
/// A tuple of:
/// - `LayoutNode`: used by the layout engine
/// - `InfoNode`: used for rendering (text, color, font size)
pub fn build_layout_and_info(
    dom: &Rc<RefCell<TreeNode<HtmlNodeType>>>,
    resolved_styles: &ResolvedStyles,
    measurer: &dyn text::TextMeasurer<TextStyle>,
    parent_text_style: TextStyle,
    mut chain: ElementChain,
) -> (LayoutNode, InfoNode) {
    let html_node = dom.borrow().value.clone();

    /* -----------------------------
       Initial values (inheritance)
    ----------------------------- */
    let mut style = Style::default();

    let mut text_style = parent_text_style;
    let mut container_style = ContainerStyle {
        background_color: Color(0, 0, 0, 0),
    };

    /* -----------------------------
       Apply resolved CSS
    ----------------------------- */
    if let HtmlNodeType::Element {
        tag_name,
        attributes,
        ..
    } = &html_node
    {
        let id = attributes
            .iter()
            .find(|a| a.name == "id")
            .map(|a| a.value.clone());

        let class_list: Vec<String> = attributes
            .iter()
            .find(|attr| attr.name == "class")
            .map(|attr| {
                attr.value
                    .split_whitespace()
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();

        chain.insert(
            0,
            ElementInfo {
                tag_name: tag_name.clone(),
                id,
                classes: class_list,
            },
        );

        for (selector, declarations) in resolved_styles {
            if selector.matches(&chain) {
                for (name, value) in declarations {
                    apply_declaration(
                        name,
                        value,
                        &mut style,
                        &mut container_style,
                        &mut text_style,
                    );
                }
            }
        }
    }

    let kind = if let HtmlNodeType::Text(t) = &html_node {
        let t = normalize_whitespace(t);
        let kind = NodeKind::Text {
            text: t.clone(),
            style: text_style,
        };

        let req = text::TextMeasureRequest {
            text: t.clone(),
            style: text_style,
            max_width: None,
            wrap: false,
        };

        let (w, h) = measurer
            .measure(&req)
            .map(|m| (m.width, m.height))
            .unwrap_or((800.0, text_style.font_size * 1.2));

        style.size.width = Length::Px(w);
        style.size.height = Length::Px(h);
        kind
    } else {
        NodeKind::Container {
            scroll_x: false,
            scroll_y: false,
            scroll_offset_x: 0.0,
            scroll_offset_y: 0.0,
            style: container_style,
        }
    };

    /* -----------------------------
       Children
    ----------------------------- */

    // NOTE:
    // Inline / LineBox は未実装。
    // block 要素の直下に text ノードが存在する場合、
    // 親を flex(row) として扱うことで inline flow を暫定的に再現する。
    // 将来的には LineBox 実装に置き換える。
    let mut layout_children = Vec::new();
    let mut info_children = Vec::new();

    if !matches!(style.display, Display::None) {
        let mut has_text_child = false;

        for child_dom in dom.borrow().children() {
            if matches!(child_dom.borrow().value, HtmlNodeType::Text(_)) {
                has_text_child = true;
                break;
            }
        }

        // 子に TextNode がある Block 要素は Flex(row) に変換
        if has_text_child && matches!(style.display, Display::Block) {
            style.display = Display::Flex {
                flex_direction: FlexDirection::Row,
            };
        }

        for child_dom in dom.borrow().children() {
            let (child_layout, child_info) = build_layout_and_info(
                child_dom,
                resolved_styles,
                measurer,
                text_style,
                chain.clone(),
            );
            layout_children.push(child_layout);
            info_children.push(child_info);
        }
    }

    let layout = LayoutNode::with_children(style, layout_children);

    let info = InfoNode {
        kind,
        children: info_children,
    };

    (layout, info)
}

fn normalize_whitespace(text: &str) -> String {
    let mut result = String::new();
    let mut prev_was_space = false;

    for c in text.chars() {
        if c.is_whitespace() {
            if !prev_was_space {
                result.push(' ');
                prev_was_space = true;
            }
        } else {
            result.push(c);
            prev_was_space = false;
        }
    }

    result
}

fn apply_declaration(
    name: &str,
    value: &CssValue,
    style: &mut Style,
    container_style: &mut ContainerStyle,
    text_style: &mut TextStyle,
) -> Option<()> {
    let resolve_css_len = |css_len: &CssValue| -> Option<Length> {
        match &css_len {
            CssValue::Length(v, Unit::Em) => Some(Length::Px(text_style.font_size * v)),
            CssValue::Length(v, Unit::Rem) => Some(Length::Px(16.0 * v)), // html sont-size 仮値
            CssValue::Length(v, u) => match u {
                Unit::Percent => Some(Length::Percent(*v)),
                Unit::Px => Some(Length::Px(*v)),
                Unit::Vw => Some(Length::Vw(*v)),
                Unit::Vh => Some(Length::Vh(*v)),
                Unit::Em | Unit::Rem => unreachable!(),
            },
            CssValue::Number(0.0) => Some(Length::Px(0.0)),
            CssValue::Keyword(s) => match s.as_str() {
                "auto" => Some(Length::Auto),
                _ => None,
            },
            _ => {
                log::error!(target: "Layouter", "Unknown CSS Length type: {:?}", css_len);
                None
            }
        }
    };
    fn expand_box<F>(
        value: &CssValue,
        resolve_css_len: &impl Fn(&CssValue) -> Option<Length>,
        mut set: F,
    ) -> Option<()>
    where
        F: FnMut(Length, Length, Length, Length),
    {
        let resolve = |v: &CssValue| -> Option<Length> { resolve_css_len(v) };

        match value {
            CssValue::List(values) => {
                let vals: Vec<Length> = values.iter().map(resolve).collect::<Option<_>>()?;

                match vals.as_slice() {
                    [a] => set(a.clone(), a.clone(), a.clone(), a.clone()),
                    [v, h] => set(v.clone(), h.clone(), v.clone(), h.clone()),
                    [t, h, b] => set(t.clone(), h.clone(), b.clone(), h.clone()),
                    [t, r, b, l] => set(t.clone(), r.clone(), b.clone(), l.clone()),
                    _ => return None,
                }
            }

            _ => {
                let v = resolve_css_len(value)?;
                set(v.clone(), v.clone(), v.clone(), v);
            }
        }

        Some(())
    }

    match (name, value) {
        /* ======================
         * Display
         * ====================== */
        ("display", CssValue::Keyword(v)) if v == "block" => {
            style.display = Display::Block;
        }
        ("display", CssValue::Keyword(v)) if v == "flex" => {
            style.display = Display::Flex {
                flex_direction: FlexDirection::Row,
            };
        }
        ("display", CssValue::Keyword(v)) if v == "inline" => {
            // tmp：inline = row flex
            style.display = Display::Flex {
                flex_direction: FlexDirection::Row,
            };
        }
        ("display", CssValue::Keyword(v)) if v == "none" => {
            style.display = Display::None;
        }

        /* ======================
         * Color / Text
         * ====================== */
        ("background-color", CssValue::Color(_c)) => {
            let c = value.to_rgba_tuple()?;
            container_style.background_color = Color(c.0, c.1, c.2, c.3);
        }
        ("background-color", CssValue::Keyword(v)) => {
            if let Some(c) = keyword_color_to_color(v) {
                container_style.background_color = c;
            }
        }

        ("color", CssValue::Color(_c)) => {
            let c = value.to_rgba_tuple()?;
            text_style.color = Color(c.0, c.1, c.2, c.3);
        }
        ("color", CssValue::Keyword(v)) => {
            if let Some(c) = keyword_color_to_color(v) {
                text_style.color = c;
            }
        }

        ("font-size", CssValue::Length(_, _)) => {
            // TODO: Add other size
            let len = resolve_css_len(value)?;
            let px = match &len {
                Length::Px(v) => *v,
                _ => {
                    log::error!(target: "Layouter", "Unknown size type for font-size: {:?}", len);
                    return None;
                }
            };
            text_style.font_size = px;
        }

        ("font-weight", CssValue::Keyword(v)) if v == "normal" => {
            text_style.font_weight = FontWeight::NORMAL;
        }
        ("font-weight", CssValue::Keyword(v)) if v == "bold" => {
            text_style.font_weight = FontWeight::BOLD;
        }

        ("font-style", CssValue::Keyword(v)) if v == "normal" => {
            text_style.font_style = FontStyle::Normal;
        }
        ("font-style", CssValue::Keyword(v)) if v == "italic" => {
            text_style.font_style = FontStyle::Italic;
        }
        ("font-style", CssValue::Keyword(v)) if v == "oblique" => {
            text_style.font_style = FontStyle::Oblique;
        }

        ("text-decoration", CssValue::Keyword(v)) if v == "none" => {
            text_style.text_decoration = TextDecoration::None;
        }
        ("text-decoration", CssValue::Keyword(v)) if v == "underline" => {
            text_style.text_decoration = TextDecoration::Underline;
        }

        ("text-align", CssValue::Keyword(v)) if v == "left" => {
            text_style.text_align = TextAlign::Left;
        }
        ("text-align", CssValue::Keyword(v)) if v == "center" => {
            text_style.text_align = TextAlign::Center;
        }
        ("text-align", CssValue::Keyword(v)) if v == "right" => {
            text_style.text_align = TextAlign::Right;
        }

        /* ======================
         * Box Model
         * ====================== */
        ("margin", v) => {
            expand_box(v, &resolve_css_len, |t, r, b, l| {
                style.spacing.margin_top = t;
                style.spacing.margin_right = r;
                style.spacing.margin_bottom = b;
                style.spacing.margin_left = l;
            })?;
        }
        ("padding", v) => {
            expand_box(v, &resolve_css_len, |t, r, b, l| {
                style.spacing.padding_top = t;
                style.spacing.padding_right = r;
                style.spacing.padding_bottom = b;
                style.spacing.padding_left = l;
            })?;
        }

        ("margin-top", _) => {
            style.spacing.margin_top = resolve_css_len(value)?;
        }
        ("margin-right", _) => {
            style.spacing.margin_right = resolve_css_len(value)?;
        }
        ("margin-bottom", _) => {
            style.spacing.margin_bottom = resolve_css_len(value)?;
        }
        ("margin-left", _) => {
            style.spacing.margin_left = resolve_css_len(value)?;
        }

        /* ======================
         * Size
         * ====================== */
        ("width", _) => {
            style.size.width = resolve_css_len(value)?;
        }
        ("height", _) => {
            style.size.height = resolve_css_len(value)?;
        }
        ("min-width", _) => {
            style.size.min_width = resolve_css_len(value)?;
        }
        ("min-height", _) => {
            style.size.min_height = resolve_css_len(value)?;
        }
        ("max-width", _) => {
            style.size.max_width = resolve_css_len(value)?;
        }
        ("max-height", _) => {
            style.size.max_height = resolve_css_len(value)?;
        }

        /* ======================
         * Flex
         * ====================== */
        ("flex-direction", CssValue::Keyword(v)) if v == "row" => {
            if let Display::Flex {
                ref mut flex_direction,
            } = style.display
            {
                *flex_direction = FlexDirection::Row;
            }
        }
        ("flex-direction", CssValue::Keyword(v)) if v == "column" => {
            if let Display::Flex {
                ref mut flex_direction,
            } = style.display
            {
                *flex_direction = FlexDirection::Column;
            }
        }

        _ => {}
    }
    Some(())
}

fn keyword_color_to_color(keyword: &str) -> Option<Color> {
    match keyword {
        "black" => Some(Color(0, 0, 0, 255)),
        "white" => Some(Color(255, 255, 255, 255)),
        "red" => Some(Color(255, 0, 0, 255)),
        "green" => Some(Color(0, 128, 0, 255)),
        "blue" => Some(Color(0, 0, 255, 255)),
        "yellow" => Some(Color(255, 255, 0, 255)),
        "gray" | "grey" => Some(Color(128, 128, 128, 255)),
        _ => {
            log::error!(target: "Layouter", "Unknown color keyword: {}", keyword);
            None
        }
    }
}

#[derive(Debug, Clone)]
pub enum DrawCommand {
    DrawText {
        x: f32,
        y: f32,
        text: String,
        style: TextStyle,
        max_width: f32,
    },
    DrawRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: Color,
    },
    DrawPolygon {
        points: Vec<(f32, f32)>,
        color: Color,
    },
    DrawEllipse {
        center: (f32, f32),
        radius_x: f32,
        radius_y: f32,
        color: Color,
    },
    PushClip {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    PopClip,
    PushTransform {
        dx: f32,
        dy: f32,
    },
    PopTransform,
}

/// LayoutNode + InfoNode → DrawCommand
/// TODO: Support TextDecoration.
pub fn generate_draw_commands(layout: &LayoutNode, info: &InfoNode) -> Vec<DrawCommand> {
    let mut commands = Vec::new();

    let rect = layout.rect;

    let abs_x = rect.x;
    let abs_y = rect.y;

    match &info.kind {
        NodeKind::Text { text, style } => {
            /*
            commands.push(DrawCommand::DrawRect {
                x: abs_x,
                y: abs_y,
                width: rect.width,
                height: rect.height,
                color: Color(255, 0, 0, 255),
            });
            */
            commands.push(DrawCommand::DrawText {
                x: abs_x,
                y: abs_y,
                text: text.clone(),
                style: *style,
                max_width: rect.width,
            });
        }
        NodeKind::Container {
            scroll_offset_x,
            scroll_offset_y,
            style,
            ..
        } => {
            commands.push(DrawCommand::PushTransform {
                dx: abs_x,
                dy: abs_y,
            });
            commands.push(DrawCommand::PushClip {
                x: 0.0,
                y: 0.0,
                width: rect.width,
                height: rect.height,
            });
            commands.push(DrawCommand::DrawRect {
                x: 0.0,
                y: 0.0,
                width: rect.width,
                height: rect.height,
                color: style.background_color,
            });
            commands.push(DrawCommand::PushTransform {
                dx: *scroll_offset_x,
                dy: *scroll_offset_y,
            });
        }
    }

    for (child_layout, child_info) in layout.children.iter().zip(&info.children) {
        commands.extend(generate_draw_commands(child_layout, child_info));
    }

    if matches!(info.kind, NodeKind::Container { .. }) {
        commands.push(DrawCommand::PopTransform);
        commands.push(DrawCommand::PopClip);
        commands.push(DrawCommand::PopTransform);
    }

    commands
}
