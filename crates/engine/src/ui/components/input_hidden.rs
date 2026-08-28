use crate::ui::CustomNode;

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
        _cmd_buf: &mut Vec<crate::renderer_model::DrawCommand>,
        _text_style: &crate::layouter::types::TextStyle,
        _text_flow_style: &crate::layouter::types::TextFlowStyle,
        _style: &ui_layout::Style,
        _size: crate::ui::ContentSize,
    ) {
    }

    fn intrinsic_size(&self) -> crate::ui::ContentSize {
        crate::ui::ContentSize::zero()
    }

    fn value(&self) -> Option<String> {
        Some(self.0.clone())
    }
}
