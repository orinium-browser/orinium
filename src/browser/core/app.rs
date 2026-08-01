//! Browser core: application entry and lifecycle manager.
//!
//! Responsibilities:
//! - Manage Browser lifetime, window, and tab collection.
//! - Coordinate network/resource loading and map responses to tabs.
//! - Drive the engine pipeline: schedule layout, collect draw commands, and hand them to the platform renderer.
//! - Handle input and window events and dispatch them to tabs/UI.
//!
//! Processing flow (high-level):
//! 1. Initialize platform components (system window, GPU renderer, network core).
//! 2. Create and register `Tab` instances and navigate to initial URLs.
//! 3. Enter event loop: handle events -> update state -> request layout -> generate draw commands -> render.
//! 4. Manage asynchronous fetches and inject resources into the engine when they arrive.
//!
//! Example (for contributors / local testing):
//! ```no_run
//! use orinium_browser::browser::{BrowserApp, BrowserUi, Tab};
//!
//! let mut tab = Tab::new();
//! tab.navigate("resource:///test/test.html".parse().unwrap());
//! let mut app = BrowserApp::default();
//! app.set_default_ui(BrowserUi::with_tab(tab));
//! app.run().unwrap();
//! ```
//!
//! Developer notes:
//! - For parsing and layout details see `engine::html`, `engine::css`, and `engine::layouter`.
//! - For platform integration see `platform::{network, renderer, system}`.
//! - Keep public API small and document invariants for Tab lifecycle and fetch handling.

use anyhow::Result;
use std::collections::{HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, io};
use url::Url;
use winit::event::WindowEvent;
use winit::window::WindowId;

use super::tab::{FetchKind, Tab, TabTask};
use super::ui::BrowserUi;
use super::{BrowserCommand, resource_loader::BrowserResourceLoader};
use crate::engine::layouter;
use crate::engine::renderer_model::{self, DrawCommand};

use crate::platform::network::NetworkCore;
use crate::platform::renderer::gpu::GpuRenderer;
use crate::platform::system::App;

pub struct RenderState {
    /// List of draw commands generated from the layout engine.
    pub draw_commands: Vec<DrawCommand>,
    /// Current window size in pixels (width, height).
    pub window_size: (u32, u32),
    /// Current scale factor (for HiDPI displays).
    pub scale_factor: f64,
    /// Current window title.
    pub window_title: String,
}

/// Stores input-related state for a single browser window.
#[derive(Default)]
pub struct InputState {
    /// Current mouse position in window coordinates.
    pub mouse_position: (f64, f64),
    /// Current keyboard modifier state (Ctrl, Shift, Alt, etc.).
    pub modifiers: winit::keyboard::ModifiersState,
}

pub struct PendingFetches {
    /// Maps (id) to (window_id, tab_id, FetchKind, Url)
    /// Id is used to track pending fetch requests.
    map: HashMap<usize, (WindowId, usize, FetchKind, Url)>,
    counter: usize,
}

impl PendingFetches {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            counter: 0,
        }
    }

    /// URLとFetchKindを受け取り、一意IDを生成して登録
    pub fn insert(
        &mut self,
        window_id: WindowId,
        tab_id: usize,
        kind: FetchKind,
        url: Url,
    ) -> usize {
        self.counter += 1;

        let id = self.generate_id(&url);

        self.map.insert(id, (window_id, tab_id, kind, url));
        dbg!(id)
    }

    fn generate_id(&self, url: &Url) -> usize {
        // URLをハッシュ化
        let mut hasher = DefaultHasher::new();
        url.hash(&mut hasher);
        let url_hash = hasher.finish() as usize;

        // 現在時刻ナノ秒
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_nanos() as usize;

        // ナノ秒 XOR カウンタ XOR URLハッシュ
        now ^ self.counter ^ url_hash
    }

    pub fn remove(&mut self, id: usize) -> Option<(WindowId, usize, FetchKind, Url)> {
        self.map.remove(&id)
    }
}

