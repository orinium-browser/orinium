use crate::engine::ui::CustomNode;

#[derive(Debug)]
pub struct InputHiddenComponent(String);

impl InputHiddenComponent {
    pub fn new(value: String) -> Self {
        Self(value)
    }
}

impl CustomNode for InputHiddenComponent {
    fn draw_sized(
        &self,
        _cmd_buf: &mut Vec<crate::engine::renderer_model::DrawCommand>,
        _text_style: &crate::engine::layouter::types::TextStyle,
        _style: &ui_layout::Style,
        _size: crate::engine::ui::ContentSize,
    ) {
    }

    fn intrinsic_size(&self) -> crate::engine::ui::ContentSize {
        crate::engine::ui::ContentSize::zero()
    }

    fn value(&self) -> Option<String> {
        Some(self.0.clone())
    }
}
