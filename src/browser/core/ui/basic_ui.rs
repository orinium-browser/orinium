// src/browser/core/ui/basic_ui.rs
//! Basic browser UI layout components.
//!
//! This module defines the browser chrome: a toolbar with a back button, a
//! reload button, and a URL bar, laid out above the page content area.
//!
//! The chrome is drawn with direct [`DrawCommand`]s (a fixed row layout, not a
//! full `ui_layout` tree): each component draws its own background and content
//! inside a translated coordinate system, exactly as the engine does for
//! replaced elements.

use std::rc::Rc;
use std::sync::Arc;

use ui_layout::Style;

use crate::engine::bridge::text::TextMeasurer;
use crate::engine::layouter::types::{Background, Color, TextStyle};
use crate::engine::renderer_model::{
    AffineTransform, Brush, DrawCommand, FillRule, Paint, Rect, rect_path,
};
use crate::engine::ui::button::ButtonComponent;
use crate::engine::ui::custom_node::{ContentSize, CustomNode, PointerEvent};
use crate::engine::ui::text_input::TextInputComponent;
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
pub enum ChromeHit {
    /// Back navigation button.
    Back,
    /// Reload current page button.
    Reload,
    /// URL entry bar.
    UrlBar,
}

/// Rectangles of the toolbar elements for a given window width.
#[derive(Debug, Clone, Copy)]
pub struct ToolbarRects {
    /// The whole toolbar strip (top edge of the window).
    pub toolbar: Rect,
    /// Back button.
    pub back: Rect,
    /// Reload button.
    pub reload: Rect,
    /// URL bar.
    pub url_bar: Rect,
}

impl ToolbarRects {
    /// Height of the toolbar strip in logical pixels.
    pub fn height(&self) -> f32 {
        self.toolbar.height
    }
}

/// Placeholder content node that renders nothing.
///
/// Will be replaced with a node that forwards draw commands from the engine's
/// renderer (a future web-view-as-custom-node integration).
#[derive(Debug)]
struct ContentPlaceholder;

impl CustomNode for ContentPlaceholder {
    fn draw_sized(
        &self,
        _cmd_buf: &mut Vec<DrawCommand>,
        _text_style: &TextStyle,
        _style: &Style,
        _size: ContentSize,
    ) {
    }

    fn intrinsic_size(&self) -> ContentSize {
        ContentSize::zero()
    }
}

/// Represents the top toolbar of the browser.
#[derive(Debug)]
pub struct BrowserToolbar {
    /// Back navigation button.
    pub back_button: ButtonComponent,
    /// Reload current page button.
    pub reload_button: ButtonComponent,
    /// URL entry bar.
    pub url_bar: TextInputComponent,
}

impl BrowserToolbar {
    /// Create a new toolbar with placeholder components.
    pub fn new() -> Self {
        let measurer: Arc<dyn TextMeasurer<TextStyle>> = Arc::new(PlatformTextMeasurer);
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
        let url_bar = TextInputComponent::new("", "Enter URL", measurer);
        Self {
            back_button,
            reload_button,
            url_bar,
        }
    }

    /// Computes the layout of the toolbar row for the given window width.
    pub fn rects(&self, width: f32) -> ToolbarRects {
        let back_size = self.back_button.intrinsic_size();
        let reload_size = self.reload_button.intrinsic_size();
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
        let url_x = (reload.x + reload.width + CHROME_GAP).min(width - CHROME_PADDING);
        let url_width = (width - CHROME_PADDING - url_x).max(0.0);
        let url_bar = Rect::new(url_x, center_y(url_size.height), url_width, url_size.height);

        ToolbarRects {
            toolbar: Rect::new(0.0, 0.0, width, row_height + CHROME_PADDING * 2.0),
            back,
            reload,
            url_bar,
        }
    }

    /// Returns the toolbar element under `(x, y)`, or `None` when the point is
    /// outside the chrome (i.e. over the page content).
    pub fn hit_test(&self, x: f32, y: f32, width: f32) -> Option<ChromeHit> {
        let rects = self.rects(width);
        if rects.back.contains(x, y) {
            Some(ChromeHit::Back)
        } else if rects.reload.contains(x, y) {
            Some(ChromeHit::Reload)
        } else if rects.url_bar.contains(x, y) {
            Some(ChromeHit::UrlBar)
        } else {
            None
        }
    }
}

impl Default for BrowserToolbar {
    fn default() -> Self {
        Self::new()
    }
}

/// The complete UI for a browser window.
///
/// It consists of a top toolbar and a content area where the web view is drawn.
#[derive(Debug)]
pub struct BrowserLayout {
    /// Toolbar with navigation controls.
    pub toolbar: BrowserToolbar,
    /// Placeholder for the rendered page – will hold a custom node that forwards
    /// draw commands from the engine's renderer.
    pub content_node: Rc<dyn CustomNode>,
    /// URL currently shown in the address bar, used to avoid overwriting text
    /// the user is editing.
    last_url: Option<String>,
}

impl BrowserLayout {
    /// Create a new UI instance with default toolbar components and an empty
    /// content node.
    pub fn new() -> Self {
        Self {
            toolbar: BrowserToolbar::new(),
            content_node: Rc::new(ContentPlaceholder),
            last_url: None,
        }
    }

    /// Toolbar rectangles for the given window width.
    pub fn toolbar_rects(&self, width: f32) -> ToolbarRects {
        self.toolbar.rects(width)
    }