/// Main browser application struct.
///
/// Responsibilities:
/// - Manage collection of `Tab` instances and the active tab index.
/// - Coordinate resource loading and pending fetch lifecycle.
/// - Orchestrate engine work (layout/draw-command generation) and submit commands to the renderer.
/// - Process input and window events and propagate them to tabs/UI.
///
/// Typical lifecycle:
/// 1. Construct `BrowserApp::new(...)`, which wires platform components (network, renderer, system).
/// 2. Create `Tab` objects, wrap them in a `BrowserUi`, and call `set_default_ui`.
/// 3. Call `run()` to start the event loop. Each loop iteration:
///    - Poll platform events (keyboard/mouse/window).
///    - Update input state and dispatch to the active tab.
///    - If DOM/CSS changes occurred, request layout and regenerate draw commands.
///    - Submit draw commands to the platform-specific renderer.
/// 4. Manage asynchronous resource fetches: match responses to pending fetch IDs and notify tabs.
///
/// Example usage:
/// ```no_run
/// use orinium_browser::browser::{BrowserApp, BrowserUi, Tab};
///
/// let mut tab = Tab::new();
/// tab.navigate("resource:///test/test.html".parse().unwrap());
/// let mut app = BrowserApp::default();
/// app.set_default_ui(BrowserUi::with_tab(tab));
/// app.run().unwrap();
/// ```
///
/// Contributor guidance:
/// - Add small unit tests to validate tab lifecycle, fetch handling, and draw-command generation.
/// - Prefer adding examples under `examples/` to demonstrate end-to-end behavior.
pub struct BrowserApp {
    /// Per-window render state, keyed by WindowId.
    renders: HashMap<WindowId, RenderState>,
    /// Per-window input state, keyed by WindowId.
    inputs: HashMap<WindowId, InputState>,
    /// Maps each window to its UI (which holds its tabs and active tab).
    windows: HashMap<WindowId, BrowserUi>,
    /// Default window size used when opening a new window.
    default_window_size: (u32, u32),
    /// Default window title used when opening a new window.
    default_window_title: String,
    network: BrowserResourceLoader,
    pending_fetches: PendingFetches,
    /// UI used when the first window opens (if set before `run()`).
    default_ui: Option<BrowserUi>,
}

impl Default for BrowserApp {
    fn default() -> Self {
        Self::new((800, 600), "Orinium Browser".to_string()).unwrap()
    }
}

impl BrowserApp {
    /// Starts the main browser event loop asynchronously.
    /// Returns an error if no default UI was set via `set_default_ui`.
    pub fn run(self) -> Result<()> {
        if self.default_ui.is_none() {
            anyhow::bail!("set_default_ui must be called before run()");
        }
        run_with_winit_backend(self)
    }

    /// Creates a new browser instance with the given default window size and title.
    /// Windows are registered later via `open_window`.
    pub fn new(
        default_window_size: (u32, u32),
        default_window_title: String,
    ) -> Result<Self, io::Error> {
        let network = BrowserResourceLoader::new(Some(Rc::new(NetworkCore::new()?)));

        Ok(Self {
            renders: HashMap::new(),
            inputs: HashMap::new(),
            windows: HashMap::new(),
            default_window_size,
            default_window_title,
            network,
            pending_fetches: PendingFetches::new(),
            default_ui: None,
        })
    }

    /// Registers a new window with the given id, size, title, scale factor, and associated tab.
    pub fn open_window(
        &mut self,
        window_id: WindowId,
        window_size: (u32, u32),
        window_title: String,
        scale_factor: f64,
        root_ui: BrowserUi,
    ) {
        self.renders.insert(
            window_id,
            RenderState {
                draw_commands: vec![],
                window_size,
                scale_factor,
                window_title,
            },
        );
        self.inputs.insert(window_id, InputState::default());
        self.windows.insert(window_id, root_ui);
    }

    /// Removes a window's state when the window is closed.
    pub fn close_window(&mut self, window_id: WindowId) {
        self.renders.remove(&window_id);
        self.inputs.remove(&window_id);
        self.windows.remove(&window_id);
    }

    /// Returns the default window size for opening new windows.
    pub fn default_window_size(&self) -> (f32, f32) {
        (
            self.default_window_size.0 as f32,
            self.default_window_size.1 as f32,
        )
    }

    /// Returns the default window title for opening new windows.
    pub fn default_window_title(&self) -> String {
        self.default_window_title.clone()
    }

