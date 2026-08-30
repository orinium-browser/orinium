//! Default browser chrome: a toolbar with a back button, a reload button, and a
//! URL bar, laid out above the page content area.
//!
//! This is the stock [`Chrome`] implementation used when no custom chrome is
//! provided. It is deliberately simple and hardcoded; the core only talks to it
//! through the [`Chrome`] trait, so it can be replaced by any user-designed UI.
//!
//! The chrome is drawn with direct [`DrawCommand`]s (a fixed row layout, not a
//! full `ui_layout` tree): each component draws its own background and content
//! inside a translated coordinate system, exactly as the engine does for
//! replaced elements.

use std::sync::Arc;

use ui_layout::Style;
use url::Url;
use winit::event::{ElementState, Ime, KeyEvent};

use crate::browser::Tab;
use crate::browser::core::resource_loader::{BrowserNetworkError, BrowserResponse};
use crate::browser::core::tab::{FetchKind, TabTask};
use crate::browser::core::ui::chrome::{Chrome, ChromeAction, ChromeEventResult, DEVTOOLS_URL};
use crate::browser::core::ui::{
    FetchRequest, logical_key_to_special_event, logical_key_to_text_key,
};
use crate::engine::bridge::text::TextMeasurer;
use crate::engine::html::ScriptingMode;
use crate::engine::layouter::types::{Color, TextFlowStyle, TextStyle};
use crate::engine::renderer_model::{
    AffineTransform, Brush, DrawCommand, FillRule, Paint, Rect, rect_path,
};
use crate::engine::ui::button::ButtonComponent;
use crate::engine::ui::custom_node::{ContentSize, CustomNode, PointerEvent};
use crate::engine::ui::input_text::InputTextComponent;
use crate::engine::ui::input_text_types::InputTextEvent;
use crate::platform::renderer::text_measurer::PlatformTextMeasurer;

/// Horizontal and vertical spacing between chrome elements.
const CHROME_PADDING: f32 = 8.0;
/// Gap between adjacent toolbar elements.
const CHROME_GAP: f32 = 8.0;
/// Toolbar background color.
const TOOLBAR_BACKGROUND: Color = Color(210, 210, 214, 255);
/// Button background color.
const BUTTON_BACKGROUND: Color = Color(240, 240, 240, 255);
/// Label color used by toolbar buttons.
const LABEL_COLOR: Color = Color(20, 20, 20, 255);

/// Identifies the toolbar element under a pointer position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChromeHit {
    /// Back navigation button.
    Back,
    /// Reload current page button.
    Reload,
    /// Scripting button (toggle the scripting mode).
    Scripting,
    /// DevTools pane toggle button.
    DevTools,
    /// URL entry bar.
    UrlBar,
}

/// Rectangles of the toolbar elements for a given window width.
#[derive(Debug, Clone, Copy)]
struct ToolbarRects {
    /// The whole toolbar strip (top edge of the window).
    toolbar: Rect,
    /// Back button.
    back: Rect,
    /// Reload button.
    reload: Rect,
    /// Scrinpting button.
    scripting: Rect,
    /// DevTools pane toggle button.
    devtools: Rect,
    /// URL bar.
    url_bar: Rect,
}

impl ToolbarRects {
    /// Height of the toolbar strip in logical pixels.
    fn height(&self) -> f32 {
        self.toolbar.height
    }
}

/// Represents the top toolbar of the browser.
#[derive(Debug)]
struct BrowserToolbar {
    /// Back navigation button.
    back_button: ButtonComponent,
    /// Reload current page button.
    reload_button: ButtonComponent,
    /// DevTools pane toggle button.
    devtools_button: ButtonComponent,
    /// Switch the scripting mode.
    scripting_button: ButtonComponent,
    /// URL entry bar.
    url_bar: InputTextComponent,
}

impl BrowserToolbar {
    /// Create a new toolbar with placeholder components.
    fn new() -> Self {
        let measurer: Arc<dyn TextMeasurer> = Arc::new(PlatformTextMeasurer::new().unwrap());
        let back_button = ButtonComponent::new(
            "← Back",
            BUTTON_BACKGROUND,
            LABEL_COLOR,
            Arc::clone(&measurer),
        );
        let reload_button = ButtonComponent::new(
            "⟳ Reload",
            BUTTON_BACKGROUND,
            LABEL_COLOR,
            Arc::clone(&measurer),
        );
        let scripting_button = ButtonComponent::new(
            display_scripting(&ScriptingMode::default()),
            BUTTON_BACKGROUND,
            LABEL_COLOR,
            Arc::clone(&measurer),
        );
        let devtools_button = ButtonComponent::new(
            "DevTools",
            BUTTON_BACKGROUND,
            LABEL_COLOR,
            Arc::clone(&measurer),
        );
        let url_bar = InputTextComponent::new("", "Enter URL", measurer);
        Self {
            back_button,
            reload_button,
            scripting_button,
            devtools_button,
            url_bar,
        }
    }

