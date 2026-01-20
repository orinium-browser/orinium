use anyhow::Result;
use std::env;
use std::sync::Arc;
use winit::event::WindowEvent;

use super::tab::Tab;
// use super::ui::init_browser_ui;
use super::{BrowserCommand, resource_loader::BrowserResourceLoader};
use crate::engine::layouter::{self, DrawCommand};
use crate::platform::network::NetworkCore;
use crate::platform::renderer::gpu::GpuRenderer;
use crate::system::App;

/// Stores rendering-related state for the browser window.
pub struct RenderState {
    /// List of draw commands generated from the layout engine.
    pub draw_commands: Vec<DrawCommand>,
    /// Current window size in pixels (width, height).
    pub window_size: (u32, u32),
    /// Current scale factor (for HiDPI displays).
    pub scale_factor: f64,
}

/// Stores input-related state for the browser window.
#[derive(Default)]
pub struct InputState {
    /// Current mouse position in window coordinates.
    pub mouse_position: (f64, f64),
}

/// Main browser application struct.
/// Holds tabs, rendering state, input state, and network resources.
pub struct BrowserApp {
    tabs: Vec<Tab>,
    active_tab: usize,
    render: RenderState,
    window_title: String,
    input: InputState,
    network: Arc<BrowserResourceLoader>,
}

impl Default for BrowserApp {
    fn default() -> Self {
        Self::new((800, 600), "Orinium Browser".to_string())
    }
}

impl BrowserApp {
    /// Starts the main browser event loop asynchronously.
    pub fn run(self) -> Result<()> {
        run_with_winit_backend(self)
    }

    /// Creates a new browser instance with the given window size and title.
    pub fn new(window_size: (u32, u32), window_title: String) -> Self {
        let network = Arc::new(BrowserResourceLoader::new(Some(Arc::new(
            NetworkCore::new(),
        ))));

        Self {
            tabs: vec![],
            active_tab: 0,
            render: RenderState {
                draw_commands: vec![],
                window_size,
                scale_factor: 1.0,
            },
            window_title,
            input: InputState::default(),
            network,
        }
    }

