//! Abstraction for the window chrome surrounding the web content.
//!
//! The browser core (`BrowserUi` / `BrowserRenderer`) owns the window, the
//! tabs, and the web content, but knows nothing about the concrete UI a user
//! draws around that content. Any [`Chrome`] implementation can replace the
//! default [`super::basic_chrome::BasicChrome`]: draw arbitrary widgets, place
//! the content area anywhere, and translate user input into [`ChromeAction`]s.
//!
//! All coordinates are logical pixels in window space.

use url::Url;
use winit::event::{ElementState, Ime, KeyEvent};

use crate::browser::core::resource_loader::{BrowserNetworkError, BrowserResponse};
use crate::browser::core::tab::FetchKind;
use crate::browser::core::ui::FetchRequest;
use crate::browser::core::webview::JsPolicy;
use crate::engine::renderer_model::{DrawCommand, Rect};
use crate::engine::ui::PointerEvent;

/// An action the chrome wants the browser core to perform on the active tab or
/// window.
#[derive(Debug, Clone, PartialEq)]
pub enum ChromeAction {
    /// Nothing to do.
    None,
    /// The chrome changed visually; repaint the window.
    Repaint,
    /// Set the JS policy.
    SetJsPolicy(JsPolicy),
    /// Navigate the active tab to this URL (e.g. Enter in the address bar).
    Navigate(Url),
    /// Go back in the active tab's history.
    Back,
    /// Reload the active tab.
    Reload,
    /// The chrome acquired a text field and wants OS-level IME enabled.
    EnableIme,
    /// A page asked the DevTools bridge to inspect rendered state.
    DevToolsRequest {
        id: u64,
        method: String,
        params: String,
    },
}

/// The outcome of dispatching a pointer event to the chrome.
#[derive(Debug, Clone, PartialEq)]
pub struct ChromeEventResult {
    /// `true` when the event hit the chrome and must not reach the page.
    pub consumed: bool,
    /// Action the browser core should perform.
    pub action: ChromeAction,
}

impl ChromeEventResult {
    /// A result that consumes nothing and requests nothing.
    pub const fn none() -> Self {
        Self {
            consumed: false,
            action: ChromeAction::None,
        }
    }
}

/// Location of the DevTools frontend served from the bundled resources.
pub(super) const DEVTOOLS_URL: &str = "resource:///devtools/index.html";

/// The window chrome surrounding the web content.
///
/// The core lays out the page area below the chrome, draws the chrome on top of
/// it, and routes user input through the chrome. Pointer events are delivered
/// in window coordinates; the chrome answers whether it consumed them and which
/// [`ChromeAction`] they produced.
pub trait Chrome: std::fmt::Debug {
    /// Rect of the content (web view).
    ///
    /// # Return:
    /// - (width, height)
    fn content_rect(&self, width: f32, height: f32) -> Rect;

    /// Draws the chrome into `cmd_buf` in window coordinates.
    fn draw(&mut self, cmd_buf: &mut Vec<DrawCommand>, width: f32, height: f32);

    /// Advances chrome-owned tabs and returns their fetch and browser actions.
    fn tick(&mut self, actions_buf: &mut Vec<ChromeAction>) -> (Vec<FetchRequest>, bool);

    /// Delivers a resource fetch result requested by the chrome itself.
    fn deliver_fetch(
        &mut self,
        kind: FetchKind,
        url: Url,
        response: Result<BrowserResponse, BrowserNetworkError>,
    );

    /// Reflects the active tab's URL, if the chrome shows one.
    fn sync_url(&mut self, url: Option<&str>);

    /// Dispatches a pointer event to the chrome.
    ///
    /// The chrome receives every pointer event, including moves over the page
    /// area, so it can track its own hover state.
    fn pointer_event(
        &mut self,
        width: f32,
        height: f32,
        event: PointerEvent,
        state: ElementState,
    ) -> ChromeEventResult;

    fn handle_scroll(
        &mut self,
        width: f32,
        height: f32,
        mouse_x: f32,
        mouse_y: f32,
        scroll_x: f32,
        scroll_y: f32,
    );

    /// Whether the chrome currently owns keyboard/IME input (e.g. a focused
    /// address bar). While `true`, key and IME events are routed to the chrome
    /// instead of the page.
    fn accepts_text_input(&self) -> bool;

    /// Dispatches a key event while the chrome owns text input.
    fn key_event(&mut self, event: &KeyEvent, ctrl: bool) -> ChromeAction;

    /// Dispatches an IME event while the chrome owns text input.
    fn ime_event(&mut self, event: &Ime) -> ChromeAction;

    fn on_devtools_response(&mut self, id: u64, result: String);

    /// Drops any text-input focus held by the chrome (e.g. the user clicked the
    /// page).
    fn blur(&mut self);

    /// Whether the chrome changed its visual state since the last check.
    ///
    /// Consumes the flag, like [`crate::engine::ui::custom_node::CustomNode::needs_repaint`].
    fn needs_repaint(&self) -> bool;
}
