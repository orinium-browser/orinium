mod components;

pub use components::*;

use crate::engine::renderer_model::DrawCommand;
use ui_layout::LayoutChild;

pub enum UiEvent {
    Scroll { dx: f32, dy: f32 },
}

pub trait UiComponent {
    fn receive_event(&self, event: UiEvent);
    fn as_layout_child(&self) -> LayoutChild;
    fn draw(&self, cmd_buf: &mut Vec<DrawCommand>);
}
