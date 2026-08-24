//! Browser UI components and window render state.

// Sub-modules
mod basic_chrome;
mod basic_context_menu;
mod chrome;
mod context_menu;
mod renderer;

pub use basic_chrome::BasicChrome;
pub use basic_context_menu::{BasicContextMenu, MenuItem};
pub use chrome::{Chrome, ChromeAction, ChromeEventResult};
pub use context_menu::{ClickContext, ContextMenu, MenuEventResult};
pub use renderer::BrowserRenderer;

use std::collections::HashMap;
use std::sync::Arc;

use url::Url;
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};

use crate::browser::Tab;
use crate::browser::core::resource_loader::{BrowserNetworkError, BrowserResponse};
use crate::browser::core::tab::{FetchKind, TabTask};
use crate::engine::layouter;
use crate::engine::layouter::types::ColorScheme;
use crate::engine::renderer_model::{DrawCommand, Rect};
use crate::engine::ui::PointerEvent;
use crate::engine::ui::custom_node::CustomNode;
use crate::engine::ui::input_text_types::{InputTextEvent, InputTextKey};
use crate::platform::renderer::gpu::GpuRenderer;

use super::BrowserCommand;

/// Render state for a browser window.
#[derive(Debug, Clone)]
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

impl Default for RenderState {
    fn default() -> Self {
        Self {
            draw_commands: Vec::new(),
            window_size: (1280, 800),
            scale_factor: 1.0,
            window_title: String::new(),
        }
    }
}

impl RenderState {
    /// Creates a new `RenderState` with the specified size, scale factor, and title.
    pub fn new(window_size: (u32, u32), scale_factor: f64, window_title: String) -> Self {
        Self {
            draw_commands: Vec::new(),
            window_size,
            scale_factor,
            window_title,
        }
    }

    /// Calculates the viewport dimensions in scaled logical pixels.
    pub fn viewport(&self) -> (f32, f32) {
        let sf = self.scale_factor as f32;
        (
            self.window_size.0 as f32 / sf,
            self.window_size.1 as f32 / sf,
        )
    }
}

/// Stores input-related state for a single browser window.
#[derive(Default)]
struct InputState {
    /// Current mouse position in window coordinates.
    mouse_position: (f64, f64),
    /// Current keyboard modifier state (Ctrl, Shift, Alt, etc.).
    modifiers: winit::keyboard::ModifiersState,
    /// The custom node currently under the pointer, if any.
    hovered: Option<Arc<dyn CustomNode>>,
    /// The DOM node under the pointer when the left button was pressed.
    ///
    /// Used to detect a completed click (press and release on the same node),
    /// which is forwarded to the page's JS `onclick` handler.
    pressed_dom_id: Option<u32>,
}

/// タブから発生したリソース取得リクエスト。
pub(crate) struct FetchRequest {
    pub(crate) tab_id: TabId,
    pub(crate) url: Url,
    pub(crate) kind: FetchKind,
}

/// [`BrowserUi::tick`] の結果。
pub(crate) struct BrowserUiTick {
    pub(crate) fetches: Vec<FetchRequest>,
    pub(crate) needs_redraw: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub usize);

impl std::fmt::Display for TabId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// BrowserUi は 1 ウィンドウ分の状態管理を担当する。
///
/// 責務:
/// - タブとアクティブタブの管理
/// - ウィンドウ・入力イベント（キーボード / IME / マウス / スクロール）の処理
/// - タブの tick と fetch 結果の配送
/// - 実際の描画は [`BrowserRenderer`] へ委譲する
pub struct BrowserUi {
    tabs: HashMap<TabId, Tab>,
    active_tab_id: TabId,
    next_tab_id: TabId,
    renderer: BrowserRenderer,
    input: InputState,

    /// ToDo: add set color scheme event
    system_color_scheme: ColorScheme,
}

impl Default for BrowserUi {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserUi {
    /// Creates a UI with the default [`BasicChrome`], the default
    /// [`BasicContextMenu`] and no tabs.
    pub fn new() -> Self {
        Self::with_chrome(Box::new(BasicChrome::new()))
    }

