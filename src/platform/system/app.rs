use std::collections::HashMap;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use crate::browser::{BrowserApp, BrowserCommand};
use crate::platform::renderer::gpu::GpuRenderer;

pub struct WindowState {
    pub window: Arc<Window>,
    pub gpu_renderer: GpuRenderer,
}

pub struct App {
    windows: HashMap<WindowId, WindowState>,
    browser_app: BrowserApp,
}

impl App {
    pub fn new(browser_app: BrowserApp) -> Self {
        Self {
            windows: HashMap::new(),
            browser_app,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let default_size = self.browser_app.default_window_size();
        let default_title = self.browser_app.default_window_title();
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_inner_size(winit::dpi::PhysicalSize::new(
                            default_size.0,
                            default_size.1,
                        ))
                        .with_title(&default_title),
                )
                .unwrap(),
        );
        let window_id = window.id();
        let scale_factor = window.scale_factor();
        let gpu_renderer = pollster::block_on(GpuRenderer::new(window.clone(), None)).unwrap();

        self.browser_app.open_window(
            window_id,
            (default_size.0 as u32, default_size.1 as u32),
            default_title,
            scale_factor,
            0,
        );

        let mut state = WindowState {
            window,
            gpu_renderer,
        };

        // 初回描画
        self.browser_app
            .apply_draw_commands(window_id, &mut state.gpu_renderer);
        state.window.request_redraw();

        self.windows.insert(window_id, state);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if !self.windows.contains_key(&window_id) {
            return;
        }

        let cmd = {
            let state = self.windows.get_mut(&window_id).unwrap();
            self.browser_app
                .handle_window_event(window_id, event, &mut state.gpu_renderer)
        };

        match cmd {
            BrowserCommand::Exit => {
                self.windows.remove(&window_id);
                self.browser_app.close_window(window_id);
                if self.windows.is_empty() {
                    event_loop.exit();
                }
            }
            BrowserCommand::RequestRedraw => {
                if let Some(state) = self.windows.get(&window_id) {
                    state.window.request_redraw();
                    state
                        .window
                        .set_title(&self.browser_app.window_title(window_id));
                }
            }
            BrowserCommand::RenameWindowTitle => {
                if let Some(state) = self.windows.get(&window_id) {
                    state
                        .window
                        .set_title(&self.browser_app.window_title(window_id));
                }
            }
            BrowserCommand::None => {}
        }
    }
}