    /// Returns a mutable reference to the currently active tab, if any.
    fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active_tab)
    }

    /// Rebuilds the render tree for the active tab and generates draw commands.
    fn rebuild_render_tree(&mut self) {
        let size = self.window_size();
        let sf = self.render.scale_factor as f32;

        let (layout, info, title) = match self.active_tab_mut() {
            Some(tab) => {
                let title = tab
                    .title()
                    .filter(|t| !t.is_empty())
                    .or_else(|| tab.url().filter(|u| !u.is_empty()));

                if let Some((layout, info)) = tab.layout_and_info() {
                    (layout, info, title)
                } else {
                    return;
                }
            }
            None => return,
        };

        ui_layout::LayoutEngine::layout(layout, size.0 / sf, size.1 / sf);
        self.render.draw_commands = layouter::generate_draw_commands(layout, info);

        if let Some(t) = title {
            self.window_title = t;
        }
    }

    /// Handles a `winit` window event and returns a `BrowserCommand`.
    pub fn handle_window_event(
        &mut self,
        event: WindowEvent,
        gpu: &mut GpuRenderer,
    ) -> BrowserCommand {
        match event {
            WindowEvent::CloseRequested => BrowserCommand::Exit,

            WindowEvent::RedrawRequested => {
                self.redraw(gpu);
                BrowserCommand::RenameWindowTitle
            }

            WindowEvent::Resized(size) => {
                self.render.window_size = (size.width, size.height);
                gpu.resize(size);
                self.redraw(gpu);
                BrowserCommand::RequestRedraw
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                gpu.set_scale_factor(scale_factor);
                self.render.scale_factor = scale_factor;
                self.redraw(gpu);
                BrowserCommand::RequestRedraw
            }

            WindowEvent::MouseWheel { delta, .. } => {
                self.handle_scroll(delta);
                BrowserCommand::RequestRedraw
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.input.mouse_position = (position.x, position.y);
                BrowserCommand::None
            }

            WindowEvent::MouseInput { button, .. } => self.handle_mouse_input(button),

            _ => BrowserCommand::None,
        }
    }

    /// Handles mouse input events, mainly left-clicks for the active tab.
    fn handle_mouse_input(&mut self, button: winit::event::MouseButton) -> BrowserCommand {
        if button != winit::event::MouseButton::Left {
            return BrowserCommand::None;
        }

        let (x, y) = self.input.mouse_position;
        let sf = self.render.scale_factor;
        if let Some(tab) = self.active_tab_mut() {
            Self::handle_mouse_click(tab, (x / sf) as f32, (y / sf) as f32);
            BrowserCommand::RequestRedraw
        } else {
            BrowserCommand::None
        }
    }

    /// Handles scrolling for the active tab, updating its layout container offsets.
    ///
    /// Currently a stub.
    fn handle_scroll(&mut self, delta: winit::event::MouseScrollDelta) {
        let scroll_amount = match delta {
            winit::event::MouseScrollDelta::LineDelta(_, y) => -y * 60.0,
            winit::event::MouseScrollDelta::PixelDelta(pos) => -pos.y as f32,
        };

        if let Some(tab) = self.active_tab_mut() {
            if let Some((_layout, info)) = tab.layout_and_info() {
                if let layouter::NodeKind::Container {
                    scroll_offset_y, ..
                } = &mut info.kind
                {
                    *scroll_offset_y = (*scroll_offset_y + scroll_amount).clamp(0.0, f32::MAX);
                }
            }
        }
    }

    /// Handles a mouse click in the given tab at the specified coordinates.
    pub fn handle_mouse_click(tab: &mut Tab, x: f32, y: f32) {
        println!("clicked");
        let hit_path = match tab.layout_and_info() {
            Some((layout, info)) => crate::engine::input::hit_test(layout, info, x, y),
            None => return,
        };

        let href_opt = {
            if let Some(hit) = hit_path
                .iter()
                .find(|e| matches!(e.info.kind, layouter::NodeKind::Link { .. }))
            {
                if let layouter::NodeKind::Link { href, .. } = &hit.info.kind {
                    println!("Link clicked: {}", href);
                    Some(href.clone())
                } else {
                    None
                }
            } else {
                None
            }
        };

        match href_opt {
            Some(href) => {
                let _ = tab.move_to(&href);
            }
            None => return,
        }
    }

    /// Rebuilds the render tree and sends draw commands to the GPU.
    pub fn redraw(&mut self, gpu: &mut GpuRenderer) {
        self.rebuild_render_tree();
        self.apply_draw_commands(gpu);
        if let Err(e) = gpu.render() {
            log::error!("Render error occurred: {}", e);
        }
    }

    /// Applies the current draw commands to the GPU renderer.
    pub fn apply_draw_commands(&self, gpu: &mut GpuRenderer) {
        gpu.parse_draw_commands(&self.render.draw_commands);
    }

    /// Adds a new tab to the browser.
    pub fn add_tab(&mut self, tab: Tab) {
        self.tabs.push(tab);
    }

    /// Returns the current window size as `(width, height)` in floating-point pixels.
    pub fn window_size(&self) -> (f32, f32) {
        (
            self.render.window_size.0 as f32,
            self.render.window_size.1 as f32,
        )
    }

    /// Returns the current window title.
    pub fn window_title(&self) -> String {
        self.window_title.clone()
    }

    /// Returns a clone of the browser's network resource loader.
    pub fn network(&self) -> Arc<BrowserResourceLoader> {
        Arc::clone(&self.network)
    }

    /// Sets the current scale factor for rendering.
    pub fn set_scale_factor(&mut self, sf: f64) {
        self.render.scale_factor = sf;
    }
}

fn run_with_winit_backend(app: BrowserApp) -> Result<()> {
    configure_winit_backend_for_wslg();
    if env::var_os("ORINIUM_FORCE_X11").is_some() {
        configure_winit_backend_forced_x11();
    }

    run_event_loop(app)
}

fn run_event_loop(app: BrowserApp) -> Result<()> {
    let event_loop =
        winit::event_loop::EventLoop::<crate::platform::system::State>::with_user_event()
            .build()?;
    let mut app = App::new(app);
    event_loop.run_app(&mut app)?;
    Ok(())
}

fn configure_winit_backend_forced_x11() {
    let current = env::var("WINIT_UNIX_BACKEND").ok();
    let should_force_x11 = match current.as_deref() {
        Some("x11") => false,
        _ => true,
    };

    if should_force_x11 {
        unsafe {
            env::set_var("WINIT_UNIX_BACKEND", "x11");
            env::remove_var("WAYLAND_DISPLAY");
        }
        log::info!("Forcing X11 (WINIT_UNIX_BACKEND=x11, WAYLAND_DISPLAY cleared)");
    }
}

fn configure_winit_backend_for_wslg() {
    let is_wsl = env::var_os("WSL_DISTRO_NAME").is_some() || env::var_os("WSL_INTEROP").is_some();
    if !is_wsl {
        return;
    }

    // On WSLg, Wayland is often unstable; default to X11 unless explicitly requested.
    if env::var_os("ORINIUM_PREFER_WAYLAND").is_some() {
        return;
    }

    let current = env::var("WINIT_UNIX_BACKEND").ok();
    let should_force_x11 = match current.as_deref() {
        Some("x11") => false,
        _ => true,
    };

    if should_force_x11 {
        unsafe {
            env::set_var("WINIT_UNIX_BACKEND", "x11");
            env::remove_var("WAYLAND_DISPLAY");
        }
        log::info!("WSLg detected: defaulting to X11 backend for stability");
    }
}
