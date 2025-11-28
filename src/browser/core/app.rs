use super::tab::Tab;
// use super::ui::init_browser_ui;

use crate::engine::renderer::{DrawCommand, RenderTree};
use crate::platform::renderer::gpu::GpuRenderer;
use crate::system::App;

use anyhow::Result;
use winit::event::WindowEvent;

pub enum BrowserCommand {
    Exit,
    RequestRedraw,
    None,
}

pub struct BrowserApp {
    #[allow(unused)]
    tabs: Vec<Tab>,
    // render_tree: RenderTree,
    draw_commands: Vec<DrawCommand>,
    window_size: (u32, u32), // (x, y)
    window_title: String,
}

impl Default for BrowserApp {
    fn default() -> Self {
        Self::new((800, 600), "Orinium Browser".to_string())
    }
}

impl BrowserApp {
    pub fn run(self) -> Result<()> {
        let event_loop =
            winit::event_loop::EventLoop::<crate::platform::system::State>::with_user_event()
                .build()?;
        let mut app = App::new(self);
        event_loop.run_app(&mut app)?;
        Ok(())
    }

    pub fn new(window_size: (u32, u32), window_title: String) -> Self {
        //let (render_tree, draw_commands) = init_browser_ui(window_size);
        Self {
            tabs: vec![],
            // render_tree,
            draw_commands: vec![],
            window_size,
            window_title,
        }
    }

    // 開発テスト用
    pub fn with_draw_info(
        mut self,
        _render_tree: RenderTree,
        draw_commands: Vec<DrawCommand>,
    ) -> Self {
        // self.render_tree = render_tree;
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

    pub fn window_size(&self) -> (u32, u32) {
        self.window_size
    }

    pub fn window_title(&self) -> String {
        self.window_title.clone()
    }
}
