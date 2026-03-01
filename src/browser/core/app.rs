use anyhow::Result;
use std::collections::{HashMap, hash_map::DefaultHasher};
use std::env;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;
use winit::event::WindowEvent;

use super::tab::{FetchKind, Tab, TabTask};
// use super::ui::init_browser_ui;
use super::{BrowserCommand, resource_loader::BrowserResourceLoader};
use crate::engine::layouter;
use crate::engine::renderer_model::{self, DrawCommand};
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

pub struct PendingFetches {
    /// Maps (id) to (tab_id, FetchKind)
    /// Id is used to track pending fetch requests.
    map: HashMap<usize, (usize, FetchKind, Url)>,
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
    pub fn insert(&mut self, tab_id: usize, kind: FetchKind, url: Url) -> usize {
        self.counter += 1;

        let id = self.generate_id(&url);

        self.map.insert(id, (tab_id, kind, url));
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

    pub fn remove(&mut self, id: usize) -> Option<(usize, FetchKind, Url)> {
        self.map.remove(&id)
    }
}

/// Main browser application struct.
/// Holds tabs, rendering state, input state, and network resources.
pub struct BrowserApp {
    tabs: Vec<Tab>,
    active_tab: usize,
    render: RenderState,
    window_title: String,
    input: InputState,
    network: BrowserResourceLoader,
    pending_fetches: PendingFetches,
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
        let network = BrowserResourceLoader::new(Some(Rc::new(NetworkCore::new())));

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
            pending_fetches: PendingFetches::new(),
        }
    }

    pub fn tick(&mut self) -> BrowserCommand {
        let tab_id = self.active_tab;

        self.handle_network_messages();

        let Some(tab) = self.tabs.get_mut(tab_id) else {
            return BrowserCommand::None;
        };

        for task in tab.tick() {
            match task {
                TabTask::Fetch { url, kind } => {
                    log::info!("Fetch requested in App: url={}", url);
                    let id = self.pending_fetches.insert(tab_id, kind, url.clone());
                    self.network.fetch_async(url, id);
                }
                TabTask::NeedsRedraw => {
                    return BrowserCommand::RequestRedraw;
                }
            }
        }

        BrowserCommand::None
    }

    fn handle_network_messages(&mut self) {
        let messages = self.network.try_receive();

        for msg in messages {
            log::info!("Network message received in App for fetch_id={}", msg.id);

            // pending_fetches から fetch 情報を取得
            let Some((tab_id, kind, url)) = self.pending_fetches.remove(msg.id) else {
                log::warn!("No pending fetch found for fetch_id={}", msg.id);
                continue;
            };

            // Tab を取得
            let Some(tab) = self.tabs.get_mut(tab_id) else {
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
                    }
                }
                Err(err) => {
                    log::error!("NetworkError: {}", err);
                    tab.on_fetch_failed(err, url);
                }
            }
        }
    }

    /// Returns a mutable reference to the currently active tab, if any.
    fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active_tab)
    }

    /// Rebuilds the render tree for the active tab and generates draw commands.
    fn rebuild_render_tree(&mut self) {
        let (title, draw_commands) = {
            let sf = self.render.scale_factor as f32;
            let viewport = (
                self.render.window_size.0 as f32 / sf,
                self.render.window_size.1 as f32 / sf,
            );

            let Some(tab) = self.active_tab_mut() else {
                return;
            };

            tab.relayout(viewport);

            let Some((layout, info)) = tab.layout_and_info() else {
                log::debug!("No layout/info available for active tab");
                return;
            };

            let title = tab.title();
            let draw_commands = renderer_model::generate_draw_commands(layout, info);

            (title, draw_commands)
        };

        self.render.draw_commands = draw_commands;

        if let Some(title) = title {
            self.window_title = title;
        }
    }

    /// Handles a `winit` window event and returns a `BrowserCommand`.
    pub fn handle_window_event(
        &mut self,
        event: WindowEvent,
        gpu: &mut GpuRenderer,
    ) -> BrowserCommand {
        let browser_cmd = match event {
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
        };
        let cmd_from_tick = self.tick();
        match browser_cmd {
            BrowserCommand::None => {
                if matches!(cmd_from_tick, BrowserCommand::RequestRedraw) {
                    self.redraw(gpu);
                }
                cmd_from_tick
            }
            _ => browser_cmd,
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

        let window_size = self.window_size();
        let sf = self.render.scale_factor as f32;

        if let Some(tab) = self.tabs.get_mut(self.active_tab)
            && let Some((layout, info)) = tab.layout_and_info_mut()
            && let layouter::types::NodeKind::Container {
                scroll_offset_y, ..
            } = &mut info.kind
        {
            *scroll_offset_y = (*scroll_offset_y + scroll_amount).clamp(
                0.0,
                (layout
                    .layout_boxes
                    .iter()
                    .map(|l| l.children_box.height)
                    .sum::<f32>()
                    - (window_size.1 / sf))
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

    /// Rebuilds the render tree and sends draw commands to the GPU.
    pub fn redraw(&mut self, gpu: &mut GpuRenderer) {
        self.rebuild_render_tree();
        self.apply_draw_commands(gpu);
        if let Err(e) = gpu.render() {
            log::error!(target: "BrowserApp::redraw", "Render error occurred: {}", e);
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