    /// Creates a UI with one tab and the default [`BasicChrome`].
    pub fn with_tab(tab: Tab) -> Self {
        Self::with_tab_and_chrome(tab, Box::new(BasicChrome::new()))
    }

    /// Creates a UI with a custom chrome, the default [`BasicContextMenu`]
    /// and no tabs.
    pub fn with_chrome(chrome: Box<dyn Chrome>) -> Self {
        Self::with_chrome_and_menu(chrome, Box::new(BasicContextMenu::new()))
    }

    /// Creates a UI with one tab, a custom chrome and the default
    /// [`BasicContextMenu`].
    pub fn with_tab_and_chrome(tab: Tab, chrome: Box<dyn Chrome>) -> Self {
        Self::with_tab_and_menu(tab, chrome, Box::new(BasicContextMenu::new()))
    }

    /// Creates a UI with a custom chrome, a custom context menu and no tabs.
    pub fn with_chrome_and_menu(chrome: Box<dyn Chrome>, menu: Box<dyn ContextMenu>) -> Self {
        Self {
            tabs: HashMap::new(),
            active_tab_id: TabId(0),
            next_tab_id: TabId(1),
            renderer: BrowserRenderer::with_chrome_and_menu(chrome, menu),
            input: InputState::default(),
            system_color_scheme: dark_light::detect().map(Into::into).unwrap_or_else(|e| {
                log::error!("Failed to detect system color scheme, using default: {e}");
                Default::default()
            }),
        }
    }

    /// Creates a UI with one tab, a custom chrome and a custom context menu.
    pub fn with_tab_and_menu(
        mut tab: Tab,
        chrome: Box<dyn Chrome>,
        menu: Box<dyn ContextMenu>,
    ) -> Self {
        let system_color_scheme = dark_light::detect().map(Into::into).unwrap_or_else(|e| {
            log::error!("Failed to detect system color scheme, using default: {e}");
            Default::default()
        });

        tab.set_system_color_scheme(system_color_scheme);

        let tab_id = TabId(1);

        let mut tabs = HashMap::new();
        tabs.insert(tab_id, tab);

        Self {
            tabs,
            active_tab_id: tab_id,
            next_tab_id: TabId(tab_id.0 + 1),
            renderer: BrowserRenderer::with_chrome_and_menu(chrome, menu),
            input: InputState::default(),
            system_color_scheme,
        }
    }

    /// Returns the active tab, if any.
    pub fn active_tab_id(&self) -> Option<TabId> {
        if self.active_tab_id.0 == 0 {
            None
        } else {
            Some(self.active_tab_id)
        }
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.active_tab_id().and_then(|id| self.tabs.get(&id))
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.active_tab_id().and_then(|id| self.tabs.get_mut(&id))
    }

    /// Returns the tab list.
    pub fn tab(&self, id: &TabId) -> Option<&Tab> {
        self.tabs.get(id)
    }

    pub fn tab_mut(&mut self, id: &TabId) -> Option<&mut Tab> {
        self.tabs.get_mut(id)
    }

    pub fn add_tab(&mut self, mut tab: Tab) {
        tab.set_system_color_scheme(self.system_color_scheme);
        self.next_tab_id.0 += 1;
    }

    /// ウィンドウの初期サイズ・スケール・タイトルを設定する。
    pub fn set_window(&mut self, window_size: (u32, u32), scale_factor: f64, window_title: String) {
        self.renderer
            .set_window(window_size, scale_factor, window_title);
    }

    /// Returns the window size in physical pixels.
    pub fn window_size(&self) -> (u32, u32) {
        self.renderer.render_state.window_size
    }

    /// Returns the current window title.
    pub fn window_title(&self) -> String {
        self.renderer.render_state.window_title.clone()
    }

    /// Rebuilds the render tree and sends draw commands to the GPU for this window.
    pub fn redraw(&mut self, gpu: &mut GpuRenderer) {
        let id = self.active_tab_id();
        self.renderer.redraw(&mut self.tabs, id, gpu);
    }

