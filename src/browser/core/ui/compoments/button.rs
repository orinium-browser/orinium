use super::{ComponentEvent, DrawCommandEmitter};
use crate::engine::layouter::types::{Color, FontWeight, TextStyle};
use crate::engine::renderer_model::DrawCommand;
use ui_layout::LayoutNode;

/// ボタンコンポーネントの定義
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
        let mut commands = Vec::new();

        for box_model in &self.layout.layout_boxes {
            let rect = box_model.padding_box;
            // border
            let border_color = if self.focused {
                Color(255, 200, 0, 255)
            } else {
                Color(10, 10, 10, 255)
            };
            commands.push(DrawCommand::DrawRect {
                x: rect.x - 2.0,
                y: rect.y - 2.0,
                width: rect.width + 4.0,
                height: rect.height + 4.0,
                color: border_color,
            });
            let fill_color = if self.active {
                Color(20, 90, 200, 255)
            } else if self.hovered {
                Color(60, 140, 255, 255)
            } else {
                Color(40, 120, 240, 255)
            };
            commands.push(DrawCommand::DrawRect {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                color: fill_color,
            });
            commands.push(DrawCommand::DrawText {
                x: rect.x + rect.width / 2.0,
                y: rect.y + rect.height / 2.0 - 8.0,
                text: self.label.clone(),
                style: text_style,
                max_width: rect.width - 16.0,
            });
        }

        commands
    }
}

pub fn draw_from_layout(layout: &LayoutNode) -> Vec<DrawCommand> {
    layout
        .layout_boxes
        .iter()
        .map(|box_model| {
            let rect = box_model.padding_box;
            DrawCommand::DrawRect {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                color: Color(230, 230, 230, 255),
            }
        })
        .collect()
}

pub fn handle_pointer_down(x: f32, y: f32) {
    log::info!("HTML <button> component handled PointerDown at ({}, {})", x, y);
}
