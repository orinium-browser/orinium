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
    font_path: Option<String>,
}

impl State {
    pub async fn new(window: Arc<Window>, font_path: Option<&str>) -> anyhow::Result<Self> {
        let gpu_renderer = GpuRenderer::new(window.clone(), font_path).await?;
        Ok(Self {
            window,
            gpu_renderer,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.gpu_renderer
            .resize(winit::dpi::PhysicalSize::new(width, height));
    }

    pub fn render(&mut self) -> anyhow::Result<bool> {
        let animating = self.gpu_renderer.render()?;
        Ok(animating)
    }

    pub fn get_gpu_renderer(&mut self) -> &mut GpuRenderer {
        &mut self.gpu_renderer
    }
}

#[allow(dead_code)]
impl Default for App {
    fn default() -> Self {
        Self::new(None)
    }
}

impl App {
    pub fn new(font_path: Option<String>) -> Self {
        Self {
            state: None,
            draw_commands: Vec::new(),
            font_path,
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

        let font_path_ref = self.font_path.as_deref();
        self.state = Some(pollster::block_on(State::new(window, font_path_ref)).unwrap());

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
                match state.gpu_renderer.render() {
                    Ok(animating) => {
                        if animating && !self.draw_commands.is_empty() {
                            // update queued text sections to reflect current animated scroll
                            state.gpu_renderer.update_draw_commands(&self.draw_commands);
                            state.window.request_redraw();
                        }
                    }
                    Err(e) => {
                        log::error!("render error: {}", e);
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // delta.y: positive when scrolling up on some platforms; invert if needed
                let scroll_amount = match delta {
                    MouseScrollDelta::LineDelta(_x, y) => -y * 60.0, // make wheel scroll larger
                    MouseScrollDelta::PixelDelta(pos) => -pos.y as f32,
                };
                state.gpu_renderer.scroll_text_by(scroll_amount);
                log::debug!("mouse wheel scroll_amount={} text_scroll_before={}", scroll_amount, state.gpu_renderer.text_scroll());
                if !self.draw_commands.is_empty() {
                    state.gpu_renderer.update_draw_commands(&self.draw_commands);
                }
                log::debug!("text_scroll_after={}", state.gpu_renderer.text_scroll());
                state.window.request_redraw();
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
                if key_state.is_pressed() {
                    match code {
                        KeyCode::Escape => event_loop.exit(),
                        KeyCode::PageDown => {
                            // 大きくスクロールしたとき
                            let win_size = state.window.inner_size();
                            let dy = (win_size.height as f32) * 0.9;
                            state.gpu_renderer.scroll_text_by(dy);
                            if !self.draw_commands.is_empty() {
                                state.gpu_renderer.update_draw_commands(&self.draw_commands);
                            }
                            state.window.request_redraw();
                        }
                        KeyCode::PageUp => {
                            let win_size = state.window.inner_size();
                            let dy = -(win_size.height as f32) * 0.9;
                            state.gpu_renderer.scroll_text_by(dy);
                            if !self.draw_commands.is_empty() {
                                state.gpu_renderer.update_draw_commands(&self.draw_commands);
                            }
                            state.window.request_redraw();
                        }
                        KeyCode::ArrowDown => {
                            state.gpu_renderer.scroll_text_by(40.0);
                            if !self.draw_commands.is_empty() {
                                state.gpu_renderer.update_draw_commands(&self.draw_commands);
                            }
                            state.window.request_redraw();
                        }
                        KeyCode::ArrowUp => {
                            state.gpu_renderer.scroll_text_by(-40.0);
                            if !self.draw_commands.is_empty() {
                                state.gpu_renderer.update_draw_commands(&self.draw_commands);
                            }
                            state.window.request_redraw();
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}