    /// Applies the current draw commands to the GPU renderer.
    pub fn apply_draw_commands(&self, gpu: &mut GpuRenderer) {
        self.renderer.apply_draw_commands(gpu);
    }

    /// Ticks all tabs and collects fetch requests and redraw demands.
    pub(crate) fn tick(&mut self) -> BrowserUiTick {
        let mut fetches = Vec::new();
        let mut needs_redraw = false;

        let tab_ids: Vec<_> = self.tabs.keys().copied().collect();

        for tab_id in &tab_ids {
            let Some(tab) = self.tab_mut(tab_id) else {
                continue;
            };
            for task in tab.tick() {
                match task {
                    TabTask::Fetch { url, kind } => {
                        log::info!("Fetch requested in BrowserUi: url={}", url);
                        fetches.push(FetchRequest {
                            tab_id: *tab_id,
                            url,
                            kind,
                        });
                    }
                    TabTask::NeedsRedraw => {
                        needs_redraw = true;
                    }
                    TabTask::DevToolsRequest { id, .. } => {
                        // The DevTools pane inspects the visible page; any
                        // other tab answers for itself.
                        let response = serde_json::json!({
                            "ok": false,
                            "error": "no inspected page",
                        })
                        .to_string();

                        if let Some(requester) = self.tab_mut(tab_id) {
                            requester.on_devtools_response(id, response);
                        }
                    }
                }
            }
        }

        BrowserUiTick {
            fetches,
            needs_redraw,
        }
    }

    /// Delivers a fetched resource to the target tab.
    pub(crate) fn deliver_fetch(
        &mut self,
        tab_id: &TabId,
        kind: FetchKind,
        url: Url,
        response: Result<BrowserResponse, BrowserNetworkError>,
    ) {
        let Some(tab) = self.tab_mut(tab_id) else {
            log::warn!("There is no Tab called id={}", tab_id);
            return;
        };

        match response {
            Ok(resp) => {
                log::info!("Fetch Done in BrowserUi for tab_id={}", tab_id);

                match kind {
                    FetchKind::Html => {
                        let html = String::from_utf8_lossy(&resp.body).to_string();
                        tab.on_fetch_succeeded_html(html);
                    }
                    FetchKind::Css => {
                        let css = String::from_utf8_lossy(&resp.body).to_string();
                        tab.on_fetch_succeeded_css_from(css, &url);
                    }
                    FetchKind::Script { index } => {
                        let source = String::from_utf8_lossy(&resp.body).to_string();
                        tab.on_fetch_succeeded_script(index, source);
                    }
                    FetchKind::DynamicScript { node_id } => {
                        let source = String::from_utf8_lossy(&resp.body).to_string();
                        tab.on_fetch_succeeded_dynamic_script(node_id, source);
                    }
                    FetchKind::DynamicCss { node_id } => {
                        let source = String::from_utf8_lossy(&resp.body).to_string();
                        tab.on_fetch_succeeded_dynamic_style(node_id, source);
                    }
                    FetchKind::Image { source } => {
                        tab.on_fetch_succeeded_image(source, &resp.body);
                    }
                    FetchKind::Audio { source } => {
                        tab.on_fetch_succeeded_audio(source, &resp.body);
                    }
                    FetchKind::JavaScript { request_id, .. } => {
                        let redirected = resp.url != url.as_str();
                        tab.on_fetch_succeeded_js(request_id, resp, redirected);
                    }
                }
            }
            Err(err) => {
                log::error!("NetworkError: {}", err);
                match kind {
                    FetchKind::Image { .. } | FetchKind::Audio { .. } => {
                        log::warn!("Media fetch failed without aborting page load: {}", url);
                    }
                    FetchKind::Script { index } => {
                        log::warn!("Classic script fetch failed without aborting page load: {url}");
                        tab.on_fetch_failed_script(index);
                    }
                    FetchKind::DynamicScript { node_id } => {
                        log::warn!("Dynamic script fetch failed without aborting page load: {url}");
                        tab.on_fetch_failed_dynamic_script(node_id);
                    }
                    FetchKind::DynamicCss { node_id } => {
                        log::warn!(
                            "Dynamic stylesheet fetch failed without aborting page load: {url}"
                        );
                        tab.on_fetch_failed_dynamic_style(node_id);
                    }
                    FetchKind::JavaScript { request_id, .. } => {
                        tab.on_fetch_failed_js(request_id, err.to_string());
                    }
                    FetchKind::Html | FetchKind::Css => tab.on_fetch_failed(err, url),
                }
            }
        }
    }

