//! Manages the application lifecycle, including window creation and event handling.
//!
//! This module is the OS shell. It owns the `winit` event loop, the windows
//! and their [`GpuRenderer`]s, and knows nothing about the browser: all
//! browser interaction goes through the [`BrowserHost`] port declared in
//! [`super::shell`].

use std::collections::HashMap;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use crate::platform::renderer::gpu::GpuRenderer;
use crate::platform::system::shell::{BrowserHost, ShellCommand, WindowGeometry};

pub struct WindowState {
    pub window: Arc<Window>,
    pub gpu_renderer: GpuRenderer,
}

pub struct App {
    windows: HashMap<WindowId, WindowState>,
    host: Box<dyn BrowserHost>,
}

impl App {
    /// Wraps a browser host in the OS event loop.
    pub fn new(host: Box<dyn BrowserHost>) -> Self {
        Self {
            windows: HashMap::new(),
            host,
        }
    }

    /// Creates a window from `geometry` and registers it with the host.
    ///
    /// The size and scale factor the host asked for are only a starting
    /// point: once the window exists the *observed* values win, because the
    /// display scale factor is not known before that.
    fn create_window(&mut self, event_loop: &ActiveEventLoop, geometry: WindowGeometry) {
        let WindowGeometry { size, title, .. } = geometry;

        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_inner_size(winit::dpi::PhysicalSize::new(size.0, size.1))
                        .with_title(&title),
                )
                .unwrap(),
        );
        let window_id = window.id();
        let initial_size = window.inner_size();
        let scale_factor = window.scale_factor();
        let gpu_renderer = pollster::block_on(GpuRenderer::new(Arc::clone(&window), None)).unwrap();

        self.host.open_window(
            window_id,
            WindowGeometry {
                size: (initial_size.width, initial_size.height),
                title,
                scale_factor,
            },
        );

        let mut state = WindowState {
            window,
            gpu_renderer,
        };

        self.host
            .apply_draw_commands(window_id, &mut state.gpu_renderer);
        state.window.request_redraw();

        self.windows.insert(window_id, state);
    }

    /// The geometry the host wants new windows to have.
    fn default_geometry(&self) -> WindowGeometry {
        WindowGeometry {
            size: self.host.default_window_size(),
            title: self.host.default_window_title(),
            // Replaced with the observed value once the window exists.
            scale_factor: 1.0,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if !self.host.has_pending_default_window() {
            log::error!("BrowserApp has no default UI configured");
            return;
        }

        self.create_window(event_loop, self.default_geometry());
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
            self.host
                .handle_window_event(window_id, event, &mut state.gpu_renderer)
        };

        match cmd {
            ShellCommand::Exit => {
                self.windows.remove(&window_id);
                self.host.close_window(window_id);
                if self.windows.is_empty() {
                    event_loop.exit();
                }
            }
            ShellCommand::RequestRedraw => {
                if let Some(state) = self.windows.get(&window_id) {
                    state.window.request_redraw();
                    state.window.set_title(&self.host.window_title(window_id));
                }
            }
            ShellCommand::RenameWindowTitle => {
                if let Some(state) = self.windows.get(&window_id) {
                    state.window.set_title(&self.host.window_title(window_id));
                }
            }
            ShellCommand::OpenNewWindow => {
                let geometry = self.default_geometry();
                self.create_window(event_loop, geometry);
            }
            ShellCommand::SetImeAllowed { allowed, position } => {
                if let Some(state) = self.windows.get(&window_id) {
                    state.window.set_ime_allowed(allowed);
                    if allowed {
                        state.window.set_ime_cursor_area(
                            winit::dpi::PhysicalPosition::new(position.0, position.1 + 24.0),
                            winit::dpi::PhysicalSize::new(1.0, 1.0),
                        );
                    }
                    state.window.request_redraw();
                }
            }
            ShellCommand::None => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        let window_ids: Vec<WindowId> = self.windows.keys().copied().collect();
        for window_id in window_ids {
            if self.host.poll_window(window_id)
                && let Some(state) = self.windows.get(&window_id)
            {
                state.window.request_redraw();
            }
        }
    }
}