    /// Computes the layout of the toolbar row for the given window width.
    fn rects(&self, width: f32) -> ToolbarRects {
        let back_size = self.back_button.intrinsic_size();
        let reload_size = self.reload_button.intrinsic_size();
        let scripting_size = self.scripting_button.intrinsic_size();
        let devtools_size = self.devtools_button.intrinsic_size();
        let url_size = self.url_bar.intrinsic_size();

        let row_height = [back_size.height, reload_size.height, url_size.height]
            .into_iter()
            .fold(0.0, f32::max);
        let top = CHROME_PADDING;
        let center_y = |height: f32| top + (row_height - height) * 0.5;

        let back = Rect::new(
            CHROME_PADDING,
            center_y(back_size.height),
            back_size.width,
            back_size.height,
        );
        let reload = Rect::new(
            back.x + back.width + CHROME_GAP,
            center_y(reload_size.height),
            reload_size.width,
            reload_size.height,
        );
        let scripting = Rect::new(
            reload.x + reload.width + CHROME_GAP,
            center_y(scripting_size.height),
            scripting_size.width,
            scripting_size.height,
        );
        let devtools = Rect::new(
            scripting.x + scripting.width + CHROME_GAP,
            center_y(devtools_size.height),
            devtools_size.width,
            devtools_size.height,
        );
        let url_x = (devtools.x + devtools.width + CHROME_GAP).min(width - CHROME_PADDING);
        let url_width = (width - CHROME_PADDING - url_x).max(0.0);
        let url_bar = Rect::new(url_x, center_y(url_size.height), url_width, url_size.height);

        ToolbarRects {
            toolbar: Rect::new(0.0, 0.0, width, row_height + CHROME_PADDING * 2.0),
            back,
            reload,
            scripting,
            devtools,
            url_bar,
        }
    }

    /// Returns the toolbar element under `(x, y)`, or `None` when the point is
    /// outside the chrome (i.e. over the page content).
    fn hit_test(&self, x: f32, y: f32, width: f32) -> Option<ChromeHit> {
        let rects = self.rects(width);
        if rects.back.contains(x, y) {
            Some(ChromeHit::Back)
        } else if rects.reload.contains(x, y) {
            Some(ChromeHit::Reload)
        } else if rects.scripting.contains(x, y) {
            Some(ChromeHit::Scripting)
        } else if rects.devtools.contains(x, y) {
            Some(ChromeHit::DevTools)
        } else if rects.url_bar.contains(x, y) {
            Some(ChromeHit::UrlBar)
        } else {
            None
        }
    }
}

/// The default chrome for a browser window: a top toolbar and the page content
/// area below it.
#[derive(Debug)]
pub struct BasicChrome {
    toolbar: BrowserToolbar,
    /// URL currently shown in the address bar, used to avoid overwriting text
    /// the user is editing.
    last_url: Option<String>,

    is_debug_open: bool,
    debug_pane: Tab,

    /// Whether a press that started inside the debug pane is still held.
    /// Its release must be routed back to the pane even if the pointer has
    /// since moved over the toolbar or the page, so the click completes.
    debug_press_active: bool,

    scripting_mode: ScriptingMode,
    /// Toolbar element currently under the pointer, if any.
    hovered: Option<ChromeHit>,
}

impl BasicChrome {
    /// Create a new default chrome with an empty toolbar.
    pub fn new() -> Self {
        let mut tab = Tab::default();
        tab.navigate(DEVTOOLS_URL.parse().unwrap());

        Self {
            toolbar: BrowserToolbar::new(),
            last_url: None,
            is_debug_open: false,
            debug_pane: tab,
            debug_press_active: false,
            scripting_mode: ScriptingMode::default(),
            hovered: None,
        }
    }

    /// Dispatches a pointer event to the element under `hit` and returns
    /// whether the element consumed it. For [`PointerEvent::Up`] the value
    /// doubles as "a click was completed".
    fn dispatch(&self, hit: ChromeHit, event: PointerEvent) -> bool {
        let node: &dyn CustomNode = match hit {
            ChromeHit::Back => &self.toolbar.back_button,
            ChromeHit::Reload => &self.toolbar.reload_button,
            ChromeHit::Scripting => &self.toolbar.scripting_button,
            ChromeHit::DevTools => &self.toolbar.devtools_button,
            ChromeHit::UrlBar => &self.toolbar.url_bar,
        };
        node.on_pointer_event(event)
    }

