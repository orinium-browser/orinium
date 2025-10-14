use std::sync::Arc;

use crate::engine::renderer::DrawCommand;
use crate::platform::renderer::gpu::GpuRenderer;

#[allow(unused_imports)]
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::Window,
};

pub struct State {
    window: Arc<Window>,
    gpu_renderer: GpuRenderer,
}

pub struct App {
    state: Option<State>,
    draw_commands: Vec<DrawCommand>,
}

impl State {
    pub async fn new(window: Arc<Window>) -> anyhow::Result<Self> {
        let gpu_renderer = GpuRenderer::new(window.clone()).await?;
        Ok(Self {
            window,
            gpu_renderer,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.gpu_renderer
            .resize(winit::dpi::PhysicalSize::new(width, height));
    }

    pub fn render(&mut self) -> anyhow::Result<()> {
        self.gpu_renderer.render()?;
        Ok(())
    }

    pub fn get_gpu_renderer(&mut self) -> &mut GpuRenderer {
        &mut self.gpu_renderer
    }
}

#[allow(dead_code)]
impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            state: None,
            draw_commands: Vec::new(),
        }
    }

    pub fn set_draw_commands(&mut self, commands: Vec<DrawCommand>) {
        self.draw_commands = commands;
        if let Some(state) = &mut self.state {
            state.gpu_renderer.update_draw_commands(&self.draw_commands);
        }
    }
}

impl ApplicationHandler<State> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        #[allow(unused_mut)]
        let mut window_attributes = Window::default_attributes();

        let window = Arc::new(event_loop.create_window(window_attributes).unwrap());

        self.state = Some(pollster::block_on(State::new(window)).unwrap());

        if !self.draw_commands.is_empty() {
            log::info!(
                "Applying {} draw commands to GPU renderer",
                self.draw_commands.len()
            );
            if let Some(state) = &mut self.state {
                state.gpu_renderer.update_draw_commands(&self.draw_commands);
                log::info!("Draw commands applied successfully");
                state.window.request_redraw();
            }
        } else {
            log::warn!("No draw commands to apply");
        }
    }

    #[allow(unused_mut)]
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, mut event: State) {
        self.state = Some(event);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let state = match &mut self.state {
            Some(canvas) => canvas,
            None => return,
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => state.resize(size.width, size.height),
            WindowEvent::RedrawRequested => {
                state.render().ok();
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state: key_state,
                        ..
                    },
                ..
            } => {
                if let (KeyCode::Escape, true) = (code, key_state.is_pressed()) {
                    event_loop.exit()
                }
            }
            _ => {}
        }
    }
}
