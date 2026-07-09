use crate::engine::renderer_model::DrawCommand;

pub enum UiEvent {
    Scroll { dx: f32, dy: f32 },
}

pub trait UiComponent: ui_layout::CustomLayout {
    fn receive_event(&self, event: UiEvent);
    fn draw(&self, cmd_buf: &mut Vec<DrawCommand>);
}
