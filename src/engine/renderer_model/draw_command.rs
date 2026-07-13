//! Draw command definition for rendering, which represents drawing instructions.

use crate::engine::layouter::text_layouter::TextFlowLayouter;
use crate::engine::layouter::types::{
    Background, Color, Gradient, InfoNode, NodeKind, TextDecoration, TextStyle,
};
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
    DrawGradientRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        gradient: Gradient,
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

// --------------------------------
// Helpers
// --------------------------------

fn push_transform(cmd_buf: &mut Vec<DrawCommand>, dx: f32, dy: f32) -> bool {
    if dx != 0.0 || dy != 0.0 {
        cmd_buf.push(DrawCommand::PushTransform { dx, dy });
        true
    } else {
        false
    }
}

/// Draw the four border edges inside the current coordinate system.
/// Coordinates are relative to the border-box origin.
fn draw_border(
    cmd_buf: &mut Vec<DrawCommand>,
    border_box: &ui_layout::Rect,
    padding_box: &ui_layout::Rect,
    bc: &crate::engine::layouter::types::BorderColor,
) {
    let bw_top = (padding_box.y - border_box.y).max(0.0);
    let bw_bottom =
        (border_box.y + border_box.height - (padding_box.y + padding_box.height)).max(0.0);
    let bw_left = (padding_box.x - border_box.x).max(0.0);
    let bw_right = (border_box.x + border_box.width - (padding_box.x + padding_box.width)).max(0.0);

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

/// Draw the background inside the padding box.
/// Coordinates are relative to the border-box origin.
fn draw_background(
    cmd_buf: &mut Vec<DrawCommand>,
    border_box: &ui_layout::Rect,
    padding_box: &ui_layout::Rect,
    background: &Background,
) {
    let x = padding_box.x - border_box.x;
    let y = padding_box.y - border_box.y;
    match background {
        Background::Color(c) if c.3 > 0 => {
            cmd_buf.push(DrawCommand::DrawRect {
                x,
                y,
                width: padding_box.width,
                height: padding_box.height,
                color: *c,
            });
        }
        Background::Gradient(g) => {
            cmd_buf.push(DrawCommand::DrawGradientRect {
                x,
                y,
                width: padding_box.width,
                height: padding_box.height,
                gradient: g.clone(),
            });
        }
        _ => {}
    }
}

/// Push all draw commands for a single box model, returning the pop state.
fn push_box_model(
    cmd_buf: &mut Vec<DrawCommand>,
    box_model: &ui_layout::BoxModel,
    style: &crate::engine::layouter::types::ContainerStyle,
    scroll_offset_x: f32,
    scroll_offset_y: f32,
    is_inline: bool,
) -> BoxPushState {
    let border_box = box_model.border_box;
    let padding_box = box_model.padding_box;
    let content_box = box_model.content_box;

    let clip = !is_inline && padding_box.width > 0.0 && padding_box.height > 0.0;
    let dx = content_box.x - border_box.x;
    let dy = content_box.y - border_box.y;

    let state = BoxPushState {
        border: push_transform(cmd_buf, border_box.x, border_box.y),
        clip: if clip {
            cmd_buf.push(DrawCommand::PushClip {
                x: padding_box.x - border_box.x,
                y: padding_box.y - border_box.y,
                width: padding_box.width,
                height: padding_box.height,
            });
            true
        } else {
            false
        },
        content: push_transform(cmd_buf, dx, dy),
        scroll: push_transform(cmd_buf, scroll_offset_x, -scroll_offset_y),
    };

    // Inline elements are fragmented across lines, so their borders
    // cannot be drawn as a single rectangle per box model.
    // TODO: support inline borders (draw per fragment).
    if !is_inline {
        draw_border(cmd_buf, &border_box, &padding_box, &style.border_color);
    }

    draw_background(cmd_buf, &border_box, &padding_box, &style.background);

    state
}

/// Pop commands for a single box model (reverse order of pushes).
fn pop_box_model(cmd_buf: &mut Vec<DrawCommand>, state: BoxPushState) {
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

/// Draw text spans for a single text node.
fn draw_text(cmd_buf: &mut Vec<DrawCommand>, style: &TextStyle, text_id: usize) {
    if let Some(result) = TextFlowLayouter::get_result(text_id) {
        for (i, line_text) in result.line_texts.iter().enumerate() {
            let span = &result.spans[i];
            let (x, y) = span.line_pos;

            cmd_buf.push(DrawCommand::DrawText {
                x,
                y,
                text: line_text.as_str().into(),
                style: style.clone(),
            });

            let font_size = style.font_size;
            let line_thickness = (font_size * 0.08).max(1.0);
            let line_y_adj = if line_text.is_empty() {
                y
            } else {
                y + font_size
            };
            let (line_y, draw) = match style.text_decoration {
                TextDecoration::None => (0.0, false),
                TextDecoration::Underline => (line_y_adj, true),
                TextDecoration::LineThrough => (y + font_size * 0.5, true),
                TextDecoration::Overline => (y, true),
            };

            if draw {
                cmd_buf.push(DrawCommand::DrawRect {
                    x,
                    y: line_y,
                    width: span.x_range.end - span.x_range.start,
                    height: line_thickness,
                    color: style.text_decoration_color.unwrap_or(style.color),
                });
            }
        }
    }
}

// --------------------------------
// Public entry point
// --------------------------------

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
            let is_inline = matches!(layout.layout_box, ui_layout::LayoutBox::InlineBox(_));

            for box_model in &layout.layout_box {
                box_states.push(push_box_model(
                    cmd_buf,
                    &box_model,
                    style,
                    *scroll_offset_x,
                    *scroll_offset_y,
                    is_inline,
                ));
            }
        }
    }

    let mut layout_iter = layout.children.iter();

    for child_info in &info.children {
        match &child_info.kind {
            NodeKind::Text { text_id, style, .. } => {
                draw_text(cmd_buf, style, *text_id);
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

    if matches!(info.kind, NodeKind::Container { .. }) {
        for state in box_states.iter().rev() {
            pop_box_model(cmd_buf, *state);
        }
    }
}
