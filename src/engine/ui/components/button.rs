use ui_layout::Style;

use crate::engine::{
    layouter::types::{Color, TextStyle},
    renderer_model::DrawCommand,
    ui::custom_node::CustomNode,
};

#[derive(Debug)]
pub struct ButtonComponent {
    pub label: String,
    pub button_color: Color,
    pub label_color: Color,
}

impl CustomNode for ButtonComponent {
    fn draw_sized(
        &self,
        cmd_buf: &mut Vec<DrawCommand>,
        text_style: &TextStyle,
        _style: &Style,
        size: (f32, f32),
    ) {
        let mut style = text_style.clone();
        style.color = self.label_color;
        let y = ((size.1 - style.font_size) * 0.5).max(0.0);
        cmd_buf.push(DrawCommand::DrawText {
            text: self.label.as_str().into(),
            x: 0.0,
            y,
            style,
        });
    }

    fn background_color(&self) -> Option<Color> {
        Some(self.button_color)
    }

    fn intrinsic_size(&self) -> (f32, f32) {
        (120.0, 36.0)
    }
}
