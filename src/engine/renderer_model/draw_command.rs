//! Draw command definition for rendering, which represents drawing instructions.

use crate::engine::layouter::types::{Color, InfoNode, NodeKind, TextDecoration, TextStyle};
use smol_str::SmolStr;
use ui_layout::{LayoutChild, LayoutNode};

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

/// Per-box-model push state for balanced pop generation.
#[derive(Default, Clone, Copy)]
struct BoxPushState {
    border: bool,
    clip: bool,
    content: bool,
    scroll: bool,
}

/// LayoutNode + InfoNode → DrawCommand
pub fn generate_draw_commands(
    cmd_buf: &mut Vec<DrawCommand>,
    layout: &LayoutNode,
    info: &InfoNode,
) {
    let mut box_states: Vec<BoxPushState> = Vec::new();

    match &info.kind {
        NodeKind::Text { .. } | NodeKind::LineBreak => unreachable!(),

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
                let is_inline = matches!(layout.layout_box, ui_layout::LayoutBox::InlineBox(_));

                let mut state = BoxPushState::default();

                if border_box.x != 0.0 || border_box.y != 0.0 {
                    cmd_buf.push(DrawCommand::PushTransform {
                        dx: border_box.x,
                        dy: border_box.y,
                    });
                    state.border = true;
                }

                if !is_inline {
                    // Inline elements are fragmented across lines, so their borders
                    // cannot be drawn as a single rectangle per box model.
                    // TODO: support inline borders (draw per fragment).
                    let bc = &style.border_color;

                    // 各辺の border 幅 = border_box と padding_box の座標差
                    let bw_top = (padding_box.y - border_box.y).max(0.0);
                    let bw_bottom = (border_box.y + border_box.height
                        - (padding_box.y + padding_box.height))
                        .max(0.0);
                    let bw_left = (padding_box.x - border_box.x).max(0.0);
                    let bw_right = (border_box.x + border_box.width
                        - (padding_box.x + padding_box.width))
                        .max(0.0);

                    // 上下は full-width、左右は上下と重ならないよう y をずらして高さを詰める。
                    // CSS 仕様上コーナーでは上下の色が優先される（top > left > bottom > right）。
                    if bw_top > 0.0 {
                        cmd_buf.push(DrawCommand::DrawRect {
                            x: 0.0,
                            y: 0.0,
                            width: border_box.width,
                            height: bw_top,
                            color: bc.top,
                        });
                    }

                    if bw_bottom > 0.0 {
                        cmd_buf.push(DrawCommand::DrawRect {
                            x: 0.0,
                            y: border_box.height - bw_bottom,
                            width: border_box.width,
                            height: bw_bottom,
                            color: bc.bottom,
                        });
                    }

                    if bw_left > 0.0 {
                        cmd_buf.push(DrawCommand::DrawRect {
                            x: 0.0,
                            y: bw_top,
                            width: bw_left,
                            height: border_box.height - bw_top - bw_bottom,
                            color: bc.left,
                        });
                    }

                    if bw_right > 0.0 {
                        cmd_buf.push(DrawCommand::DrawRect {
                            x: border_box.width - bw_right,
                            y: bw_top,
                            width: bw_right,
                            height: border_box.height - bw_top - bw_bottom,
                            color: bc.right,
                        });
                    }
                }

                if !is_inline && padding_box.width > 0.0 && padding_box.height > 0.0 {
                    cmd_buf.push(DrawCommand::PushClip {
                        x: padding_box.x - border_box.x,
                        y: padding_box.y - border_box.y,
                        width: padding_box.width,
                        height: padding_box.height,
                    });
                    state.clip = true;
                }

                if style.background_color.3 > 0 {
                    cmd_buf.push(DrawCommand::DrawRect {
                        x: padding_box.x - border_box.x,
                        y: padding_box.y - border_box.y,
                        width: padding_box.width,
                        height: padding_box.height,
                        color: style.background_color,
                    });
                }

                let dx = content_box.x - border_box.x;
                let dy = content_box.y - border_box.y;
                if dx != 0.0 || dy != 0.0 {
                    cmd_buf.push(DrawCommand::PushTransform { dx, dy });
                    state.content = true;
                }

                if *scroll_offset_x != 0.0 || *scroll_offset_y != 0.0 {
                    cmd_buf.push(DrawCommand::PushTransform {
                        dx: *scroll_offset_x,
                        dy: -*scroll_offset_y,
                    });
                    state.scroll = true;
                }

                box_states.push(state);
            }
        }
    }

    let mut layout_iter = layout.children.iter();

    for child_info in &info.children {
        match &child_info.kind {
            NodeKind::Text { texts, style } => {
                for text in texts {
                    if let Some(LayoutChild::Fragment(frag_node)) = layout_iter.next() {
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
            NodeKind::LineBreak => {
                layout_iter.next();
            }
            NodeKind::Container { .. } => {
                if let Some(LayoutChild::Node(node)) = layout_iter.next() {
                    generate_draw_commands(cmd_buf, node, child_info);
                }
            }
        }
    }

    // Pop commands for containers (reverse order of pushes)
    if matches!(info.kind, NodeKind::Container { .. }) {
        for state in box_states.iter().rev() {
            if state.scroll {
                cmd_buf.push(DrawCommand::PopTransform);
            }
            if state.content {
                cmd_buf.push(DrawCommand::PopTransform);
            }
            if state.clip {
                cmd_buf.push(DrawCommand::PopClip);
            }
            if state.border {
                cmd_buf.push(DrawCommand::PopTransform);
            }
        }
    }
}
