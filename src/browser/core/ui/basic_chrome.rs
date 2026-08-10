// src/browser/core/ui/basic_chrome.rs
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
use winit::event::{Ime, KeyEvent};

use crate::browser::core::ui::chrome::{Chrome, ChromeAction, ChromeEventResult};
use crate::browser::core::ui::{logical_key_to_special_event, logical_key_to_text_key};
use crate::engine::bridge::text::TextMeasurer;
use crate::engine::layouter::types::{Background, Color, TextStyle};
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
    /// Debug tool button (dumps the current layout result).
    Debug,
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
    /// Debug tool button.
    debug: Rect,
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
    /// Show the current LayoutNode
    debug_button: ButtonComponent,
    /// URL entry bar.
    url_bar: InputTextComponent,
}

impl BrowserToolbar {
    /// Create a new toolbar with placeholder components.
    fn new() -> Self {
        let measurer: Arc<dyn TextMeasurer<TextStyle>> =
            Arc::new(PlatformTextMeasurer::new().unwrap());
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
        let debug_button = ButtonComponent::new(
            "Debug",
            BUTTON_BACKGROUND,
            LABEL_COLOR,
            Arc::clone(&measurer),
        );
        let url_bar = InputTextComponent::new("", "Enter URL", measurer);
        Self {
            back_button,
            reload_button,
            debug_button,
            url_bar,
        }
    }

    /// Computes the layout of the toolbar row for the given window width.
    fn rects(&self, width: f32) -> ToolbarRects {
        let back_size = self.back_button.intrinsic_size();
        let reload_size = self.reload_button.intrinsic_size();
        let debug_size = self.debug_button.intrinsic_size();
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
        let debug = Rect::new(
            reload.x + reload.width + CHROME_GAP,
            center_y(debug_size.height),
            debug_size.width,
            debug_size.height,
        );
        let url_x = (debug.x + debug.width + CHROME_GAP).min(width - CHROME_PADDING);
        let url_width = (width - CHROME_PADDING - url_x).max(0.0);
        let url_bar = Rect::new(url_x, center_y(url_size.height), url_width, url_size.height);

        ToolbarRects {
            toolbar: Rect::new(0.0, 0.0, width, row_height + CHROME_PADDING * 2.0),
            back,
            reload,
            debug,
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
        } else if rects.debug.contains(x, y) {
            Some(ChromeHit::Debug)
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
    /// Wheather
    is_debug: bool,
    /// String content from LayoutNode.
    ///
    /// (debug info, scroll x, scroll y)
    debug_layout_node: (String, f32, f32),
    /// Toolbar element currently under the pointer, if any.
    hovered: Option<ChromeHit>,
}

impl BasicChrome {
    /// Create a new default chrome with an empty toolbar.
    pub fn new() -> Self {
        Self {
            toolbar: BrowserToolbar::new(),
            last_url: None,
            is_debug: false,
            debug_layout_node: (String::new(), 0.0, 0.0),
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
            ChromeHit::Debug => &self.toolbar.debug_button,
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
        if self.is_debug {
            Rect::new(0.0, toolbar_height, width / 2.0, height - toolbar_height)
        } else {
            Rect::new(0.0, toolbar_height, width, height - toolbar_height)
        }
    }

    fn draw(&self, cmd_buf: &mut Vec<DrawCommand>, width: f32, height: f32) {
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
        let style = Style::default();

        let components: [(&dyn CustomNode, Rect); 4] = [
            (&self.toolbar.back_button, rects.back),
            (&self.toolbar.reload_button, rects.reload),
            (&self.toolbar.debug_button, rects.debug),
            (&self.toolbar.url_bar, rects.url_bar),
        ];

        for (node, rect) in components {
            if let Some(Background::Color(color)) = node.background() {
                cmd_buf.push(DrawCommand::Fill {
                    path: rect_path(rect.x, rect.y, rect.width, rect.height),
                    rule: FillRule::NonZero,
                    paint: Paint {
                        brush: Brush::Solid(color),
                        opacity: 1.0,
                    },
                });
            }
            cmd_buf.push(DrawCommand::PushTransform {
                transform: AffineTransform::translate(rect.x, rect.y),
            });
            node.draw_sized(
                cmd_buf,
                &text_style,
                &style,
                ContentSize {
                    width: rect.width,
                    height: rect.height,
                },
            );
            cmd_buf.push(DrawCommand::PopTransform);
        }

        if self.is_debug {
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

            cmd_buf.push(DrawCommand::DrawText {
                x: self.debug_layout_node.1,
                y: self.debug_layout_node.2,
                text: self.debug_layout_node.0.clone().into(),
                style: text_style,
            });

            cmd_buf.push(DrawCommand::PopTransform);
            cmd_buf.push(DrawCommand::PopClip);
        }
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
        _height: f32,
        event: PointerEvent,
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
            // Pointer over the page: clear any chrome hover.
            self.clear_hover();
            return ChromeEventResult::none();
        };

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
            ChromeHit::Debug if clicked => {
                if self.is_debug {
                    self.is_debug = false
                } else {
                    self.is_debug = true
                }
                ChromeAction::DumpLayoutNode
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
        x: f32,
        y: f32,
        scroll_x: f32,
        scroll_y: f32,
    ) {
        if self.is_debug {
            let rects = self.toolbar.rects(width);

            let rect = Rect {
                x: width / 2.0,
                y: rects.height(),
                width: width / 2.0,
                height: height - rects.height(),
            };

            if rect.contains(x, y) {
                self.debug_layout_node.1 -= scroll_x;
                self.debug_layout_node.2 -= scroll_y;
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

    fn debug_set_layout_node(&mut self, node: &ui_layout::LayoutNode) {
        if self.is_debug {
            self.debug_layout_node.0 = format!("{:#}", node);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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
        );
        assert!(result.consumed);

        let result = chrome.pointer_event(
            800.0,
            600.0,
            PointerEvent::Move {
                x: rects.reload.x + 1.0,
                y: rects.reload.y + 1.0,
            },
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
}