    /// Drops any hover state when the pointer leaves the toolbar.
    fn clear_hover(&mut self) {
        if let Some(previous) = self.hovered.take() {
            self.dispatch(previous, PointerEvent::Leave);
        }
    }
}

impl Default for BasicChrome {
    fn default() -> Self {
        Self::new()
    }
}

impl Chrome for BasicChrome {
    fn content_rect(&self, width: f32, height: f32) -> Rect {
        let toolbar_height = self.toolbar.rects(width).height();
        if self.is_debug_open {
            Rect::new(0.0, toolbar_height, width / 2.0, height - toolbar_height)
        } else {
            Rect::new(0.0, toolbar_height, width, height - toolbar_height)
        }
    }

    fn draw(&mut self, cmd_buf: &mut Vec<DrawCommand>, width: f32, height: f32) {
        let rects = self.toolbar.rects(width);

        cmd_buf.push(DrawCommand::Fill {
            path: rect_path(
                rects.toolbar.x,
                rects.toolbar.y,
                rects.toolbar.width,
                rects.toolbar.height,
            ),
            rule: FillRule::NonZero,
            paint: Paint {
                brush: Brush::Solid(TOOLBAR_BACKGROUND),
                opacity: 1.0,
            },
        });

        let text_style = TextStyle::default();
        let text_flow_style = TextFlowStyle::default();
        let style = Style::default();

        let components: [(&dyn CustomNode, Rect); 5] = [
            (&self.toolbar.back_button, rects.back),
            (&self.toolbar.reload_button, rects.reload),
            (&self.toolbar.scripting_button, rects.scripting),
            (&self.toolbar.devtools_button, rects.devtools),
            (&self.toolbar.url_bar, rects.url_bar),
        ];

        for (node, rect) in components {
            cmd_buf.push(DrawCommand::PushTransform {
                transform: AffineTransform::translate(rect.x, rect.y),
            });
            node.draw_sized(
                cmd_buf,
                &text_style,
                &text_flow_style,
                &style,
                ContentSize {
                    width: rect.width,
                    height: rect.height,
                },
            );
            cmd_buf.push(DrawCommand::PopTransform);
        }

        if self.is_debug_open {
            cmd_buf.push(DrawCommand::PushTransform {
                transform: AffineTransform::translate(width / 2.0, rects.height()),
            });

            let rect_path = rect_path(0.0, 0.0, width / 2.0, height - rects.height());
            cmd_buf.push(DrawCommand::Fill {
                path: rect_path.clone(),
                rule: FillRule::NonZero,
                paint: Paint {
                    brush: Brush::Solid(Color(200, 200, 200, 200)),
                    opacity: 1.0,
                },
            });
            cmd_buf.push(DrawCommand::PushClip {
                path: rect_path,
                rule: FillRule::NonZero,
            });

            self.debug_pane
                .draw(cmd_buf, width / 2.0, height - rects.height());

            cmd_buf.push(DrawCommand::PopClip);
            cmd_buf.push(DrawCommand::PopTransform);
        }
    }

    fn tick(&mut self, actions_buf: &mut Vec<ChromeAction>) -> (Vec<FetchRequest>, bool) {
        let mut fetches_buf = Vec::new();
        let mut redraw = false;
        for task in self.debug_pane.tick() {
            match task {
                TabTask::Fetch { url, kind, origin } => {
                    log::info!("Fetch requested in BasicChrome: url={}", url);
                    fetches_buf.push(FetchRequest { url, kind, origin });
                }
                TabTask::NeedsRedraw => redraw = true,
                TabTask::DevToolsRequest { id, method, params } => {
                    actions_buf.push(ChromeAction::DevToolsRequest { id, method, params });
                }
            }
        }

        (fetches_buf, redraw)
    }

    fn deliver_fetch(
        &mut self,
        kind: FetchKind,
        url: Url,
        response: Result<BrowserResponse, BrowserNetworkError>,
    ) {
        self.debug_pane.deliver_fetch(kind, url, response);
    }

    fn sync_url(&mut self, url: Option<&str>) {
        let url = url.map(str::to_string);
        if self.last_url != url {
            self.last_url.clone_from(&url);
            self.toolbar.url_bar.set_value(url.unwrap_or_default());
        }
    }