    pub fn tick(&mut self, window_id: WindowId) -> BrowserCommand {
        self.handle_network_messages();

        // tick all tabs of the given window and collect redraw requests
        let mut needs_redraw = false;
        let Some(ui) = self.windows.get_mut(&window_id) else {
            return BrowserCommand::None;
        };
        let tab_count = ui.tabs.len();
        for tab_id in 0..tab_count {
            let Some(tab) = ui.tabs.get_mut(tab_id) else {
                continue;
            };
            for task in tab.tick() {
                match task {
                    TabTask::Fetch { url, kind } => {
                        log::info!("Fetch requested in App: url={}", url);
                        let id = self
                            .pending_fetches
                            .insert(window_id, tab_id, kind, url.clone());
                        self.network.fetch_async(url, id);
                    }
                    TabTask::NeedsRedraw => {
                        needs_redraw = true;
                    }
                }
            }
        }

        if needs_redraw {
            BrowserCommand::RequestRedraw
        } else {
            BrowserCommand::None
        }
    }

    fn handle_network_messages(&mut self) {
        let messages = self.network.try_receive();

        for msg in messages {
            log::info!("Network message received in App for fetch_id={}", msg.id);

            // pending_fetches から fetch 情報を取得
            let Some((window_id, tab_id, kind, url)) = self.pending_fetches.remove(msg.id) else {
                log::warn!("No pending fetch found for fetch_id={}", msg.id);
                continue;
            };

            // Tab を取得（該当ウィンドウの UI から）
            let Some(ui) = self.windows.get_mut(&window_id) else {
                log::warn!("There is no window called id={:?}", window_id);
                continue;
            };
            let Some(tab) = ui.tabs.get_mut(tab_id) else {
                log::warn!("There is no Tab called id={}", tab_id);
                continue;
            };

            match msg.response {
                Ok(resp) => {
                    log::info!("Fetch Done in App for tab_id={}", tab_id);

                    match kind {
                        FetchKind::Html => {
                            let html = String::from_utf8_lossy(&resp.body).to_string();
                            tab.on_fetch_succeeded_html(html);
                        }
                        FetchKind::Css => {
                            let css = String::from_utf8_lossy(&resp.body).to_string();
                            tab.on_fetch_succeeded_css(css);
                        }
                        FetchKind::Image { source } => {
                            tab.on_fetch_succeeded_image(source, &resp.body);
                        }
                    }
                }
                Err(err) => {
                    log::error!("NetworkError: {}", err);
                    if matches!(kind, FetchKind::Image { .. }) {
                        log::warn!("Image fetch failed without aborting page load: {}", url);
                    } else {
                        tab.on_fetch_failed(err, url);
                    }
                }
            }
        }
    }

    #[allow(dead_code)]
    /// Returns a mutable reference to the currently active tab of the given window, if any.
    fn active_tab_mut(&mut self, window_id: WindowId) -> Option<&mut Tab> {
        let ui = self.windows.get_mut(&window_id)?;
        ui.tabs.get_mut(ui.active_tab)
    }

    /// Returns the tab index associated with the given window (falls back to `0`).
    fn tab_id_for_window(&self, window_id: WindowId) -> usize {
        self.windows
            .get(&window_id)
            .map(|ui| ui.active_tab)
            .unwrap_or(0)
    }

    /// Rebuilds the render tree for the window's assigned tab and generates draw commands.
    fn rebuild_render_tree(&mut self, window_id: WindowId) {
        let tab_id = self.tab_id_for_window(window_id);

        let (viewport, mut draw_commands) = {
            let Some(render) = self.renders.get_mut(&window_id) else {
                return;
            };

            let sf = render.scale_factor as f32;

            let viewport = (
                render.window_size.0 as f32 / sf,
                render.window_size.1 as f32 / sf,
            );

            // Reuse allocation
            let mut draw_commands = std::mem::take(&mut render.draw_commands);
            draw_commands.clear();

            (viewport, draw_commands)
        };

        let title = {
            let Some(ui) = self.windows.get_mut(&window_id) else {
                return;
            };
            let Some(tab) = ui.tabs.get_mut(tab_id) else {
                return;
            };

            tab.relayout(viewport);

            let Some((layout, info)) = tab.layout_and_info() else {
                log::debug!("No layout/info available for tab {}", tab_id);
                return;
            };

            renderer_model::generate_draw_commands(&mut draw_commands, layout, info);

            tab.title()
        };

        let Some(render) = self.renders.get_mut(&window_id) else {
            return;
        };

        // Return reused buffer
        render.draw_commands = draw_commands;

        if let Some(title) = title {
            render.window_title = title;
        }
    }

