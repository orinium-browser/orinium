//! Browser core: application entry and lifecycle manager.
//!
//! Responsibilities:
//! - Manage the window collection and each window's [`BrowserUi`].
//! - Forward winit window events to the owning window's UI.
//! - Coordinate network/resource loading and route responses to the owning window.
//!
//! Tab state, input handling, and rendering are delegated to [`BrowserUi`] /
//! [`BrowserRenderer`]; this type stays a thin orchestrator.
//!
//! Processing flow (high-level):
//! 1. Initialize platform components (system window, GPU renderer, network core).
//! 2. Create and register `BrowserUi` instances and navigate to initial URLs.
//! 3. Enter event loop: forward events -> delegate to `BrowserUi` -> route fetches.
//!
//! Example (for contributors / local testing):
//! ```no_run
//! use orinium_browser::browser::{BrowserApp, BrowserUi, Tab};
//!
//! let mut tab = Tab::default();
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

use super::tab::FetchKind;
use super::ui::BrowserUi;
use super::{BrowserCommand, resource_loader::BrowserResourceLoader};
use crate::browser::core::ui::TabId;
use crate::platform::network::{NetworkCore, NetworkRequest};
use crate::platform::renderer::gpu::GpuRenderer;
use crate::platform::system::App;

pub struct PendingFetches {
    /// Maps (id) to (window_id, tab_id, FetchKind, Url)
    /// Id is used to track pending fetch requests.
    map: HashMap<usize, (WindowId, TabId, FetchKind, Url)>,
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
        tab_id: TabId,
        kind: FetchKind,
        url: Url,
    ) -> usize {
        self.counter += 1;

        let id = self.generate_id(&url);

        self.map.insert(id, (window_id, tab_id, kind, url));
        id
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

    pub fn remove(&mut self, id: usize) -> Option<(WindowId, TabId, FetchKind, Url)> {
        self.map.remove(&id)
    }
}

/// Main browser application struct.
///
/// Responsibilities:
/// - Manage the window collection and per-window [`BrowserUi`] instances.
/// - Forward winit window events to each window's UI.
/// - Coordinate resource loading and route fetched results to the owning window.
///
/// Tab state, input handling, and rendering are delegated to [`BrowserUi`] /
/// [`BrowserRenderer`].
///
/// Typical lifecycle:
/// 1. Construct `BrowserApp::new(...)`, which wires platform components (network, system).
/// 2. Create `Tab` objects, wrap them in a `BrowserUi`, and call `set_default_ui`.
/// 3. Call `run()` to start the event loop. Each loop iteration:
///    - Forward winit events to the owning window's `BrowserUi`.
///    - Tick the UI, forward fetch requests to the network, and route responses back.
///
/// Example usage:
/// ```no_run
/// use orinium_browser::browser::{BrowserApp, BrowserUi, Tab};
///
/// let mut tab = Tab::default();
/// tab.navigate("resource:///test/test.html".parse().unwrap());
/// let mut app = BrowserApp::default();
/// app.set_default_ui(BrowserUi::with_tab(tab));
/// app.run().unwrap();
/// ```
pub struct BrowserApp {
    /// Maps each window to its UI (tabs, input state, renderer).
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
        Self::new((1280, 800), "Orinium Browser".to_string()).unwrap()
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
            windows: HashMap::new(),
            default_window_size,
            default_window_title,
            network,
            pending_fetches: PendingFetches::new(),
            default_ui: None,
        })
    }

    /// Registers a new window with the given id, size, title, scale factor, and associated UI.
    pub fn open_window(
        &mut self,
        window_id: WindowId,
        window_size: (u32, u32),
        window_title: String,
        scale_factor: f64,
        mut root_ui: BrowserUi,
    ) {
        root_ui.set_window(window_size, scale_factor, window_title);
        self.windows.insert(window_id, root_ui);
    }

    /// Removes a window's state when the window is closed.
    pub fn close_window(&mut self, window_id: WindowId) {
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

    /// Sets the UI to use when the first window opens.
    /// Must be called before `run()`.
    pub fn set_default_ui(&mut self, ui: BrowserUi) {
        self.default_ui = Some(ui);
    }

    /// Takes the default UI, or returns `None` if not set.
    pub fn take_default_ui(&mut self) -> Option<BrowserUi> {
        self.default_ui.take()
    }

    /// Handles a `winit` window event for the given window and returns a `BrowserCommand`.
    pub fn handle_window_event(
        &mut self,
        window_id: WindowId,
        event: WindowEvent,
        gpu: &mut GpuRenderer,
    ) -> BrowserCommand {
        let browser_cmd = match self.windows.get_mut(&window_id) {
            Some(ui) => ui.handle_window_event(event, gpu),
            None => BrowserCommand::None,
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

    /// Rebuilds the render tree and sends draw commands to the GPU for the given window.
    pub fn redraw(&mut self, window_id: WindowId, gpu: &mut GpuRenderer) {
        let Some(ui) = self.windows.get_mut(&window_id) else {
            return;
        };
        ui.redraw(gpu);
    }

    /// Applies the current draw commands for the given window to the GPU renderer.
    pub fn apply_draw_commands(&self, window_id: WindowId, gpu: &mut GpuRenderer) {
        if let Some(ui) = self.windows.get(&window_id) {
            ui.apply_draw_commands(gpu);
        }
    }

    /// Advances background page work for a window between OS events.
    ///
    /// This keeps animated custom controls, such as an active audio timer,
    /// repainting even when the user is not moving the pointer.
    pub(crate) fn poll_window(&mut self, window_id: WindowId) -> bool {
        matches!(self.tick(window_id), BrowserCommand::RequestRedraw)
    }

    /// Returns the current window size for the given window as `(width, height)` in floating-point pixels.
    pub fn window_size(&self, window_id: WindowId) -> (f32, f32) {
        match self.windows.get(&window_id) {
            Some(ui) => {
                let (width, height) = ui.window_size();
                (width as f32, height as f32)
            }
            None => (
                self.default_window_size.0 as f32,
                self.default_window_size.1 as f32,
            ),
        }
    }

    /// Returns the window title for the given window.
    pub fn window_title(&self, window_id: WindowId) -> String {
        match self.windows.get(&window_id) {
            Some(ui) => ui.window_title(),
            None => self.default_window_title.clone(),
        }
    }

    /// Ticks the given window's UI and forwards its fetch requests to the network.
    fn tick(&mut self, window_id: WindowId) -> BrowserCommand {
        self.handle_network_messages();

        let Some(ui) = self.windows.get_mut(&window_id) else {
            return BrowserCommand::None;
        };
        let outcome = ui.tick();

        for fetch in outcome.fetches {
            let url = fetch.request.url;
            let kind = fetch.request.kind;
            log::info!("Fetch requested in App: url={}", url);
            let request = match &kind {
                FetchKind::JavaScript {
                    method,
                    headers,
                    body,
                    ..
                } => NetworkRequest {
                    url: url.to_string(),
                    method: method.clone(),
                    headers: headers.clone(),
                    body: body.clone(),
                },
                _ => NetworkRequest::get(url.to_string()),
            };
            let id = self
                .pending_fetches
                .insert(window_id, fetch.tab_id, kind, url);
            self.network.fetch_request_async(request, id);
        }

        if outcome.needs_redraw {
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

            // 該当ウィンドウの UI へ配送
            let Some(ui) = self.windows.get_mut(&window_id) else {
                log::warn!("There is no window called id={:?}", window_id);
                continue;
            };
            ui.deliver_fetch(&tab_id, kind, url, msg.response);
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
