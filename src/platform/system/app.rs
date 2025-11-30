use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use crate::browser::{BrowserApp, BrowserCommand};
use crate::platform::renderer::gpu::GpuRenderer;

pub struct State {
    pub window: Arc<Window>,
    pub gpu_renderer: GpuRenderer,
}

pub struct App {
    state: Option<State>,
    browser_app: BrowserApp,
}

impl App {
    pub fn new(browser_app: BrowserApp) -> Self {
        Self {
            state: None,
            browser_app,
        }
    }
}

impl ApplicationHandler<State> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // reqed = requested
        let reqed_window_size = self.browser_app.window_size();
        let reqed_window_title = self.browser_app.window_title();
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_inner_size(winit::dpi::PhysicalSize::new(
                            reqed_window_size.0,
                            reqed_window_size.1,
                        ))
                        .with_title(reqed_window_title),
                )
                .unwrap(),
        );
        let state = State {
            window: window.clone(),
            gpu_renderer: pollster::block_on(GpuRenderer::new(window.clone(), None)).unwrap(),
        };
        self.state = Some(state);

        // 初回描画
        if let Some(state) = &mut self.state {
            self.browser_app
                .apply_draw_commands(&mut state.gpu_renderer);
            state.window.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        if let Some(state) = &mut self.state {
            match self
                .browser_app
                .handle_window_event(event, &mut state.gpu_renderer)
            {
                BrowserCommand::Exit => event_loop.exit(),
                BrowserCommand::RequestRedraw => {
                    state.window.request_redraw();
                    state.window.set_title(&self.browser_app.window_title());
                }
                BrowserCommand::RenameWindowTitle => {
                    state.window.set_title(&self.browser_app.window_title())
                }
                BrowserCommand::None => {}
            }
        }
    }
}