    /// Handles a `winit` window event for the given window and returns a `BrowserCommand`.
    pub fn handle_window_event(
        &mut self,
        window_id: WindowId,
        event: WindowEvent,
        gpu: &mut GpuRenderer,
    ) -> BrowserCommand {
        let browser_cmd = match event {
            WindowEvent::CloseRequested => BrowserCommand::Exit,

            WindowEvent::RedrawRequested => {
                self.redraw(window_id, gpu);
                BrowserCommand::RenameWindowTitle
            }

            WindowEvent::Resized(size) => {
                if let Some(render) = self.renders.get_mut(&window_id) {
                    render.window_size = (size.width, size.height);
                }
                gpu.resize(size);
                self.redraw(window_id, gpu);
                BrowserCommand::RequestRedraw
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                gpu.set_scale_factor(scale_factor);
                if let Some(render) = self.renders.get_mut(&window_id) {
                    render.scale_factor = scale_factor;
                }
                self.redraw(window_id, gpu);
                BrowserCommand::RequestRedraw
            }

            WindowEvent::MouseWheel { delta, .. } => {
                self.handle_scroll(window_id, delta);
                BrowserCommand::RequestRedraw
            }

            WindowEvent::CursorMoved { position, .. } => {
                if let Some(input) = self.inputs.get_mut(&window_id) {
                    input.mouse_position = (position.x, position.y);
                }
                BrowserCommand::None
            }

            WindowEvent::MouseInput { button, .. } => self.handle_mouse_input(window_id, button),

            WindowEvent::ModifiersChanged(modifiers) => {
                if let Some(input) = self.inputs.get_mut(&window_id) {
                    input.modifiers = modifiers.state();
                }
                BrowserCommand::None
            }

            WindowEvent::KeyboardInput { event, .. } => {
                self.handle_keyboard_input(window_id, event)
            }

            _ => BrowserCommand::None,
        };
        let cmd_from_tick = self.tick(window_id);
        match browser_cmd {
            BrowserCommand::None => {
                if matches!(cmd_from_tick, BrowserCommand::RequestRedraw) {
                    self.redraw(window_id, gpu);
                }
                cmd_from_tick
            }
            BrowserCommand::RenameWindowTitle => {
                if matches!(cmd_from_tick, BrowserCommand::RequestRedraw) {
                    // tick() が追加の処理を要求 → RequestRedraw に昇格させる。
                    // RequestRedraw のハンドラはタイトル設定も行うので情報は失われない。
                    self.redraw(window_id, gpu);
                    BrowserCommand::RequestRedraw
                } else {
                    browser_cmd
                }
            }
            _ => {
                if matches!(cmd_from_tick, BrowserCommand::RequestRedraw) {
                    self.redraw(window_id, gpu);
                }
                browser_cmd
            }
        }
    }

    /// Handles keyboard input events and returns a `BrowserCommand`.
    fn handle_keyboard_input(
        &mut self,
        window_id: WindowId,
        event: winit::event::KeyEvent,
    ) -> BrowserCommand {
        // TODO: あとで消す
        const KEY_NEW_WINDOW: &str = "n";

        if event.state != winit::event::ElementState::Pressed {
            return BrowserCommand::None;
        }

        let ctrl = self
            .inputs
            .get(&window_id)
            .is_some_and(|i| i.modifiers.control_key());

        if ctrl
            && let winit::keyboard::Key::Character(ch) = &event.logical_key
            && ch.as_str().eq_ignore_ascii_case(KEY_NEW_WINDOW)
        {
            return BrowserCommand::OpenNewWindow;
        }

        BrowserCommand::None
    }

    /// Handles mouse input events, mainly left-clicks for the active tab.
    fn handle_mouse_input(
        &mut self,
        window_id: WindowId,
        button: winit::event::MouseButton,
    ) -> BrowserCommand {
        if button != winit::event::MouseButton::Left {
            return BrowserCommand::None;
        }

        let (x, y, sf) = match (self.inputs.get(&window_id), self.renders.get(&window_id)) {
            (Some(input), Some(render)) => (
                input.mouse_position.0,
                input.mouse_position.1,
                render.scale_factor,
            ),
            _ => return BrowserCommand::None,
        };

        let tab_id = self.tab_id_for_window(window_id);
        let Some(ui) = self.windows.get_mut(&window_id) else {
            return BrowserCommand::None;
        };
        if let Some(tab) = ui.tabs.get_mut(tab_id) {
            Self::handle_mouse_click(tab, (x / sf) as f32, (y / sf) as f32);
            BrowserCommand::RequestRedraw
        } else {
            BrowserCommand::None
        }
    }