    fn pointer_event(
        &mut self,
        width: f32,
        height: f32,
        event: PointerEvent,
        state: ElementState,
    ) -> ChromeEventResult {
        let (x, y) = match event {
            PointerEvent::Move { x, y }
            | PointerEvent::Down { x, y }
            | PointerEvent::Up { x, y } => (x, y),
            PointerEvent::Leave => {
                self.clear_hover();
                return ChromeEventResult::none();
            }
        };

        let Some(hit) = self.toolbar.hit_test(x, y, width) else {
            // Pointer over the page or the debug pane: clear any chrome hover.
            self.clear_hover();

            if !self.is_debug_open {
                return ChromeEventResult::none();
            }

            let rects = self.toolbar.rects(width);
            let pane = Rect {
                x: width / 2.0,
                y: rects.height(),
                width: width / 2.0,
                height: height - rects.height(),
            };
            let inside = pane.contains(x, y);

            // The page area must not be consumed, or clicks would never
            // reach the browsed tab. Only the debug pane (and a press that
            // started there) is handled by the chrome.
            if !inside && !self.debug_press_active {
                return ChromeEventResult::none();
            }

            // Debug-pane local coordinates.
            let px = x - pane.x;
            let py = y - pane.y;

            match event {
                // Moves only update hover; routing them through
                // `handle_mouse_input` would synthesize pointer-ups and
                // cancel in-flight clicks.
                PointerEvent::Move { .. } => {
                    self.debug_pane.handle_pointer_move(px, py);
                }
                PointerEvent::Down { .. } => {
                    self.debug_press_active = inside;
                    self.debug_pane.handle_mouse_input(px, py, state);
                }
                _ => {
                    self.debug_press_active = false;
                    self.debug_pane.handle_mouse_input(px, py, state);
                }
            }

            return ChromeEventResult {
                consumed: true,
                action: ChromeAction::None,
            };
        };

        // A release that belongs to a press started in the debug pane never
        // activates chrome buttons the pointer happens to have drifted over.
        if matches!(event, PointerEvent::Up { .. }) && self.debug_press_active {
            self.debug_press_active = false;
            let rects = self.toolbar.rects(width);
            self.debug_pane
                .handle_mouse_input(x - width / 2.0, y - rects.height(), state);
            return ChromeEventResult {
                consumed: true,
                action: ChromeAction::None,
            };
        }

        let handled = self.dispatch(hit, event);
        let clicked = matches!(event, PointerEvent::Up { .. }) && handled;

        if matches!(event, PointerEvent::Move { .. })
            && self.hovered != Some(hit)
            && let Some(previous) = self.hovered.replace(hit)
        {
            self.dispatch(previous, PointerEvent::Leave);
        }

        let action = match hit {
            ChromeHit::UrlBar if matches!(event, PointerEvent::Down { .. }) => {
                self.toolbar.url_bar.set_focused(true);
                ChromeAction::EnableIme
            }
            ChromeHit::Back if clicked => ChromeAction::Back,
            ChromeHit::Reload if clicked => ChromeAction::Reload,
            ChromeHit::Scripting if clicked => {
                self.scripting_mode = if self.scripting_mode == ScriptingMode::Enabled {
                    ScriptingMode::Disabled
                } else {
                    ScriptingMode::Enabled
                };

                self.toolbar.scripting_button.label =
                    display_scripting(&self.scripting_mode).into();

                ChromeAction::SetJsPolicy(self.scripting_mode.into())
            }
            ChromeHit::DevTools if clicked => {
                self.is_debug_open = !self.is_debug_open;
                ChromeAction::None
            }
            _ => ChromeAction::None,
        };

        ChromeEventResult {
            consumed: true,
            action,
        }
    }

    fn handle_scroll(
        &mut self,
        width: f32,
        height: f32,
        mouse_x: f32,
        mouse_y: f32,
        scroll_x: f32,
        scroll_y: f32,
    ) {
        if self.is_debug_open {
            let rects = self.toolbar.rects(width);

            let rect = Rect {
                x: width / 2.0,
                y: rects.height(),
                width: width / 2.0,
                height: height - rects.height(),
            };

            // The core hands us coordinates relative to the content rect
            // origin; the pane sits at (width / 2, toolbar height) in window
            // space, so only its horizontal offset needs translating.
            if rect.contains(mouse_x, mouse_y + rects.height()) {
                self.debug_pane.scroll_at(
                    mouse_x - rect.x,
                    mouse_y,
                    scroll_x,
                    scroll_y,
                    (rect.width, rect.height),
                );
            }
        }
    }

