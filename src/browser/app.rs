use super::tab::Tab;
use crate::engine::renderer::DrawCommand;
use crate::platform::renderer::gpu::GpuRenderer;
use winit::event::WindowEvent;

pub enum BrowserCommand {
    Exit,
    RequestRedraw,
    None,
}

pub struct BrowserApp {
    tabs: Tab,
    draw_commands: Vec<DrawCommand>,
}

impl Default for BrowserApp {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserApp {
    pub fn new() -> Self {
        Self {
            tabs: Tab::new(),
            draw_commands: vec![],
        }
    }

    // 開発テスト用
    pub fn with_draw_commands(mut self, draw_commands: Vec<DrawCommand>) -> Self {
        self.draw_commands = draw_commands;
        self
    }

    pub fn apply_draw_commands(&self, gpu: &mut GpuRenderer) {
        gpu.parse_draw_commands(&self.draw_commands);
    }

    pub fn handle_window_event(
        &mut self,
        event: WindowEvent,
        gpu: &mut GpuRenderer,
    ) -> BrowserCommand {
        match event {
            WindowEvent::CloseRequested => {
                return BrowserCommand::Exit;
            }
            WindowEvent::RedrawRequested => {
                if let Ok(animating) = gpu.render()
                    && animating
                {
                    self.apply_draw_commands(gpu);
                }
            }
            /*
            WindowEvent::MouseWheel { delta, .. } => {
                let scroll_amount = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => -y * 60.0,
                    winit::event::MouseScrollDelta::PixelDelta(pos) => -pos.y as f32,
                };
                self.apply_draw_commands(gpu);
                return BrowserCommand::RequestRedraw;
            }
            */
            _ => {}
        }
        BrowserCommand::None
    }
}
