use super::{ComponentEvent, DrawCommandEmitter};
use crate::engine::layouter::types::{Color, FontWeight, TextStyle};
use crate::engine::renderer_model::DrawCommand;
use ui_layout::LayoutNode;

/// Button
#[derive(Debug)]
pub struct Button {
    pub id: String,
    pub label: String,
    pub layout: LayoutNode,
    pub hovered: bool,
    pub active: bool,
    pub focused: bool,
}

impl Button {
    pub fn new(id: impl Into<String>, label: impl Into<String>, layout: LayoutNode) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            layout,
            hovered: false,
            active: false,
            focused: false,
        }
    }

    pub fn hit_test(&self, x: f32, y: f32) -> bool {
        self.layout.layout_boxes.iter().any(|box_model| {
            let rect = box_model.padding_box;
            x >= rect.x && x <= rect.x + rect.width && y >= rect.y && y <= rect.y + rect.height
        })
    }

    pub fn on_event(&mut self, event: ComponentEvent) -> bool {
        match event {
            ComponentEvent::PointerDown { x, y } => {
                log::info!("Button {} handled PointerDown at ({}, {})", self.id, x, y);
                self.active = true;
                true
            }
        }
    }
}

impl DrawCommandEmitter for Button {
    fn draw_commands(&self) -> Vec<DrawCommand> {
        let text_style = TextStyle {
            font_size: 16.0,
            font_weight: FontWeight::BOLD,
            color: Color(255, 255, 255, 255),
            ..Default::default()
        };

        self.layout
            .layout_boxes
            .iter()
            .flat_map(|box_model| {
                let rect = box_model.padding_box;
                let border_color = if self.focused {
                    Color(20, 20, 20, 255)
                } else {
                    Color(10, 10, 10, 255)
                };
                let fill_color = if self.active {
                    Color(20, 90, 200, 255)
                } else if self.hovered {
                    Color(60, 140, 255, 255)
                } else {
                    Color(40, 120, 240, 255)
                };

                vec![
                    DrawCommand::PushTransform { dx: rect.x, dy: rect.y },
                    DrawCommand::DrawRect {
                        x: -2.0,
                        y: -2.0,
                        width: rect.width + 4.0,
                        height: rect.height + 4.0,
                        color: border_color,
                    },
                    DrawCommand::DrawRect {
                        x: 0.0,
                        y: 0.0,
                        width: rect.width,
                        height: rect.height,
                        color: fill_color,
                    },
                    DrawCommand::PushClip { x: 0.0, y: 0.0, width: rect.width, height: rect.height },
                    DrawCommand::DrawText {
                        x: 0.0,
                        y: (rect.height - text_style.font_size) / 2.0 - 3.0,
                        text: self.label.clone(),
                        style: TextStyle {
                            font_size: text_style.font_size,
                            font_weight: text_style.font_weight,
                            color: text_style.color,
                            text_align: crate::engine::layouter::types::TextAlign::Center,
                            ..Default::default()
                        },
                        max_width: rect.width * 2.0,
                    },
                    DrawCommand::PopClip,
                    DrawCommand::PopTransform,
                ]
            })
            .collect()
    }
}

pub fn find_first_text(info: &crate::engine::layouter::types::InfoNode) -> Option<String> {
    match &info.kind {
        crate::engine::layouter::types::NodeKind::Text { text, .. } => Some(text.clone()),
        _ => info.children.iter().filter_map(|c| find_first_text(c)).next(),
    }
}

fn find_first_text_style(info: &crate::engine::layouter::types::InfoNode) -> Option<TextStyle> {
    match &info.kind {
        crate::engine::layouter::types::NodeKind::Text { style, .. } => Some(*style),
        _ => info.children.iter().filter_map(|c| find_first_text_style(c)).next(),
    }
}

pub fn draw_from_layout(layout: &LayoutNode, info: &crate::engine::layouter::types::InfoNode) -> Vec<DrawCommand> {
    let default_font_size = 14.0;

    layout
        .layout_boxes
        .iter()
        .flat_map(|box_model| {
            // Use padding_box as the outer drawing origin, but derive the content area from ui_layout so
            // text is laid out and clipped precisely inside the content_box (respecting padding/border).
            let outer = box_model.padding_box;
            let content = box_model.content_box;
            let label = find_first_text(info).unwrap_or_else(|| "".to_string());
            let text_style = find_first_text_style(info).unwrap_or(TextStyle {
                font_size: default_font_size,
                ..Default::default()
            });
            let font_size = text_style.font_size.max(10.0);

            // clip coordinates relative to the transform origin (outer.x/outer.y)
            let clip_x_rel = content.x - outer.x;
            let clip_y_rel = content.y - outer.y;

            // center text vertically inside content box
            let text_y = clip_y_rel + (content.height - font_size) / 2.0;

            vec![
                DrawCommand::PushTransform { dx: outer.x, dy: outer.y },

                DrawCommand::DrawRect {
                    x: 0.0,
                    y: 1.0,
                    width: outer.width,
                    height: outer.height,
                    color: Color(0, 0, 0, 40),
                },
                // fill (background)
                DrawCommand::DrawRect {
                    x: 0.0,
                    y: 0.0,
                    width: outer.width,
                    height: outer.height,
                    color: Color(255, 255, 255, 255),
                },
                // border (drawn after fill so it appears on top)
                DrawCommand::DrawRect {
                    x: -1.0,
                    y: -1.0,
                    width: outer.width + 2.0,
                    height: outer.height + 2.0,
                    color: Color(200, 200, 200, 255),
                },
                // clip to content box and draw text inside
                DrawCommand::PushClip { x: clip_x_rel, y: clip_y_rel, width: content.width, height: content.height },
                DrawCommand::DrawText {
                    // x is relative to current transform origin; use clip_x_rel so that the text's layout
                    // origin aligns with the content box's left edge (so center alignment centers inside content)
                    x: clip_x_rel,
                    y: text_y,
                    text: label.clone(),
                    style: TextStyle {
                        font_size,
                        font_weight: FontWeight::BOLD,
                        color: Color(30, 30, 30, 255),
                        text_align: crate::engine::layouter::types::TextAlign::Center,
                        ..Default::default()
                    },
                    max_width: content.width,
                },
                DrawCommand::PopClip,
                DrawCommand::PopTransform,
            ]
        })
        .collect()
}

pub fn handle_pointer_down(x: f32, y: f32) {
    log::info!("HTML <button> component handled PointerDown at ({}, {})", x, y);
}