    fn accepts_text_input(&self) -> bool {
        self.toolbar.url_bar.is_focused()
    }

    fn key_event(&mut self, event: &KeyEvent, ctrl: bool) -> ChromeAction {
        if let winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter) = &event.logical_key {
            let url = self.toolbar.url_bar.state().value;
            self.toolbar
                .url_bar
                .handle_text_input(InputTextEvent::Enter);
            match Url::parse(&url).or_else(|_| Url::parse(&format!("https://{url}"))) {
                Ok(url) => ChromeAction::Navigate(url),
                Err(_) => {
                    log::warn!("Ignoring invalid URL entered in address bar: {}", url);
                    ChromeAction::Repaint
                }
            }
        } else {
            let special = logical_key_to_special_event(&event.logical_key, ctrl);
            let key = logical_key_to_text_key(&event.logical_key);
            let handled = if let Some(special) = special {
                self.toolbar.url_bar.handle_text_input(special)
            } else if let Some(key) = key {
                self.toolbar
                    .url_bar
                    .handle_text_input(InputTextEvent::Key(key))
            } else if !ctrl && !self.toolbar.url_bar.is_composing() {
                event.text.as_ref().is_some_and(|text| {
                    self.toolbar
                        .url_bar
                        .handle_text_input(InputTextEvent::Insert(text.to_string()))
                })
            } else {
                false
            };

            if handled {
                ChromeAction::Repaint
            } else {
                ChromeAction::None
            }
        }
    }

    fn ime_event(&mut self, event: &Ime) -> ChromeAction {
        let event = match event {
            Ime::Preedit(text, _) => InputTextEvent::Preedit(text.clone()),
            Ime::Commit(text) => InputTextEvent::Commit(text.clone()),
            Ime::Disabled => InputTextEvent::CancelComposition,
            Ime::Enabled => return ChromeAction::None,
        };

        if self.toolbar.url_bar.handle_text_input(event) {
            ChromeAction::Repaint
        } else {
            ChromeAction::None
        }
    }

    fn on_devtools_response(&mut self, id: u64, result: String) {
        self.debug_pane.on_devtools_response(id, result);
    }

    fn blur(&mut self) {
        self.toolbar.url_bar.set_focused(false);
    }

    fn needs_repaint(&self) -> bool {
        self.toolbar.back_button.needs_repaint()
            || self.toolbar.reload_button.needs_repaint()
            || self.toolbar.url_bar.needs_repaint()
    }
}

