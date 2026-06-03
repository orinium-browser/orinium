//! Draw command definition for rendering, which represents drawing instructions.

use crate::engine::layouter::types::{Color, InfoNode, NodeKind, TextDecoration, TextStyle};
use smol_str::SmolStr;
use ui_layout::{FragmentNode, LayoutChild, LayoutNode};

#[derive(Debug, Clone)]
pub enum DrawCommand {
    DrawText {
        x: f32,
        y: f32,
        text: SmolStr,
        style: TextStyle,
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
pub fn generate_draw_commands(
    cmd_buf: &mut Vec<DrawCommand>,
    layout: &LayoutNode,
    info: &InfoNode,
) {
    match &info.kind {
        NodeKind::Text { texts, style, .. } => {
            let fragments: Vec<&FragmentNode> = layout
                .children
                .iter()
                .filter_map(|c| c.fragment())
                .collect();

            debug_assert!(
                texts.len() <= fragments.len(),
                "`generate_draw_commands` may be called before layout is complete."
            );
            for (text, fragment_node) in texts.iter().zip(fragments) {
                let placement = fragment_node.placement;
                let fragment = fragment_node.node;
                let (abs_x, abs_y) = placement.offset;

                // テキスト
                cmd_buf.push(DrawCommand::DrawText {
                    x: abs_x,
                    y: abs_y,
                    text: text.into(),
                    style: *style,
                });

                // テキストデコレーション
                let font_size = style.font_size;
                let line_thickness = (font_size * 0.08).max(1.0);

                let (line_y, draw) = match style.text_decoration {
                    TextDecoration::None => (0.0, false),
                    TextDecoration::Underline => (abs_y + font_size, true),
                    TextDecoration::LineThrough => (abs_y + font_size * 0.5, true),
                    TextDecoration::Overline => (abs_y, true),
                };

                if draw {
                    cmd_buf.push(DrawCommand::DrawRect {
                        x: abs_x,
                        y: line_y,
                        width: fragment.width(),
                        height: line_thickness,
                        color: style.color,
                    });
                }
            }
        }

        NodeKind::Container {
            scroll_offset_x,
            scroll_offset_y,
            style,
            ..
        } => {
            for box_model in &layout.layout_box {
                let border_box = box_model.border_box;
                let padding_box = box_model.padding_box;
                let content_box = box_model.content_box;

                // ===== border (solid only for now) =====
                cmd_buf.push(DrawCommand::PushTransform {
                    dx: border_box.x,
                    dy: border_box.y,
                });

                let bc = &style.border_color;

                // top
                let border_width = (padding_box.y - border_box.y).max(0.0);
                cmd_buf.push(DrawCommand::DrawRect {
                    x: 0.0,
                    y: 0.0,
                    width: border_box.width,
                    height: border_width,
                    color: bc.top,
                });

                // bottom
                let border_width = (border_box.y + border_box.height
                    - (padding_box.y + padding_box.height))
                    .max(0.0);
                cmd_buf.push(DrawCommand::DrawRect {
                    x: 0.0,
                    y: border_box.height - border_width,
                    width: border_box.width,
                    height: border_width,
                    color: bc.bottom,
                });

                // left
                let border_width = (padding_box.x - border_box.x).max(0.0);
                cmd_buf.push(DrawCommand::DrawRect {
                    x: 0.0,
                    y: 0.0,
                    width: border_width,
                    height: border_box.height,
                    color: bc.left,
                });

                // right
                let border_width = (border_box.x + border_box.width
                    - (padding_box.x + padding_box.width))
                    .max(0.0);
                cmd_buf.push(DrawCommand::DrawRect {
                    x: border_box.width - border_width,
                    y: 0.0,
                    width: border_width,
                    height: border_box.height,
                    color: bc.right,
                });

                // ===== clip + background + content =====
                let is_inline = matches!(layout.layout_box, ui_layout::LayoutBox::InlineBox(_));
                if !is_inline {
                    cmd_buf.push(DrawCommand::PushClip {
                        x: padding_box.x - border_box.x,
                        y: padding_box.y - border_box.y,
                        width: padding_box.width,
                        height: padding_box.height,
                    });
                }

                // background
                cmd_buf.push(DrawCommand::DrawRect {
                    x: padding_box.x - border_box.x,
                    y: padding_box.y - border_box.y,
                    width: padding_box.width,
                    height: padding_box.height,
                    color: style.background_color,
                });

                // content + scroll
                cmd_buf.push(DrawCommand::PushTransform {
                    dx: content_box.x - border_box.x,
                    dy: content_box.y - border_box.y,
                });
                cmd_buf.push(DrawCommand::PushTransform {
                    dx: *scroll_offset_x,
                    dy: -*scroll_offset_y,
                });
            }
        }
    }

    for (child_child, child_info) in layout.children.iter().zip(&info.children) {
        match child_child {
            LayoutChild::Node(node) => {
                generate_draw_commands(cmd_buf, node, child_info);
            }
            LayoutChild::Fragment(frag_node) => {
                if let NodeKind::Text { texts, style } = &child_info.kind {
                    if let Some(text) = texts.first() {
                        let (x, y) = frag_node.placement.offset;

                        cmd_buf.push(DrawCommand::DrawText {
                            x,
                            y,
                            text: text.into(),
                            style: *style,
                        });

                        let font_size = style.font_size;
                        let line_thickness = (font_size * 0.08).max(1.0);
                        let (line_y, draw) = match style.text_decoration {
                            TextDecoration::None => (0.0, false),
                            TextDecoration::Underline => (y + font_size, true),
                            TextDecoration::LineThrough => (y + font_size * 0.5, true),
                            TextDecoration::Overline => (y, true),
                        };
                        if draw {
                            cmd_buf.push(DrawCommand::DrawRect {
                                x,
                                y: line_y,
                                width: frag_node.node.width(),
                                height: line_thickness,
                                color: style.color,
                            });
                        }
                    }
                }
            }
        }
    }

    // Pop commands for containers
    if matches!(info.kind, NodeKind::Container { .. }) {
        let is_inline = matches!(layout.layout_box, ui_layout::LayoutBox::InlineBox(_));
        for _ in &layout.layout_box {
            cmd_buf.push(DrawCommand::PopTransform);
            cmd_buf.push(DrawCommand::PopTransform);
            if !is_inline {
                cmd_buf.push(DrawCommand::PopClip);
            }
            cmd_buf.push(DrawCommand::PopTransform);
        }
    }
}
