use crate::engine::{
    layouter::types::{Color, TextStyle},
    renderer_model::DrawCommand,
    ui::custom_node::CustomNode,
};

#[derive(Debug)]
pub struct ButtonComponent {
    pub label: String,
    pub color: Option<Color>,
}

impl CustomNode for ButtonComponent {
    fn draw(&self, cmd_buf: &mut Vec<DrawCommand>, text_style: &TextStyle) {
        cmd_buf.push(DrawCommand::DrawText {
            text: self.label.as_str().into(),
            x: 0.0,
            y: 0.0,
            style: text_style.clone(),
        });
    }

    fn background_color(&self) -> Option<Color> {
        self.color
    }

    fn intrinsic_size(&self) -> (f32, f32) {
        (120.0, 36.0)
    }
}
