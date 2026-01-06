pub mod css_resolver;

use css_resolver::ResolvedStyles;

use crate::engine::bridge::text;
use crate::engine::css::cssom::CssValue;
use crate::engine::tree::TreeNode;
use crate::html::{HtmlNodeType, util as html_util};
use std::cell::RefCell;
use std::rc::Rc;
use ui_layout::{Display, FlexDirection, ItemStyle, LayoutNode, Style};

#[derive(Debug, Clone)]
pub struct InfoNode {
    pub kind: NodeKind,
    pub color: Color,
    pub font_size: f32,
    pub text: Option<String>,
    pub children: Vec<InfoNode>,
}

#[derive(Debug, Clone)]
pub enum NodeKind {
    Container,
    Text,
    Scrollable,
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

impl TryFrom<(u8, u8, u8, f32)> for Color {
    type Error = ();

    fn try_from((r, g, b, a): (u8, u8, u8, f32)) -> Result<Self, Self::Error> {
        if !(0.0..=1.0).contains(&a) {
            return Err(());
        }
        Ok(Color(r, g, b, (a * 255.0).round() as u8))
    }
}

/// # 継承引数
///
/// - parent_color
/// - parent_font_size
///
/// 関数呼び出し時に適切な値に設定してください。
pub fn build_layout_and_info(
    dom: &Rc<RefCell<TreeNode<HtmlNodeType>>>,
    resolved_styles: &ResolvedStyles,
    measurer: &dyn text::TextMeasurer,
    parent_color: Color,
    parent_font_size: f32,
) -> (LayoutNode, InfoNode) {
    let html_node = dom.borrow().value.clone();

    /* -----------------------------
    Skip non-rendered elements
    ----------------------------- */
    if let HtmlNodeType::Element { tag_name, .. } = &html_node {
        if is_non_rendered_element(tag_name) {
            return (
                LayoutNode::new(Style {
                    display: Display::None,
                    ..Default::default()
                }),
                InfoNode {
                    kind: NodeKind::Container,
                    color: parent_color,
                    font_size: parent_font_size,
                    text: None,
                    children: Vec::new(),
                },
            );
        }
    }

    /* -----------------------------
    Initial values (inheritance)
    ----------------------------- */
    let mut kind = NodeKind::Container;
    let mut style = Style {
        display: Display::Block,
        item_style: ItemStyle {
            flex_grow: 0.0,
            flex_basis: None,
            ..Default::default()
        },
        column_gap: 0.0,
        row_gap: 0.0,
        ..Default::default()
    };

    let mut color = parent_color;
    let mut font_size = parent_font_size;
    let mut text: Option<String> = None;

    /* -----------------------------
    Apply resolved CSS
    ----------------------------- */
    if let HtmlNodeType::Element { tag_name, .. } = &html_node {
        if let Some(decls) = resolved_styles.get(tag_name) {
            for (name, value) in decls {
                apply_declaration(name, value, &mut style, &mut color, &mut font_size);
            }
        }
    }

    /* -----------------------------
    HTML defaults / semantics
    ----------------------------- */
    match &html_node {
        HtmlNodeType::Text(t) => {
            kind = NodeKind::Text;
            text = Some(t.clone());

            let req = text::TextMeasurementRequest {
                text: t.clone(),
                font: text::FontDescription {
                    family: None,
                    size_px: font_size,
                },
                constraints: text::LayoutConstraints {
                    max_width: None,
                    wrap: true,
                    max_lines: None,
                },
            };

            let (w, h) = measurer
                .measure(&req)
                .map(|m| (m.width, m.height))
                .unwrap_or((800.0, font_size * 1.2));

            style.size.width = Some(w);
            style.size.height = Some(h);
            style.item_style.flex_basis = Some(h);
        }

        HtmlNodeType::Element { tag_name, .. } => {
            if html_util::is_inline_element(tag_name) {
                style.display = Display::Flex {
                    flex_direction: FlexDirection::Row,
                };
            } else if html_util::is_block_level_element(tag_name) {
                style.display = Display::Block;
            }
        }

        _ => {}
    }

    /* -----------------------------
    Children (propagate computed style)
    ----------------------------- */
    let mut layout_children = Vec::new();
    let mut info_children = Vec::new();

    for child_dom in dom.borrow().children() {
        let (child_layout, child_info) =
            build_layout_and_info(child_dom, resolved_styles, measurer, color, font_size);
        layout_children.push(child_layout);
        info_children.push(child_info);
    }

    let layout = LayoutNode::with_children(style, layout_children);
    let info = InfoNode {
        kind,
        color,
        font_size,
        text,
        children: info_children,
    };

    (layout, info)
}

fn apply_declaration(
    name: &str,
    value: &CssValue,
    style: &mut Style,
    color: &mut Color,
    font_size: &mut f32,
) {
    match (name, value) {
        ("display", CssValue::Keyword(v)) if v == "block" => {
            style.display = Display::Block;
        }
        ("display", CssValue::Keyword(v)) if v == "flex" => {
            style.display = Display::Flex {
                flex_direction: FlexDirection::Row,
            };
        }
        ("color", CssValue::Color(c)) => {
            if let Ok(c) = Color::try_from(c.to_rgba_tuple(None)) {
                *color = c;
            }
        }
        ("font-size", CssValue::Length(len)) => {
            *font_size = len.to_px(16.0).unwrap_or(16.0);
        }
        _ => {}
    }
}

fn is_non_rendered_element(tag: &str) -> bool {
    matches!(tag, "head" | "meta" | "title" | "link" | "style" | "script")
}

#[derive(Debug, Clone)]
pub enum DrawCommand {
    DrawText {
        x: f32,
        y: f32,
        text: String,
        font_size: f32,
        color: Color,
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
/// `parent_x` / `parent_y` are offset from the parent node
pub fn generate_draw_commands(
    layout: &LayoutNode,
    info: &InfoNode,
    parent_x: f32,
    parent_y: f32,
) -> Vec<DrawCommand> {
    let mut commands = Vec::new();

    let rect = layout.rect;

    let abs_x = parent_x + rect.x;
    let abs_y = parent_y + rect.y;

    match info.kind {
        NodeKind::Text => {
            if let Some(text) = &info.text {
                commands.push(DrawCommand::DrawText {
                    x: abs_x,
                    y: abs_y,
                    text: text.clone(),
                    font_size: info.font_size,
                    color: info.color,
                    max_width: rect.width,
                });
            }
        }
        NodeKind::Scrollable => {
            commands.push(DrawCommand::PushTransform {
                dx: abs_x,
                dy: abs_y,
            });
            for (child_layout, child_info) in layout.children.iter().zip(&info.children) {
                commands.extend(generate_draw_commands(child_layout, child_info, 0.0, 0.0));
            }
            commands.push(DrawCommand::PopTransform);
            return commands;
        }
        NodeKind::Container => {
            // TODO: Add clipping
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
        }
    }

    for (child_layout, child_info) in layout.children.iter().zip(&info.children) {
        commands.extend(generate_draw_commands(child_layout, child_info, 0.0, 0.0));
    }

    if matches!(info.kind, NodeKind::Container) {
        commands.push(DrawCommand::PopClip);
        commands.push(DrawCommand::PopTransform);
    }

    commands
}
