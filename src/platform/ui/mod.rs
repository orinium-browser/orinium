use std::sync::Arc;

use crate::engine::renderer::DrawCommand;
use crate::platform::renderer::gpu::GpuRenderer;
use crate::platform::renderer::scroll_bar::ScrollBar;

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
    // Scrollbar UI state
    scroll_bar: ScrollBar,
    last_cursor: (f32, f32),
    dragging_scrollbar: bool,
    drag_start_y: f32,
    drag_start_scroll: f32,
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
            scroll_bar: ScrollBar::new(),
            last_cursor: (0.0, 0.0),
            dragging_scrollbar: false,
            drag_start_y: 0.0,
            drag_start_scroll: 0.0,
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
            WindowEvent::Resized(size) => {
                // レンダラーのサイズを更新
                state.resize(size.width, size.height);
                // 既に描画コマンドがある場合は頂点バッファ（スクロールバー含む）を再生成
                if !self.draw_commands.is_empty() {
                    state.gpu_renderer.update_draw_commands(&self.draw_commands);
                }
                // 変化後すぐに1フレーム描画して古い頂点が残る表示を防ぐ
                if let Err(e) = state.gpu_renderer.render() {
                    log::error!("render on resize error: {}", e);
                }
                state.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                match state.gpu_renderer.render() {
                    Ok(animating) => {
                        if animating && !self.draw_commands.is_empty() {
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
            WindowEvent::CursorMoved { position, .. } => {
                let (x, y) = (position.x as f32, position.y as f32);

                 if self.dragging_scrollbar {
                    let vw = state.window.inner_size().width as f32;
                    let vh = state.window.inner_size().height as f32;
                    let content_h = state.gpu_renderer.content_height();
                    if let Some((_x1, y1, _x2, y2)) = self.scroll_bar.thumb_rect(vw, vh, content_h, self.drag_start_scroll) {
                        let thumb_h = y2 - y1;
                        let max_thumb_top = (vh - 2.0 * self.scroll_bar.margin - thumb_h).max(0.0);
                        if max_thumb_top > 0.0 {
                            let dy = y - self.drag_start_y;
                            let scrollable = (content_h - vh).max(0.0);
                            let delta_scroll = dy / max_thumb_top * scrollable;
                            let new_scroll = (self.drag_start_scroll + delta_scroll).clamp(0.0, scrollable);
                            state.gpu_renderer.set_text_scroll_immediate(new_scroll);
                            if !self.draw_commands.is_empty() {
                                state.gpu_renderer.update_draw_commands(&self.draw_commands);
                            }
                            state.window.request_redraw();
                        }
                    }
                    self.last_cursor = (x, y);
                    return;
                }

                if let Some(state_ref) = &mut self.state {
                    let vw = state_ref.window.inner_size().width as f32;
                    let vh = state_ref.window.inner_size().height as f32;
                    let content_h = state_ref.gpu_renderer.content_height();
                    let hovered = self.scroll_bar.hit_test_thumb(vw, vh, content_h, state_ref.gpu_renderer.text_scroll(), x, y);
                    if hovered != state_ref.gpu_renderer.scrollbar_hover() {
                        state_ref.gpu_renderer.set_scrollbar_hover(hovered);
                        // requeue vertices so color change is visible
                        if !self.draw_commands.is_empty() {
                            state_ref.gpu_renderer.update_draw_commands(&self.draw_commands);
                        }
                        state_ref.window.request_redraw();
                    }
                }

                self.last_cursor = (x, y);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                 if button == MouseButton::Left {
                     match state {
                         ElementState::Pressed => {
                            let vw = self.state.as_ref().unwrap().window.inner_size().width as f32;
                            let vh = self.state.as_ref().unwrap().window.inner_size().height as f32;
                            let content_h = self.state.as_ref().unwrap().gpu_renderer.content_height();
                            let (px, py) = self.last_cursor;
                            if self.scroll_bar.hit_test_thumb(vw, vh, content_h, self.state.as_ref().unwrap().gpu_renderer.text_scroll(), px, py) {
                                self.dragging_scrollbar = true;
                                self.drag_start_y = py;
                                self.drag_start_scroll = self.state.as_ref().unwrap().gpu_renderer.text_scroll();
                            }
                         }
                         ElementState::Released => {
                             self.dragging_scrollbar = false;
                         }
                     }
                 }
             }
            _ => {}
        }
    }
}
