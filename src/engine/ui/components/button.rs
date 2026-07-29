use ui_layout::Length;

use crate::engine::{
    layouter::types::{Color, TextStyle},
    renderer_model::DrawCommand,
    ui::custom_node::CustomNode,
};

#[derive(Debug)]
pub struct ButtonComponent {
    pub width: Length,
    pub height: Length,
    pub label: String,
    pub button_color: Color,
    pub label_color: Color,
}

impl CustomNode for ButtonComponent {
    fn draw(&self, cmd_buf: &mut Vec<DrawCommand>, text_style: &TextStyle) {
        let mut style = text_style.clone();
        style.color = self.label_color;
        cmd_buf.push(DrawCommand::DrawText {
            text: self.label.as_str().into(),
            x: 0.0,
            y: 0.0,
            style,
        });
    }

    fn background_color(&self) -> Option<Color> {
        Some(self.button_color)
    }

    fn intrinsic_size(&self) -> (f32, f32) {
        (
            self.width.resolve_with(None, 0.0, 0.0).unwrap_or(120.0),
            self.height.resolve_with(None, 0.0, 0.0).unwrap_or(36.0),
        )
    }
}
