//! Abstraction for the context menu shown when the user right-clicks the web
//! content.
//!
//! The browser core (`BrowserUi` / `BrowserRenderer`) detects right-clicks on
//! the web view and hands them to any [`ContextMenu`] implementation through
//! [`ClickContext`]: click position, the link under the cursor, and the
//! document URL. The implementation draws itself as a window-space overlay
//! (on top of the chrome) and reports selected items back as
//! [`ChromeAction`]s, exactly like a [`Chrome`](super::Chrome) does, so it can
//! be replaced by any user-designed menu without touching the core.
//!
//! All coordinates are logical pixels in window space.

use crate::browser::core::ui::chrome::ChromeAction;
use crate::engine::renderer_model::DrawCommand;
use crate::engine::ui::PointerEvent;

/// Information about the right-click that requested the context menu.
#[derive(Debug, Clone, PartialEq)]
pub struct ClickContext {
    /// Click position in window logical coordinates.
    pub window_pos: (f32, f32),
    /// Click position relative to the web content area origin (page space).
    pub page_pos: (f32, f32),
    /// URL of the link under the cursor, if any.
    pub link_url: Option<String>,
    /// URL of the document shown in the web view, if any.
    pub document_url: Option<String>,
}

/// The outcome of dispatching a pointer event to an open context menu.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuEventResult {
    /// `true` when the event hit the menu and must not reach the chrome or
    /// the page.
    pub consumed: bool,
    /// Action the browser core should perform.
    pub action: ChromeAction,
}

impl MenuEventResult {
    /// A result that consumes nothing and requests nothing.
    pub const fn none() -> Self {
        Self {
            consumed: false,
            action: ChromeAction::None,
        }
    }

    /// A result that consumes the event and requests `action`.
    pub const fn consumed(action: ChromeAction) -> Self {
        Self {
            consumed: true,
            action,
        }
    }
}

/// The context menu opened by right-clicking the web view.
///
/// The core opens the menu with [`open`](ContextMenu::open) when the web
/// content is right-pressed; while the menu reports
/// [`is_open`](ContextMenu::is_open), every pointer event over the window is
/// routed to it before the chrome or the page. The menu renders above all
/// other UI via [`draw`](ContextMenu::draw).
pub trait ContextMenu: std::fmt::Debug {
    /// Requests the menu at the given click position.
    ///
    /// Returns `true` when the menu opened (and consumed the click);
    /// implementations may return `false` to decline (e.g. no items apply).
    fn open(&mut self, ctx: &ClickContext) -> bool;

    /// Closes the menu without running an action.
    fn close(&mut self);

    /// Whether the menu is currently open.
    fn is_open(&self) -> bool;

    /// Draws the open menu into `cmd_buf` in window coordinates.
    ///
    /// Called every frame after the chrome so the menu paints on top of
    /// everything else. Implementations should emit nothing while closed.
    fn draw(&self, cmd_buf: &mut Vec<DrawCommand>, width: f32, height: f32);

    /// Dispatches a pointer event to the open menu.
    ///
    /// Only called while the menu is open. Returning
    /// [`consumed`](MenuEventResult.consumed) keeps the event away from the
    /// chrome and the page.
    fn pointer_event(&mut self, width: f32, height: f32, event: PointerEvent) -> MenuEventResult;

    /// Whether the menu changed its visual state since the last check.
    ///
    /// Consumes the flag, like [`crate::engine::ui::custom_node::CustomNode::needs_repaint`].
    fn needs_repaint(&self) -> bool;
}
