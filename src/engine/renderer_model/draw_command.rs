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
pub fn generate_draw_commands(layout: &LayoutNode, info: &InfoNode) -> Vec<DrawCommand> {
    let mut commands = Vec::new();

    match &info.kind {
        NodeKind::Text { text, style, .. } => {
            for box_model in &layout.layout_boxes {
                let rect = box_model.padding_box;

                let abs_x = rect.x;
                let abs_y = rect.y;

                // テキスト
                commands.push(DrawCommand::DrawText {
                    x: abs_x,
                    y: abs_y,
                    text: text.clone(),
                    style: *style,
                    max_width: rect.width,
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
                    commands.push(DrawCommand::DrawRect {
                        x: abs_x,
                        y: line_y,
                        width: rect.width,
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
            for box_model in &layout.layout_boxes {
                let border_box = box_model.border_box;
                let padding_box = box_model.padding_box;
                let content_box = box_model.content_box;

                // コンテナ全体を移動
                commands.push(DrawCommand::PushTransform {
                    dx: border_box.x,
                    dy: border_box.y,
                });

                let bc = &style.border_color;

                // border (top)
                let top_width = padding_box.y - border_box.y;
                if top_width > 0.0 {
                    commands.push(DrawCommand::DrawRect {
                        x: 0.0,
                        y: 0.0,
                        width: border_box.width,
                        height: top_width,
                        color: bc.top,
                    });
                }

                // bottom
                let bottom_width =
                    border_box.height - (padding_box.y - border_box.y + padding_box.height);
                if bottom_width > 0.0 {
                    commands.push(DrawCommand::DrawRect {
                        x: 0.0,
                        y: border_box.height - bottom_width,
                        width: border_box.width,
                        height: bottom_width,
                        color: bc.bottom,
                    });
                }

                // left
                let left_width = padding_box.x - border_box.x;
                if left_width > 0.0 {
                    commands.push(DrawCommand::DrawRect {
                        x: 0.0,
                        y: 0.0,
                        width: left_width,
                        height: border_box.height,
                        color: bc.left,
                    });
                }

                // right
                let right_width =
                    border_box.width - (padding_box.x - border_box.x + padding_box.width);
                if right_width > 0.0 {
                    commands.push(DrawCommand::DrawRect {
                        x: border_box.width - right_width,
                        y: 0.0,
                        width: right_width,
                        height: border_box.height,
                        color: bc.right,
                    });
                }

                // clip
                commands.push(DrawCommand::PushClip {
                    x: padding_box.x - border_box.x,
                    y: padding_box.y - border_box.y,
                    width: padding_box.width,
                    height: padding_box.height,
                });

                // background
                commands.push(DrawCommand::DrawRect {
                    x: padding_box.x - border_box.x,
                    y: padding_box.y - border_box.y,
                    width: padding_box.width,
                    height: padding_box.height,
                    color: style.background_color,
                });

                // content + scroll
                commands.push(DrawCommand::PushTransform {
                    dx: content_box.x - border_box.x,
                    dy: content_box.y - border_box.y,
                });
                commands.push(DrawCommand::PushTransform {
                    dx: *scroll_offset_x,
                    dy: -*scroll_offset_y,
                });
            }
        }
    }

    for (child_layout, child_info) in layout.children.iter().zip(&info.children) {
        commands.extend(generate_draw_commands(child_layout, child_info));
    }

    // Pop commands for containers
    if matches!(info.kind, NodeKind::Container { .. }) {
        for _ in &layout.layout_boxes {
            commands.push(DrawCommand::PopTransform);
            commands.push(DrawCommand::PopTransform);
            commands.push(DrawCommand::PopClip);
            commands.push(DrawCommand::PopTransform);
        }
    }

    commands
}
