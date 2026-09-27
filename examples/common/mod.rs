//! Shared helpers for the shape-drawing examples.
//!
//! These examples drive winit directly and build [`DrawCommand`]s by hand,
//! without the browser. This module provides the common window-processing
//! methods so each example only supplies its own command list.

use anyhow::Result;
use orinium_browser::engine::renderer_model::DrawCommand;
use orinium_browser::platform::renderer::gpu::GpuRenderer;
use std::sync::Arc;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

/// What to do after handling a window event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventOutcome {
    Continue,
    Close,
}

/// A winit window paired with a [`GpuRenderer`].
///
/// Provides the common window-processing methods used by the shape-drawing
/// examples: create the window and renderer, and drive the event loop through
/// [`ShapeWindow::handle_event`].
pub struct ShapeWindow {
    window: Arc<Window>,
    gpu: GpuRenderer,
}

impl ShapeWindow {
    /// Creates a window with the given title and inner size, and initializes
    /// the GPU renderer against it.
    pub fn new(event_loop: &ActiveEventLoop, title: &str, width: u32, height: u32) -> Result<Self> {
        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title(title)
                    .with_inner_size(winit::dpi::PhysicalSize::new(width, height)),
            )?,
        );
        let gpu = pollster::block_on(GpuRenderer::new(window.clone(), None))?;
        Ok(Self { window, gpu })
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// Parses the given commands into the mesh and renders one frame.
    pub fn draw(&mut self, commands: &[DrawCommand]) -> Result<()> {
        self.gpu.parse_draw_commands(commands);
        self.gpu.render()
    }

    /// Processes a single window event, (re)drawing `commands` on redraw.
    /// Returns [`EventOutcome::Close`] when the window asks to close.
    pub fn handle_event(&mut self, event: WindowEvent, commands: &[DrawCommand]) -> EventOutcome {
        match event {
            WindowEvent::CloseRequested => EventOutcome::Close,
            WindowEvent::Resized(size) => {
                self.gpu.resize(size);
                EventOutcome::Continue
            }
            WindowEvent::RedrawRequested => {
                if let Err(err) = self.draw(commands) {
                    log::error!("redraw failed: {err:#}");
                }
                EventOutcome::Continue
            }
            _ => EventOutcome::Continue,
        }
    }
}
