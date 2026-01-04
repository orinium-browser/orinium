use crate::engine::bridge::text;
use crate::engine::css::cssom::CssNodeType;
use crate::engine::tree::{Tree, TreeNode};
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

/// DOM + CSSOM → LayoutNode + InfoNode
pub fn build_layout_and_info(
    dom: &Rc<RefCell<TreeNode<HtmlNodeType>>>,
    cssoms: &[Tree<CssNodeType>],
    measurer: &dyn text::TextMeasurer,
    font_size: f32,
) -> (LayoutNode, InfoNode) {
    let html_node = dom.borrow().value.clone();

    let mut kind = NodeKind::Container;
    let mut style = Style {
        display: Display::Flex {
            flex_direction: FlexDirection::Column,
        },
        item_style: ItemStyle {
            flex_grow: 0.0,
            flex_basis: None,
            ..Default::default()
        },
        column_gap: 0.0,
        row_gap: 0.0,
        ..Default::default()
    };
    let color = Color(0, 0, 0, 255);
    let mut text: Option<String> = None;

    let mut font_size = font_size;

    match &html_node {
        HtmlNodeType::Text(t) => {
            kind = NodeKind::Text;
            text = Some(t.clone());

            // テキストのサイズを測定
            let req = text::TextMeasurementRequest {
                text: t.clone(),
                font: text::FontDescription {
                    family: None,
                    size_px: font_size,
                },
                constraints: text::LayoutConstraints {
                    max_width: Some(800.0), // とりあえず仮の最大幅
                    wrap: true,
                    max_lines: None,
                },
            };
            let (w, h) = if let Ok(mears) = measurer.measure(&req) {
                (mears.width, mears.height)
            } else {
                (800.0, font_size * 1.2)
            };

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
                style.display = Display::Flex {
                    flex_direction: FlexDirection::Column,
                };
            }
            match tag_name.as_str() {
                "h1" => font_size = 32.0,
                "h2" => font_size = 24.0,
                "h3" => font_size = 18.0,
                _ => {}
            }
        }
        _ => {}
    }

    // TODO: CSS 適用
    #[allow(unused)]
    {
        for css in cssoms {}
    }

    let mut layout_children = Vec::new();
    let mut info_children = Vec::new();

    for child_dom in dom.borrow().children() {
        let (child_layout, child_info) =
            build_layout_and_info(child_dom, cssoms, measurer, font_size);
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
