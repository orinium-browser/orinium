use crate::engine::renderer::DrawCommand;
use crate::platform::renderer::gpu::GpuRenderer;
use winit::event::WindowEvent;

pub enum BrowserCommand {
    Exit,
    None,
}
pub struct BrowserApp {
    draw_commands: Vec<DrawCommand>,
}

impl BrowserApp {
    pub fn new() -> Self {
        Self {
            draw_commands: vec![],
        }
    }

    // 開発テスト用
    pub fn with_draw_commands(draw_commands: Vec<DrawCommand>) -> Self {
        Self { draw_commands }
    }

    pub fn apply_draw_commands(&self, gpu: &mut GpuRenderer) {
        gpu.update_draw_commands(&self.draw_commands);
    }

    pub fn handle_window_event(&mut self, event: WindowEvent, gpu: &mut GpuRenderer) -> BrowserCommand {
        match event {
            WindowEvent::CloseRequested => {
                return BrowserCommand::Exit;
            }
            WindowEvent::RedrawRequested => {
                if let Ok(animating) = gpu.render() {
                    if animating {
                        self.apply_draw_commands(gpu);
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let scroll_amount = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => -y * 60.0,
                    winit::event::MouseScrollDelta::PixelDelta(pos) => -pos.y as f32,
                };
                gpu.scroll_text_by(scroll_amount);
                self.apply_draw_commands(gpu);
            }
            _ => {}
        }
        BrowserCommand::None
    }
}
