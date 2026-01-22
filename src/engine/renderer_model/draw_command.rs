use crate::engine::layouter::types::{Color, InfoNode, NodeKind, TextDecoration, TextStyle};
use ui_layout::LayoutNode;

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
        NodeKind::Text { text, style, .. } => {
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

            let font_size = style.font_size;
            let line_thickness = (font_size * 0.08).max(1.0);

            let (line_y, draw) = match style.text_decoration {
                TextDecoration::None => (0.0, false),
                TextDecoration::Underline => (abs_y + font_size, true),
                TextDecoration::LineThrough => (abs_y + font_size * 0.5, true),
                TextDecoration::Overline => (abs_y, true),
            };

            if draw {
                commands.push(DrawCommand::DrawRect {
                    x: abs_x,
                    y: line_y,
                    width: rect.width,
                    height: line_thickness,
                    color: style.color,
                });
            }
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
                dy: -*scroll_offset_y,
            });
        }
        NodeKind::Link { style, .. } => {
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
        }
    }

    for (child_layout, child_info) in layout.children.iter().zip(&info.children) {
        commands.extend(generate_draw_commands(child_layout, child_info));
    }

    if matches!(info.kind, NodeKind::Container { .. }) {
        commands.push(DrawCommand::PopTransform);
        commands.push(DrawCommand::PopClip);
        commands.push(DrawCommand::PopTransform);
    } else if matches!(info.kind, NodeKind::Link { .. }) {
        commands.push(DrawCommand::PopClip);
        commands.push(DrawCommand::PopTransform);
    }

    commands
}
