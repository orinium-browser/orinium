use crate::engine::renderer_model::DrawCommand;

pub enum UiEvent {
    Scroll { dx: f32, dy: f32 },
}

pub trait UiComponent: ui_layout::CustomLayout {
    fn receive_event(event: UiEvent);
    fn draw(cmd_buf: &mut Vec<DrawCommand>);
}