    /// Handles scrolling for the window's assigned tab, updating its layout container offsets.
    fn handle_scroll(&mut self, window_id: WindowId, delta: winit::event::MouseScrollDelta) {
        let scroll_amount = match delta {
            winit::event::MouseScrollDelta::LineDelta(_, y) => -y * 60.0,
            winit::event::MouseScrollDelta::PixelDelta(pos) => -pos.y as f32,
        };

        let (window_height, sf) = match self.renders.get(&window_id) {
            Some(render) => (render.window_size.1 as f32, render.scale_factor as f32),
            None => return,
        };

        let tab_id = self.tab_id_for_window(window_id);
        let Some(ui) = self.windows.get_mut(&window_id) else {
            return;
        };
        if let Some(tab) = ui.tabs.get_mut(tab_id)
            && let Some((layout, info)) = tab.layout_and_info_mut()
            && let layouter::types::NodeKind::Container {
                scroll_offset_y, ..
            } = &mut info.kind
        {
            *scroll_offset_y = (*scroll_offset_y + scroll_amount).clamp(
                0.0,
                (layout
                    .layout_box
                    .iter()
                    .map(|l| l.children_box.height)
                    .sum::<f32>()
                    - (window_height / sf))
                    .max(0.0),
            );
        }
    }

    /// Handles a mouse click in the given tab at the specified coordinates.
    pub fn handle_mouse_click(tab: &mut Tab, x: f32, y: f32) {
        let hit_path = match tab.layout_and_info() {
            Some((layout, info)) => crate::engine::input::hit_test(layout, info, x, y),
            None => return,
        };

        let href_opt = {
            if let Some(hit) = hit_path.iter().find(|e| {
                matches!(
                    e.info.kind,
                    layouter::types::NodeKind::Container { ref role, .. }
                        if matches!(role, layouter::types::ContainerRole::Link { .. })
                )
            }) {
                if let layouter::types::NodeKind::Container { role, .. } = &hit.info.kind
                    && let layouter::types::ContainerRole::Link { href } = role
                {
                    Some(href.clone())
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(href) = href_opt {
            tab.move_to(&href)
        }
    }

    /// Rebuilds the render tree and sends draw commands to the GPU for the given window.
    pub fn redraw(&mut self, window_id: WindowId, gpu: &mut GpuRenderer) {
        self.rebuild_render_tree(window_id);
        self.apply_draw_commands(window_id, gpu);
        if let Err(e) = gpu.render() {
            log::error!(target: "BrowserApp::redraw", "Render error occurred: {}", e);
        }
    }

    /// Applies the current draw commands for the given window to the GPU renderer.
    pub fn apply_draw_commands(&self, window_id: WindowId, gpu: &mut GpuRenderer) {
        if let Some(render) = self.renders.get(&window_id) {
            gpu.parse_draw_commands(&render.draw_commands);
        }
    }

    /// Sets the UI to use when the first window opens.
    /// Must be called before `run()`.
    pub fn set_default_ui(&mut self, ui: BrowserUi) {
        self.default_ui = Some(ui);
    }

    /// Takes the default UI, or returns `None` if not set.
    pub fn take_default_ui(&mut self) -> Option<BrowserUi> {
        self.default_ui.take()
    }

    /// Returns the current window size for the given window as `(width, height)` in floating-point pixels.
    pub fn window_size(&self, window_id: WindowId) -> (f32, f32) {
        match self.renders.get(&window_id) {
            Some(render) => (render.window_size.0 as f32, render.window_size.1 as f32),
            None => (
                self.default_window_size.0 as f32,
                self.default_window_size.1 as f32,
            ),
        }
    }

    /// Returns the window title for the given window.
    pub fn window_title(&self, window_id: WindowId) -> String {
        match self.renders.get(&window_id) {
            Some(render) => render.window_title.clone(),
            None => self.default_window_title.clone(),
        }
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
    let event_loop = winit::event_loop::EventLoop::new()?;
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    let mut app = App::new(app);
    event_loop.run_app(&mut app)?;
    Ok(())
}

fn configure_winit_backend_forced_x11() {
    let current = env::var("WINIT_UNIX_BACKEND").ok();
    let should_force_x11 = !matches!(current.as_deref(), Some("x11"));

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
    let should_force_x11 = !matches!(current.as_deref(), Some("x11"));

    if should_force_x11 {
        unsafe {
            env::set_var("WINIT_UNIX_BACKEND", "x11");
            env::remove_var("WAYLAND_DISPLAY");
        }
        log::info!("WSLg detected: defaulting to X11 backend for stability");
    }
}