    /// Content area below the toolbar for the given window size.
    pub fn content_rect(&self, width: f32, height: f32) -> Rect {
        let toolbar = self.toolbar.rects(width);
        Rect::new(
            0.0,
            toolbar.height(),
            width,
            (height - toolbar.height()).max(0.0),
        )
    }

    /// Returns the toolbar element under `(x, y)`, or `None` when the point is
    /// over the page content area.
    pub fn hit_test(&self, x: f32, y: f32, width: f32) -> Option<ChromeHit> {
        self.toolbar.hit_test(x, y, width)
    }

    /// Updates the URL shown in the address bar when the active tab navigates.
    ///
    /// The address bar is only rewritten when the displayed URL actually
    /// changes, so text the user is typing is not clobbered by a redraw.
    pub fn sync_url(&mut self, url: Option<&str>) {
        let url = url.map(str::to_string);
        if self.last_url != url {
            self.last_url.clone_from(&url);
            self.toolbar.url_bar.set_value(url.unwrap_or_default());
        }
    }

    /// Appends the toolbar chrome draw commands for the given window width.
    pub fn draw_chrome(&self, cmd_buf: &mut Vec<DrawCommand>, width: f32) {
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

        let components: [(&dyn CustomNode, Rect); 3] = [
            (&self.toolbar.back_button, rects.back),
            (&self.toolbar.reload_button, rects.reload),
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
    }

    /// Dispatches a pointer event to the toolbar element under the hit target.
    ///
    /// Returns `(consumed, clicked)`: `consumed` is true when the component
    /// handled the event, and `clicked` is true for an `Up` event that
    /// completes a click (a `Down` followed by an `Up` on the same element).
    pub fn handle_pointer_event(&self, hit: ChromeHit, event: PointerEvent) -> (bool, bool) {
        let node: &dyn CustomNode = match hit {
            ChromeHit::Back => &self.toolbar.back_button,
            ChromeHit::Reload => &self.toolbar.reload_button,
            ChromeHit::UrlBar => &self.toolbar.url_bar,
        };
        let handled = node.on_pointer_event(event);
        match event {
            PointerEvent::Up { .. } => (handled, handled),
            _ => (handled, false),
        }
    }

    /// Returns whether any toolbar component changed its visual state since the
    /// last check (consumes the dirty flags).
    pub fn toolbar_needs_repaint(&self) -> bool {
        self.toolbar.back_button.needs_repaint()
            || self.toolbar.reload_button.needs_repaint()
            || self.toolbar.url_bar.needs_repaint()
    }
}

impl Default for BrowserLayout {
    fn default() -> Self {
        Self::new()
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
    fn hit_test_finds_buttons_and_url_bar() {
        let layout = BrowserLayout::new();
        let rects = layout.toolbar_rects(800.0);

        assert_eq!(
            layout.hit_test(rects.back.x + 1.0, rects.back.y + 1.0, 800.0),
            Some(ChromeHit::Back)
        );
        assert_eq!(
            layout.hit_test(rects.reload.x + 1.0, rects.reload.y + 1.0, 800.0),
            Some(ChromeHit::Reload)
        );
        assert_eq!(
            layout.hit_test(rects.url_bar.x + 1.0, rects.url_bar.y + 1.0, 800.0),
            Some(ChromeHit::UrlBar)
        );
        // Below the toolbar is the page content area.
        assert_eq!(
            layout.hit_test(400.0, rects.toolbar.height + 10.0, 800.0),
            None
        );
    }

    #[test]
    fn draw_chrome_emits_background_and_components() {
        let layout = BrowserLayout::new();
        let mut commands = Vec::new();
        layout.draw_chrome(&mut commands, 800.0);

        // Toolbar background fill.
        assert!(matches!(commands[0], DrawCommand::Fill { .. }));
        // Each component draws a background fill, then a transform pair.
        let fills = commands
            .iter()
            .filter(|c| matches!(c, DrawCommand::Fill { .. }))
            .count();
        let transforms = commands
            .iter()
            .filter(|c| matches!(c, DrawCommand::PushTransform { .. }))
            .count();
        assert_eq!(fills, 4, "toolbar background + 3 component backgrounds");
        assert_eq!(transforms, 3);
        assert_eq!(
            commands
                .iter()
                .filter(|c| matches!(c, DrawCommand::PopTransform))
                .count(),
            3
        );
    }

    #[test]
    fn sync_url_updates_address_bar_once() {
        let mut layout = BrowserLayout::new();
        layout.sync_url(Some("https://example.com"));
        assert_eq!(layout.toolbar.url_bar.state().value, "https://example.com");
        // Syncing the same URL again must not clobber user edits.
        layout.toolbar.url_bar.handle_text_input(
            crate::engine::ui::text_input_types::TextInputEvent::Insert("zzz".into()),
        );
        layout.sync_url(Some("https://example.com"));
        assert_eq!(
            layout.toolbar.url_bar.state().value,
            "https://example.comzzz"
        );
    }

    #[test]
    fn pointer_down_then_up_reports_back_click() {
        let layout = BrowserLayout::new();
        assert_eq!(
            layout.handle_pointer_event(ChromeHit::Back, PointerEvent::Down { x: 1.0, y: 1.0 }),
            (true, false)
        );
        let (consumed, clicked) =
            layout.handle_pointer_event(ChromeHit::Back, PointerEvent::Up { x: 1.0, y: 1.0 });
        assert!(consumed && clicked);
    }
}