fn display_scripting(scripting_mode: &ScriptingMode) -> &'static str {
    match scripting_mode {
        ScriptingMode::Enabled => "JS: Enabled ",
        ScriptingMode::Disabled => "JS: Disabled",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Completes a press/release cycle at `(x, y)` over the chrome.
    fn click(chrome: &mut BasicChrome, x: f32, y: f32) -> ChromeAction {
        let down = chrome.pointer_event(
            800.0,
            600.0,
            PointerEvent::Down { x, y },
            ElementState::Pressed,
        );
        assert!(down.consumed);

        let up = chrome.pointer_event(
            800.0,
            600.0,
            PointerEvent::Up { x, y },
            ElementState::Released,
        );
        assert!(up.consumed);
        up.action
    }

    /// Clicks the DevTools button so the debug pane is open afterwards.
    fn open_debug_pane(chrome: &mut BasicChrome) -> ToolbarRects {
        let rects = chrome.toolbar.rects(800.0);
        let action = click(chrome, rects.devtools.x + 1.0, rects.devtools.y + 1.0);
        assert_eq!(action, ChromeAction::None);
        assert!(chrome.is_debug_open);
        rects
    }

    #[test]
    fn toolbar_rects_layout_left_to_right() {
        let toolbar = BrowserToolbar::new();
        let rects = toolbar.rects(800.0);

        assert!(rects.back.width > 0.0);
        assert!(rects.reload.width > 0.0);
        assert!(rects.url_bar.width > 0.0);
        assert!(rects.toolbar.width >= 800.0);

        // Elements are placed left to right without overlap.
        assert!(rects.back.x < rects.reload.x);
        assert!(rects.reload.x + rects.reload.width < rects.url_bar.x);

        // The URL bar reaches the right edge (minus padding).
        assert!((rects.url_bar.x + rects.url_bar.width + CHROME_PADDING - 800.0).abs() < 0.001);
    }

    #[test]
    fn toolbar_rects_fits_narrow_windows() {
        let toolbar = BrowserToolbar::new();
        let rects = toolbar.rects(120.0);
        assert!(rects.toolbar.height > 0.0);
        // URL bar must not extend past the window edge.
        assert!(rects.url_bar.x + rects.url_bar.width <= 120.0 + 0.001);
    }

    #[test]
    fn pointer_events_hit_chrome_and_content() {
        let mut chrome = BasicChrome::new();
        let rects = chrome.toolbar.rects(800.0);

        // A completed click on the back button requests Back.
        let result = chrome.pointer_event(
            800.0,
            600.0,
            PointerEvent::Down {
                x: rects.back.x + 1.0,
                y: rects.back.y + 1.0,
            },
            ElementState::Pressed,
        );
        assert!(result.consumed);
        assert_eq!(result.action, ChromeAction::None);

        let result = chrome.pointer_event(
            800.0,
            600.0,
            PointerEvent::Up {
                x: rects.back.x + 1.0,
                y: rects.back.y + 1.0,
            },
            ElementState::Released,
        );
        assert!(result.consumed);
        assert_eq!(result.action, ChromeAction::Back);

        // Pressing the URL bar requests OS-level IME.
        let result = chrome.pointer_event(
            800.0,
            600.0,
            PointerEvent::Down {
                x: rects.url_bar.x + 1.0,
                y: rects.url_bar.y + 1.0,
            },
            ElementState::Pressed,
        );
        assert!(result.consumed);
        assert_eq!(result.action, ChromeAction::EnableIme);
        assert!(chrome.accepts_text_input());

        // Below the toolbar is the page content area.
        let result = chrome.pointer_event(
            800.0,
            600.0,
            PointerEvent::Down {
                x: 400.0,
                y: rects.toolbar.height + 10.0,
            },
            ElementState::Pressed,
        );
        assert!(!result.consumed);
        assert_eq!(result.action, ChromeAction::None);
    }

    #[test]
    fn hovering_tracks_toolbar_elements() {
        let mut chrome = BasicChrome::new();
        let rects = chrome.toolbar.rects(800.0);

        // Move onto the back button, then onto the reload button: both become
        // dirty, and no event falls through to the page.
        let result = chrome.pointer_event(
            800.0,
            600.0,
            PointerEvent::Move {
                x: rects.back.x + 1.0,
                y: rects.back.y + 1.0,
            },
            ElementState::Released,
        );
        assert!(result.consumed);

        let result = chrome.pointer_event(
            800.0,
            600.0,
            PointerEvent::Move {
                x: rects.reload.x + 1.0,
                y: rects.reload.y + 1.0,
            },
            ElementState::Released,
        );
        assert!(result.consumed);

        // Leaving the toolbar clears hover state.
        let result = chrome.pointer_event(
            800.0,
            600.0,
            PointerEvent::Move {
                x: 400.0,
                y: rects.toolbar.height + 10.0,
            },
            ElementState::Released,
        );
        assert!(!result.consumed);
        assert!(!chrome.toolbar.back_button.is_hovered());
        assert!(!chrome.toolbar.reload_button.is_hovered());
    }

    #[test]
    fn sync_url_updates_address_bar_once() {
        let mut chrome = BasicChrome::new();
        chrome.sync_url(Some("https://example.com"));
        assert_eq!(chrome.toolbar.url_bar.state().value, "https://example.com");
        // Syncing the same URL again must not clobber user edits.
        chrome
            .toolbar
            .url_bar
            .handle_text_input(InputTextEvent::Insert("zzz".into()));
        chrome.sync_url(Some("https://example.com"));
        assert_eq!(
            chrome.toolbar.url_bar.state().value,
            "https://example.comzzz"
        );
    }

    #[test]
    fn page_clicks_fall_through_while_debug_pane_is_open() {
        let mut chrome = BasicChrome::new();
        open_debug_pane(&mut chrome);

        // The left half below the toolbar is still the browsed page; events
        // there must not be swallowed by the chrome or they would never
        // reach the active tab.
        let down = chrome.pointer_event(
            800.0,
            600.0,
            PointerEvent::Down { x: 200.0, y: 300.0 },
            ElementState::Pressed,
        );
        assert!(!down.consumed);

        let up = chrome.pointer_event(
            800.0,
            600.0,
            PointerEvent::Up { x: 200.0, y: 300.0 },
            ElementState::Released,
        );
        assert!(!up.consumed);
    }

    #[test]
    fn debug_pane_click_survives_pointer_moves_and_outside_release() {
        let mut chrome = BasicChrome::new();
        let rects = open_debug_pane(&mut chrome);
        let pane_x = 800.0 / 2.0;

        // Press inside the pane, jiggle the pointer around, and release
        // outside the pane (over the toolbar): the release belongs to the
        // pane and must not trigger the hovered chrome button.
        let down = chrome.pointer_event(
            800.0,
            600.0,
            PointerEvent::Down {
                x: pane_x + 100.0,
                y: rects.toolbar.height + 50.0,
            },
            ElementState::Pressed,
        );
        assert!(down.consumed);

        for (dx, dy) in [(1.0, 2.0), (-3.0, 1.0), (2.0, -1.0)] {
            let mv = chrome.pointer_event(
                800.0,
                600.0,
                PointerEvent::Move {
                    x: pane_x + 100.0 + dx,
                    y: rects.toolbar.height + 50.0 + dy,
                },
                ElementState::Released,
            );
            assert!(mv.consumed);
        }

        let up = chrome.pointer_event(
            800.0,
            600.0,
            PointerEvent::Up {
                x: rects.back.x + 1.0,
                y: rects.back.y + 1.0,
            },
            ElementState::Released,
        );
        assert!(up.consumed);
        assert_eq!(up.action, ChromeAction::None);
        assert!(!chrome.debug_press_active);

        // The chrome is not stuck: a normal click on Back still works.
        let action = click(&mut chrome, rects.back.x + 1.0, rects.back.y + 1.0);
        assert_eq!(action, ChromeAction::Back);
    }

    /// End-to-end fixtures driving [`BrowserUi`] exactly like the platform
    /// event loop does: pointer position bookkeeping, moves carrying a
    /// released button state, and press/release pairs through
    /// [`BrowserUi::handle_mouse_input`].
    ///
    /// Lives in this module because setup needs `BasicChrome` internals and
    /// the dispatch needs `BrowserUi` internals (both are visible to child
    /// modules of `ui`).
    mod e2e {
        use super::*;
        use crate::browser::BrowserCommand;
        use crate::browser::core::ui::{BasicContextMenu, BrowserUi, TabId};
        use std::time::Duration;

        const W: f32 = 1280.0;
        const H: f32 = 800.0;

        struct Rig {
            ui: BrowserUi,
            rects: ToolbarRects,
        }

        fn response(body: &str) -> BrowserResponse {
            BrowserResponse {
                url: String::new(),
                status: hyper::StatusCode::OK.into(),
                status_text: "OK".to_string(),
                body: body.as_bytes().to_vec(),
                headers: vec![],
            }
        }

        /// Relayouts `tab` and waits for the background layout thread, then
        /// draws again so the applied tree gets its boxes positioned.
        fn spin_layout(tab: &mut Tab, w: f32, h: f32) {
            let mut buf = Vec::new();
            tab.draw(&mut buf, w, h);
            for _ in 0..500 {
                for _ in tab.tick() {}
                if tab.layout_and_info().is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            tab.draw(&mut buf, w, h);
            assert!(tab.layout_and_info().is_some(), "layout must be ready");
        }

        /// Builds a UI whose page tab and debug pane both have loaded,
        /// positioned layouts — the state the real browser reaches before a
        /// user can click anything.
        fn rig(page_body: &str, pane_body: &str) -> Rig {
            let rects = BasicChrome::new().toolbar.rects(W);
            let toolbar_height = rects.toolbar.height;

            let mut tab = Tab::default();
            tab.navigate("https://page.test/index.html".parse().unwrap());
            tab.on_fetch_succeeded_html(format!("<html><body>{page_body}</body></html>"));
            spin_layout(&mut tab, W, H - toolbar_height);

            // Load the pane content through the chrome's own fetch pipeline.
            let mut chrome = BasicChrome::new();
            let mut actions = Vec::new();
            chrome.tick(&mut actions);
            chrome.deliver_fetch(
                FetchKind::Html,
                DEVTOOLS_URL.parse().unwrap(),
                Ok(response(&format!("<html><body>{pane_body}</body></html>"))),
            );

            let mut pane_buf = Vec::new();
            for _ in 0..500 {
                actions.clear();
                let _ = chrome.tick(&mut actions);
                if chrome.debug_pane.layout_and_info().is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(
                chrome.debug_pane.layout_and_info().is_some(),
                "pane layout must be ready"
            );
            // Position the pane boxes like a redraw would.
            chrome
                .debug_pane
                .draw(&mut pane_buf, W / 2.0, H - toolbar_height);

            let ui = BrowserUi::with_tab_and_menu(
                tab,
                Box::new(chrome),
                Box::new(BasicContextMenu::new()),
            );
            let mut ui = ui;
            ui.set_window((W as u32, H as u32), 1.0, "test".into());
            Rig { ui, rects }
        }

        fn cursor_to(rig: &mut Rig, x: f32, y: f32) {
            rig.ui.input.mouse_position = (x as f64, y as f64);
            rig.ui.handle_pointer_move(x as f64, y as f64);
        }

        fn press_left(rig: &mut Rig) -> BrowserCommand {
            rig.ui
                .handle_mouse_input(winit::event::MouseButton::Left, ElementState::Pressed)
        }

        fn release_left(rig: &mut Rig) -> BrowserCommand {
            rig.ui
                .handle_mouse_input(winit::event::MouseButton::Left, ElementState::Released)
        }

        fn open_debug_pane(rig: &mut Rig) {
            cursor_to(rig, rig.rects.devtools.x + 1.0, rig.rects.devtools.y + 1.0);
            press_left(rig);
            release_left(rig);
        }

        #[test]
        fn page_link_press_reaches_active_tab_while_pane_is_open() {
            let mut rig = rig(
                r#"<a href="https://page.test/target" style="font-size: 24px;">click me please</a>"#,
                "<p>devtools</p>",
            );
            let th = rig.rects.toolbar.height;
            open_debug_pane(&mut rig);

            // Press the link on the browsed page (left half). The press must
            // navigate the active tab; swallowing it here was bug #2.
            cursor_to(&mut rig, 30.0, th + 14.0);
            press_left(&mut rig);

            let url = rig
                .ui
                .active_tab()
                .and_then(|tab| tab.document_url().map(|u| u.to_string()));
            assert_eq!(url.as_deref(), Some("https://page.test/target"));

            release_left(&mut rig);
            let url = rig
                .ui
                .active_tab()
                .and_then(|tab| tab.document_url().map(|u| u.to_string()))
                .unwrap();
            assert_eq!(url, "https://page.test/target");
        }

        #[test]
        fn pane_link_press_routes_to_debug_pane_and_spares_the_page() {
            let mut rig = rig(
                "<p>hello</p>",
                r#"<a href="https://pane.test/target" style="font-size: 24px;">pane link</a>"#,
            );
            let th = rig.rects.toolbar.height;
            open_debug_pane(&mut rig);

            // Press the link inside the pane (right half), jiggle the
            // pointer like a real hand, then release still inside.
            cursor_to(&mut rig, W / 2.0 + 30.0, th + 14.0);
            press_left(&mut rig);

            // The pane navigated: its fetch surfaces with TabId(0).
            let mut pane_nav = false;
            for _ in 0..100 {
                let outcome = rig.ui.tick();
                pane_nav = outcome.fetches.iter().any(|fetch| {
                    fetch.tab_id == TabId(0)
                        && fetch.request.url.as_str() == "https://pane.test/target"
                });
                if pane_nav {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(pane_nav, "pane link press must navigate the pane");

            for (dx, dy) in [(2.0, 3.0), (-4.0, 1.0), (1.0, -2.0)] {
                cursor_to(&mut rig, W / 2.0 + 30.0 + dx, th + 14.0 + dy);
            }
            release_left(&mut rig);

            // None of that may leak into the browsed page.
            let url = rig
                .ui
                .active_tab()
                .and_then(|tab| tab.document_url().map(|u| u.to_string()))
                .unwrap();
            assert_eq!(url, "https://page.test/index.html");
        }

        #[test]
        fn back_button_still_works_while_pane_is_open() {
            let mut rig = rig("<p>history page</p>", "<p>devtools</p>");
            let th = rig.rects.toolbar.height;

            // Seed one history entry so Back has somewhere to go.
            if let Some(tab) = rig.ui.active_tab_mut() {
                tab.navigate("https://page.test/second.html".parse().unwrap());
                tab.on_fetch_succeeded_html("<html><body><p>second</p></body></html>".into());
                spin_layout(tab, W, H - th);
            }

            open_debug_pane(&mut rig);

            // A completed click on Back while the pane is open must navigate
            // the active tab back to the first URL.
            let (bx, by) = (rig.rects.back.x + 1.0, rig.rects.back.y + 1.0);
            cursor_to(&mut rig, bx, by);
            press_left(&mut rig);
            release_left(&mut rig);

            let url = rig
                .ui
                .active_tab()
                .and_then(|tab| tab.document_url().map(|u| u.to_string()));
            assert_eq!(url.as_deref(), Some("https://page.test/index.html"));
        }
    }
}