    /// Handles a `winit` window event for this window and returns a `BrowserCommand`.
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
                self.renderer.render_state.window_size = (size.width, size.height);
                gpu.resize(size);
                self.redraw(gpu);
                BrowserCommand::RequestRedraw
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                gpu.set_scale_factor(scale_factor);
                self.renderer.render_state.scale_factor = scale_factor;
                self.redraw(gpu);
                BrowserCommand::RequestRedraw
            }

            WindowEvent::MouseWheel { delta, .. } => {
                self.handle_scroll(delta);
                BrowserCommand::RequestRedraw
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.input.mouse_position = (position.x, position.y);
                if self.handle_pointer_move(position.x, position.y) {
                    BrowserCommand::RequestRedraw
                } else {
                    BrowserCommand::None
                }
            }

            WindowEvent::MouseInput { button, state, .. } => self.handle_mouse_input(button, state),

            WindowEvent::ModifiersChanged(modifiers) => {
                self.input.modifiers = modifiers.state();
                BrowserCommand::None
            }

            WindowEvent::KeyboardInput { event, .. } => self.handle_keyboard_input(event),

            WindowEvent::Ime(event) => self.handle_ime_input(event),

            _ => BrowserCommand::None,
        }
    }

    /// Handles keyboard input events and returns a `BrowserCommand`.
    fn handle_keyboard_input(&mut self, event: KeyEvent) -> BrowserCommand {
        // TODO: あとで消す
        const KEY_NEW_WINDOW: &str = "n";

        if event.state != ElementState::Pressed {
            return BrowserCommand::None;
        }

        let ctrl = self.input.modifiers.control_key();

        if ctrl
            && let winit::keyboard::Key::Character(ch) = &event.logical_key
            && ch.as_str().eq_ignore_ascii_case(KEY_NEW_WINDOW)
        {
            return BrowserCommand::OpenNewWindow;
        }

        let Some(tab_id) = &self.active_tab_id() else {
            return BrowserCommand::None;
        };

        // While the chrome owns text input (e.g. the address bar is focused),
        // keyboard input drives the chrome instead of the page.
        if self.renderer.chrome.accepts_text_input() {
            let action = self.renderer.chrome.key_event(&event, ctrl);
            return self.apply_chrome_action(action);
        }

        let Some((_, info)) = self.tab(tab_id).and_then(Tab::layout_and_info) else {
            return BrowserCommand::None;
        };

        let special = logical_key_to_special_event(&event.logical_key, ctrl);
        let key = logical_key_to_text_key(&event.logical_key);
        let handled = if let Some(special) = special {
            crate::engine::input::dispatch_text_input(info, special)
        } else if let Some(key) = key {
            crate::engine::input::dispatch_text_input(info, InputTextEvent::Key(key))
        } else if !ctrl && !crate::engine::input::focused_text_input_is_composing(info) {
            event.text.as_ref().is_some_and(|text| {
                crate::engine::input::dispatch_text_input(
                    info,
                    InputTextEvent::Insert(text.to_string()),
                )
            })
        } else {
            false
        };

        if handled {
            BrowserCommand::RequestRedraw
        } else {
            BrowserCommand::None
        }
    }

    /// Handles IME composition updates for the focused text input.
    fn handle_ime_input(&mut self, event: Ime) -> BrowserCommand {
        // While the chrome owns text input (e.g. the address bar is focused),
        // IME events drive the chrome instead of the page.
        if self.renderer.chrome.accepts_text_input() {
            let action = self.renderer.chrome.ime_event(&event);
            return match action {
                ChromeAction::Repaint => BrowserCommand::RequestRedraw,
                _ => BrowserCommand::None,
            };
        }

        let event = match event {
            Ime::Preedit(text, _) => InputTextEvent::Preedit(text),
            Ime::Commit(text) => InputTextEvent::Commit(text),
            Ime::Disabled => InputTextEvent::CancelComposition,
            Ime::Enabled => return BrowserCommand::None,
        };

        let Some((_, info)) = self.active_tab().and_then(Tab::layout_and_info) else {
            return BrowserCommand::None;
        };

        if crate::engine::input::dispatch_text_input(info, event) {
            BrowserCommand::RequestRedraw
        } else {
            BrowserCommand::None
        }
    }

    /// Handles mouse input events for the active tab.
    ///
    /// An open context menu intercepts every press and release before the
    /// chrome and the page; a right-press over the web content opens it.
    fn handle_mouse_input(&mut self, button: MouseButton, state: ElementState) -> BrowserCommand {
        let (x, y, sf) = (
            self.input.mouse_position.0,
            self.input.mouse_position.1,
            self.renderer.render_state.scale_factor,
        );
        let (px, py) = ((x / sf) as f32, (y / sf) as f32);

        let width = self.renderer.render_state.viewport().0;
        let height = self.renderer.render_state.viewport().1;

        // An open context menu gets every press/release before the chrome
        // and the page. Events it declines fall through to the normal flow.
        if self.renderer.menu.is_open() {
            let window_event = match state {
                ElementState::Pressed => PointerEvent::Down { x: px, y: py },
                ElementState::Released => PointerEvent::Up { x: px, y: py },
            };
            let result = self
                .renderer
                .menu
                .pointer_event(width, height, window_event);
            if result.consumed {
                return self.dispatch_action(result.action, ActionSource::ContextMenu, x, y);
            }
        } else if button == MouseButton::Right {
            // A right-press over the web content opens the context menu.
            if state == ElementState::Pressed {
                return self.open_context_menu(px, py);
            }
            return BrowserCommand::None;
        }

        if button != MouseButton::Left {
            return BrowserCommand::None;
        }

        // Click inside the chrome.
        let window_event = match state {
            ElementState::Pressed => PointerEvent::Down { x: px, y: py },
            ElementState::Released => PointerEvent::Up { x: px, y: py },
        };
        let result = self
            .renderer
            .chrome
            .pointer_event(width, height, window_event);
        if result.consumed {
            return self.dispatch_action(result.action, ActionSource::Chrome, x, y);
        }

        // Content area: dispatch to the active tab in page coordinates.
        let Rect { x: dx, y: dy, .. } = self.renderer.chrome.content_rect(width, height);

        let (px, py) = (px - dx, py - dy);

        // Hit-test the content area, dispatch the pointer event to custom
        // nodes, and remember which DOM element the press/release landed on.
        let clicked_dom_id = {
            let Some(tab) = self.active_tab() else {
                return BrowserCommand::None;
            };

            let Some((layout, info)) = tab.layout_and_info() else {
                return BrowserCommand::None;
            };

            let path = crate::engine::input::hit_test(layout, info, px, py);

            if state == ElementState::Pressed {
                crate::engine::input::dismiss_open_popups(info, &path);
            }

            let event = match state {
                ElementState::Pressed => PointerEvent::Down { x: px, y: py },
                ElementState::Released => PointerEvent::Up { x: px, y: py },
            };

            crate::engine::input::dispatch_pointer(&path, event);
            crate::engine::input::hit_dom_id(&path)
        };

        // A completed click (press and release on the same element) runs
        // the element's JS `onclick` handler, if any.
        let js_redraw = match state {
            ElementState::Pressed => {
                self.input.pressed_dom_id = clicked_dom_id;
                false
            }
            ElementState::Released => {
                let pressed = self.input.pressed_dom_id.take();

                match (pressed, clicked_dom_id) {
                    (Some(pressed), Some(released)) if pressed == released => self
                        .active_tab_mut()
                        .map(|tab| tab.on_js_click(released))
                        .unwrap_or(false),
                    _ => false,
                }
            }
        };

        // Clicking the page unfocuses the chrome's text input.
        self.renderer.chrome.blur();

        let tab = self.active_tab_mut().unwrap();

        let input_focused = handle_mouse_click(tab, px, py);
        if js_redraw {
            BrowserCommand::RequestRedraw
        } else {
            BrowserCommand::SetImeAllowed {
                allowed: input_focused,
                position: (x, y),
            }
        }
    }

    /// Applies an action produced by the chrome while it owns text input
    /// (e.g. Enter in the address bar).
    ///
    /// Chrome-originated actions always target the active page tab — never
    /// the pane that happens to own keyboard focus — so navigating from the
    /// address bar cannot load into the hidden DevTools pane tab.
    fn apply_chrome_action(&mut self, action: ChromeAction) -> BrowserCommand {
        match action {
            ChromeAction::Repaint => BrowserCommand::RequestRedraw,
            ChromeAction::None => BrowserCommand::None,
            action => {
                let (x, y) = self.input.mouse_position;
                self.dispatch_action(action, ActionSource::Chrome, x, y)
            }
        }
    }

    /// Applies a [`ChromeAction`] produced by the chrome or the context menu
    /// to the active tab.
    ///
    /// `source` decides whose repaint flag is consumed to close the event
    /// handling; `(x, y)` are window coordinates for the IME request.
    fn dispatch_action(
        &mut self,
        action: ChromeAction,
        source: ActionSource,
        x: f64,
        y: f64,
    ) -> BrowserCommand {
        match action {
            // Pressing the URL bar enables the OS IME so the caret and
            // input methods work; the platform handler also requests a
            // redraw.
            ChromeAction::EnableIme => {
                if let Some(tab) = self.active_tab()
                    && let Some((_, info)) = tab.layout_and_info()
                {
                    crate::engine::input::focus_text_input(info, None);
                }
                return BrowserCommand::SetImeAllowed {
                    allowed: true,
                    position: (x, y),
                };
            }
            ChromeAction::Back => {
                if let Some(tab) = self.active_tab_mut() {
                    tab.go_back();
                }
            }
            ChromeAction::Reload => {
                if let Some(tab) = self.active_tab_mut() {
                    tab.reload();
                }
            }
            ChromeAction::Navigate(url) => {
                if let Some(tab) = self.active_tab_mut() {
                    tab.navigate(url);
                }
            }
            ChromeAction::SetJsPolicy(policy) => {
                if let Some(tab) = self.active_tab_mut() {
                    tab.set_js_policy(policy);
                    tab.reload();
                }
            }
            ChromeAction::Repaint | ChromeAction::None => {}
        }

        let needs_repaint = match source {
            ActionSource::Chrome => self.renderer.chrome.needs_repaint(),
            ActionSource::ContextMenu => self.renderer.menu.needs_repaint(),
        };
        if needs_repaint {
            BrowserCommand::RequestRedraw
        } else {
            BrowserCommand::SetImeAllowed {
                allowed: false,
                position: (x, y),
            }
        }
    }

    /// Opens the context menu for a right-press at window logical `(px, py)`.
    ///
    /// Builds a [`ClickContext`] (positions, link under the cursor, document
    /// URL) and hands it to the menu. The menu only opens over the inspected
    /// page pane, never over the chrome or the DevTools pane.
    fn open_context_menu(&mut self, px: f32, py: f32) -> BrowserCommand {
        let width = self.renderer.render_state.viewport().0;
        let height = self.renderer.render_state.viewport().1;

        let Rect {
            x: dx,
            y: dy,
            width: content_width,
            height: content_height,
        } = self.renderer.chrome.content_rect(width, height);
        if px < dx || py < dy || px > dx + content_width || py > dy + content_height {
            return BrowserCommand::None;
        }

        let page_pos = (px - dx, py - dy);

        let Some(tab) = self.active_tab() else {
            return BrowserCommand::None;
        };
        let document_url = tab.document_url().map(|url| url.to_string());
        let link_url = tab.layout_and_info().and_then(|(layout, info)| {
            let path = crate::engine::input::hit_test(layout, info, page_pos.0, page_pos.1);
            link_href_at(&path)
        });

        let ctx = ClickContext {
            window_pos: (px, py),
            page_pos,
            link_url,
            document_url,
        };

        if self.renderer.menu.open(&ctx) {
            BrowserCommand::RequestRedraw
        } else {
            BrowserCommand::None
        }
    }

    /// Dispatches a pointer move and updates hover state for the active tab.
    ///
    /// Returns whether the move changed any visual state (and thus requires a
    /// repaint).
    fn handle_pointer_move(&mut self, x: f64, y: f64) -> bool {
        let sf = self.renderer.render_state.scale_factor;
        let (px, py) = ((x / sf) as f32, (y / sf) as f32);
        let v_width = self.renderer.render_state.viewport().0;
        let v_height = self.renderer.render_state.viewport().1;

        // An open context menu intercepts every move before chrome/page.
        if self.renderer.menu.is_open()
            && self
                .renderer
                .menu
                .pointer_event(v_width, v_height, PointerEvent::Move { x: px, y: py })
                .consumed
        {
            return self.renderer.menu.needs_repaint();
        }

        // The chrome receives every move so it can track its own hover state.
        let result = self.renderer.chrome.pointer_event(
            v_width,
            v_height,
            PointerEvent::Move { x: px, y: py },
        );
        if result.consumed {
            return self.renderer.chrome.needs_repaint();
        }

        // Content area: dispatch to the page in page coordinates.
        let viewport = self.renderer.chrome.content_rect(v_width, v_height);
        let Rect { x: px, y: py, .. } = viewport;

        let Some(tab) = self.active_tab() else {
            return false;
        };
        let Some((layout, info)) = tab.layout_and_info() else {
            return false;
        };
        let path = crate::engine::input::hit_test(layout, info, px, py);
        let mut repaint =
            crate::engine::input::dispatch_pointer(&path, PointerEvent::Move { x: px, y: py });
        let previous = self.input.hovered.clone();
        if crate::engine::input::update_hover(&path, previous.as_ref()) {
            repaint = true;
            self.input.hovered = crate::engine::input::hit_custom_node(&path).cloned();
        }
        repaint || self.renderer.chrome.needs_repaint()
    }

    /// Handles scrolling for the pane under the pointer, updating its layout
    /// container offsets; scrolls outside web content go to the chrome.
    fn handle_scroll(&mut self, delta: MouseScrollDelta) {
        let (scroll_x, scroll_y) = match delta {
            MouseScrollDelta::LineDelta(x, y) => (-x * 60.0, -y * 60.0),
            MouseScrollDelta::PixelDelta(pos) => (-pos.x as f32, -pos.y as f32),
        };

        let (w_width, w_height) = self.renderer.render_state.viewport();
        let Rect {
            x: sx,
            y: sy,
            width,
            height,
        } = self.renderer.chrome.content_rect(w_width, w_height);

        let sf = self.renderer.render_state.scale_factor;
        let (mouse_x, mouse_y) = (
            (self.input.mouse_position.0 / sf) as f32 - sx,
            (self.input.mouse_position.1 / sf) as f32 - sy,
        );

        let outside_content =
            mouse_x < sx || mouse_y < sy || mouse_x > sx + width || mouse_y > sy + height;

        if outside_content {
            self.renderer
                .chrome
                .handle_scroll(w_width, w_height, mouse_x, mouse_y, scroll_x, scroll_y);
            return;
        }

        let Some(tab) = self.active_tab_mut() else {
            return;
        };

        let Some((layout, info)) = tab.layout_and_info_mut() else {
            return;
        };

        // Prefer the scrollable container under the cursor.
        crate::engine::input::scroll_at(
            layout,
            info,
            (width, height),
            mouse_x,
            mouse_y,
            scroll_x,
            scroll_y,
        );
    }
}
/// Handles a mouse click in the given tab at the specified coordinates.
fn handle_mouse_click(tab: &mut Tab, x: f32, y: f32) -> bool {
    let hit_path = match tab.layout_and_info() {
        Some((layout, info)) => crate::engine::input::hit_test(layout, info, x, y),
        None => return false,
    };

    let input_target = hit_path.iter().find_map(|hit| {
        if let layouter::types::NodeKind::Custom { node, .. } = &hit.info.kind
            && node.accepts_text_input()
        {
            Some(Arc::clone(node))
        } else {
            None
        }
    });
    let input_focused = tab.layout_and_info().is_some_and(|(_, info)| {
        crate::engine::input::focus_text_input(info, input_target.as_ref())
    });

    let href_opt = link_href_at(&hit_path);

    if let Some(href) = href_opt {
        tab.move_to(&href)
    }
    input_focused
}

/// Returns the href of the innermost link on the hit path, if any.
///
/// The hit path is ordered child→parent, so the first match is the deepest
/// link under the pointer.
fn link_href_at(hit_path: &crate::engine::input::HitPath<'_>) -> Option<String> {
    hit_path.iter().find_map(|hit| {
        if let layouter::types::NodeKind::Container { role, .. } = &hit.info.kind
            && let layouter::types::ContainerRole::Link { href } = role
        {
            Some(href.clone())
        } else {
            None
        }
    })
}

/// Where a [`ChromeAction`] came from; decides whose repaint flag closes the
/// event handling in [`BrowserUi::dispatch_action`].
#[derive(Debug, Clone, Copy, PartialEq)]
enum ActionSource {
    /// The action was produced by the chrome.
    Chrome,
    /// The action was produced by the context menu.
    ContextMenu,
}

/// Maps a logical key to a text-editing navigation key, if any.
fn logical_key_to_text_key(key: &winit::keyboard::Key) -> Option<InputTextKey> {
    match key {
        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Backspace) => {
            Some(InputTextKey::Backspace)
        }
        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Delete) => {
            Some(InputTextKey::Delete)
        }
        winit::keyboard::Key::Named(winit::keyboard::NamedKey::ArrowLeft) => {
            Some(InputTextKey::Left)
        }
        winit::keyboard::Key::Named(winit::keyboard::NamedKey::ArrowRight) => {
            Some(InputTextKey::Right)
        }
        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Home) => Some(InputTextKey::Home),
        winit::keyboard::Key::Named(winit::keyboard::NamedKey::End) => Some(InputTextKey::End),
        _ => None,
    }
}

/// Maps a logical key to a text-editing special event (undo/redo/enter).
fn logical_key_to_special_event(key: &winit::keyboard::Key, ctrl: bool) -> Option<InputTextEvent> {
    if ctrl && let winit::keyboard::Key::Character(ch) = key {
        match ch.as_str() {
            "z" | "Z" => Some(InputTextEvent::Undo),
            "y" | "Y" => Some(InputTextEvent::Redo),
            _ => None,
        }
    } else if let winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter) = key {
        Some(InputTextEvent::Enter)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::core::webview::JsPolicy;
    use crate::engine::layouter::types::ColorScheme;

    fn ui_with_one_tab() -> BrowserUi {
        let tab = Tab::new(ColorScheme::default(), JsPolicy::default());
        BrowserUi::with_tab(tab)
    }

    #[test]
    fn get_document_serializes_children_under_the_document_root() {
        let mut ui = ui_with_one_tab();
        ui.active_tab_mut()
            .unwrap()
            .navigate("https://example.test/index.html".parse().expect("url"));
        ui.active_tab_mut()
            .unwrap()
            .on_fetch_succeeded_html("<html><body><p id=\"a\">hello</p></body></html>".to_string());

        let doc = ui
            .active_tab_mut()
            .unwrap()
            .inspect("getDocument", "{}")
            .expect("document payload");
        assert_eq!(doc["type"], "document");

        // The frontend descends into synthetic roots, so the document node
        // must expose element children (e.g. <html>).
        let children = doc["children"].as_array().expect("children array");
        assert!(
            children
                .iter()
                .any(|child| child["type"] == "element" && child["tag"] == "html"),
            "document root must expose the <html> element: {doc}"
        );
    }
}
