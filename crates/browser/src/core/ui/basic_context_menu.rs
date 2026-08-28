//! Default context menu: a vertical list of actions shown where the user
//! right-clicked the web content.
//!
//! This is the stock [`ContextMenu`] implementation installed by the browser
//! core. It is deliberately simple and hardcoded; the core only talks to it
//! through the [`ContextMenu`] trait, so it can be replaced by any
//! user-designed menu.
//!
//! The menu is drawn with direct [`DrawCommand`]s (background, border, one
//! row per item), exactly like the engine draws `<select>` popups. A row is
//! armed by pressing it and selected by releasing over the same row; any
//! press outside the menu dismisses it without running an action.

use std::cell::Cell;
use std::sync::Arc;

use crate::browser::core::ui::chrome::ChromeAction;
use crate::browser::core::ui::context_menu::{ClickContext, ContextMenu, MenuEventResult};
use crate::platform::renderer::text_measurer::PlatformTextMeasurer;
use engine::bridge::text::{TextAttribute, TextMeasureRequest, TextMeasurer};
use engine::layouter::types::{Color, TextFlowStyle, TextStyle};
use engine::renderer_model::{Brush, DrawCommand, FillRule, Paint, Rect, rect_path};
use engine::ui::PointerEvent;

/// Height of each menu row.
const ROW_HEIGHT: f32 = 28.0;
/// Minimum menu width when nothing can be measured.
const MIN_WIDTH: f32 = 180.0;
/// Left/right inset of the row labels.
const INLINE_PADDING: f32 = 14.0;
/// Menu border color.
const BORDER_COLOR: Color = Color(150, 150, 150, 255);
/// Menu background color.
const BACKGROUND: Color = Color(255, 255, 255, 255);
/// Background of the row under the cursor.
const HIGHLIGHT_BG: Color = Color(209, 231, 255, 255);
/// Label color.
const LABEL_COLOR: Color = Color(20, 20, 20, 255);

/// One selectable entry of the default context menu.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    /// The entry's visible label.
    pub label: String,
    /// The action reported to the browser core when the entry is selected.
    pub action: ChromeAction,
}

impl MenuItem {
    /// Creates an entry from a label and an action.
    pub fn new(label: impl Into<String>, action: ChromeAction) -> Self {
        Self {
            label: label.into(),
            action,
        }
    }
}

/// The default [`ContextMenu`] for a browser window: a vertical list of
/// actions shown at the right-click position.
pub struct BasicContextMenu {
    items: Vec<MenuItem>,
    measurer: Arc<dyn TextMeasurer>,
    open: bool,
    /// Requested top-left corner in window coordinates. Clamped into the
    /// window bounds whenever the menu is drawn or hit-tested.
    origin: (f32, f32),
    /// Row currently under the pointer, if any.
    hovered: Option<usize>,
    /// Row armed by a press; selection completes on release over this row.
    pressed_row: Option<usize>,
    dirty: Cell<bool>,
}

impl std::fmt::Debug for BasicContextMenu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BasicContextMenu")
            .field("items", &self.items)
            .field("open", &self.open)
            .field("origin", &self.origin)
            .field("hovered", &self.hovered)
            .field("pressed_row", &self.pressed_row)
            .finish_non_exhaustive()
    }
}

impl BasicContextMenu {
    /// Creates a menu with the default entries (`Back` / `Reload`).
    pub fn new() -> Self {
        Self::with_items(Self::default_items())
    }

    /// Creates a menu with custom entries.
    ///
    /// An empty list never opens: [`open`](Self::open) declines the request.
    pub fn with_items(items: Vec<MenuItem>) -> Self {
        let measurer: Arc<dyn TextMeasurer> = Arc::new(PlatformTextMeasurer::new().unwrap());
        Self::with_items_and_measurer(items, measurer)
    }

    fn with_items_and_measurer(items: Vec<MenuItem>, measurer: Arc<dyn TextMeasurer>) -> Self {
        Self {
            items,
            measurer,
            open: false,
            origin: (0.0, 0.0),
            hovered: None,
            pressed_row: None,
            dirty: Cell::new(true),
        }
    }

    /// The stock entries: navigate back and reload the page.
    fn default_items() -> Vec<MenuItem> {
        vec![
            MenuItem::new("← Back", ChromeAction::Back),
            MenuItem::new("⟳ Reload", ChromeAction::Reload),
        ]
    }

    /// Measured width of a single label.
    fn label_width(&self, label: &str) -> f32 {
        self.measurer
            .measure(&TextMeasureRequest {
                text: label.to_string(),
                attribute: TextAttribute {
                    style: TextStyle {
                        color: LABEL_COLOR,
                        ..TextStyle::default()
                    },
                    flow_style: TextFlowStyle::default(),
                },
            })
            .map(|fragments| fragments.iter().map(|f| f.width).sum())
            .unwrap_or(0.0)
    }

    /// Widest label plus horizontal padding.
    fn menu_width(&self) -> f32 {
        self.items
            .iter()
            .map(|item| self.label_width(&item.label))
            .fold(MIN_WIDTH, f32::max)
            + INLINE_PADDING * 2.0
    }

    /// Full menu size as `(width, height)`.
    fn size(&self) -> (f32, f32) {
        (self.menu_width(), self.items.len() as f32 * ROW_HEIGHT)
    }

    /// Top-left corner clamped so the menu stays inside `(width, height)`.
    fn clamped_origin(&self, width: f32, height: f32) -> (f32, f32) {
        let (w, h) = self.size();
        (
            self.origin.0.clamp(0.0, (width - w).max(0.0)),
            self.origin.1.clamp(0.0, (height - h).max(0.0)),
        )
    }

    /// The menu rect actually drawn / hit-tested for a given window size.
    fn rect(&self, width: f32, height: f32) -> Rect {
        let (ox, oy) = self.clamped_origin(width, height);
        let (w, h) = self.size();
        Rect::new(ox, oy, w, h)
    }

    /// The row under a window-space point, or `None` when outside the menu.
    fn row_at(&self, width: f32, height: f32, x: f32, y: f32) -> Option<usize> {
        let rect = self.rect(width, height);
        if !rect.contains(x, y) {
            return None;
        }
        Some(((y - rect.y) / ROW_HEIGHT) as usize)
    }

    fn set_hovered(&mut self, hovered: Option<usize>) {
        if self.hovered != hovered {
            self.hovered = hovered;
            self.dirty.set(true);
        }
    }

    /// Draws the menu background and border at `rect`.
    fn push_frame(&self, cmd_buf: &mut Vec<DrawCommand>, rect: &Rect) {
        let paint = |color| Paint {
            brush: Brush::Solid(color),
            opacity: 1.0,
        };
        cmd_buf.push(DrawCommand::Fill {
            path: rect_path(rect.x, rect.y, rect.width, rect.height),
            rule: FillRule::NonZero,
            paint: paint(BACKGROUND),
        });
        for path in [
            rect_path(rect.x, rect.y, rect.width, 1.0),
            rect_path(rect.x, rect.y + rect.height - 1.0, rect.width, 1.0),
            rect_path(rect.x, rect.y, 1.0, rect.height),
            rect_path(rect.x + rect.width - 1.0, rect.y, 1.0, rect.height),
        ] {
            cmd_buf.push(DrawCommand::Fill {
                path,
                rule: FillRule::NonZero,
                paint: paint(BORDER_COLOR),
            });
        }
    }
}

impl Default for BasicContextMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextMenu for BasicContextMenu {
    fn open(&mut self, ctx: &ClickContext) -> bool {
        if self.items.is_empty() {
            return false;
        }
        self.open = true;
        self.origin = ctx.window_pos;
        self.hovered = None;
        self.pressed_row = None;
        self.dirty.set(true);
        true
    }

    fn close(&mut self) {
        if self.open {
            self.open = false;
            self.hovered = None;
            self.pressed_row = None;
            self.dirty.set(true);
        }
    }

    fn is_open(&self) -> bool {
        self.open
    }

    fn draw(&self, cmd_buf: &mut Vec<DrawCommand>, width: f32, height: f32) {
        if !self.open {
            return;
        }
        let rect = self.rect(width, height);
        self.push_frame(cmd_buf, &rect);

        let text_style = TextStyle {
            color: LABEL_COLOR,
            ..TextStyle::default()
        };
        let flow_style = TextFlowStyle::default();
        let font_size = flow_style.font_size;

        for (i, item) in self.items.iter().enumerate() {
            let y = rect.y + i as f32 * ROW_HEIGHT;
            if self.hovered == Some(i) || self.pressed_row == Some(i) {
                cmd_buf.push(DrawCommand::Fill {
                    path: rect_path(rect.x, y, rect.width, ROW_HEIGHT),
                    rule: FillRule::NonZero,
                    paint: Paint {
                        brush: Brush::Solid(HIGHLIGHT_BG),
                        opacity: 1.0,
                    },
                });
            }
            cmd_buf.push(DrawCommand::DrawText {
                x: rect.x + INLINE_PADDING,
                y: y + ((ROW_HEIGHT - font_size) * 0.5).max(0.0),
                text: item.label.clone().into(),
                style: text_style.clone(),
                flow_style,
            });
        }
    }

    fn pointer_event(&mut self, width: f32, height: f32, event: PointerEvent) -> MenuEventResult {
        if !self.open {
            return MenuEventResult::none();
        }

        match event {
            PointerEvent::Move { x, y } => {
                self.set_hovered(self.row_at(width, height, x, y));
                // While open the menu owns the pointer: no page hover updates.
                MenuEventResult::consumed(ChromeAction::None)
            }
            PointerEvent::Down { x, y } => {
                let row = self.row_at(width, height, x, y);
                match row {
                    // Arm the pressed row; selection happens on release.
                    Some(row) => {
                        self.set_hovered(Some(row));
                        self.pressed_row = Some(row);
                        self.dirty.set(true);
                    }
                    // A press outside the menu dismisses it. The click is
                    // consumed so it does not reach the page underneath.
                    None => self.close(),
                }
                MenuEventResult::consumed(ChromeAction::None)
            }
            PointerEvent::Up { x, y } => {
                // Selecting requires a press armed on a row (a fresh click
                // after the menu opened). The release that completes the
                // opening right-press arrives with no armed row and must not
                // close the menu.
                let pressed = self.pressed_row.take();
                match pressed.filter(|row| Some(*row) == self.row_at(width, height, x, y)) {
                    Some(row) => {
                        let action = self.items[row].action.clone();
                        self.close();
                        MenuEventResult::consumed(action)
                    }
                    None => MenuEventResult::consumed(ChromeAction::None),
                }
            }
            PointerEvent::Leave => {
                self.set_hovered(None);
                self.pressed_row = None;
                MenuEventResult::consumed(ChromeAction::None)
            }
        }
    }

    fn needs_repaint(&self) -> bool {
        self.dirty.replace(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::bridge::text::FallbackTextMeasurer;

    const WINDOW_WIDTH: f32 = 800.0;
    const WINDOW_HEIGHT: f32 = 600.0;

    fn menu() -> BasicContextMenu {
        BasicContextMenu::with_items_and_measurer(
            vec![
                MenuItem::new("Back", ChromeAction::Back),
                MenuItem::new("Reload", ChromeAction::Reload),
            ],
            Arc::new(FallbackTextMeasurer),
        )
    }

    /// A click context at window position `(x, y)` (page info unused here).
    fn ctx(x: f32, y: f32) -> ClickContext {
        ClickContext {
            window_pos: (x, y),
            page_pos: (x, y),
            link_url: None,
            document_url: None,
        }
    }

    /// Window-space center of the given row when the menu opened at `(100, 100)`.
    fn row_center(row: usize) -> (f32, f32) {
        (
            100.0 + MIN_WIDTH / 2.0 + INLINE_PADDING,
            100.0 + row as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0,
        )
    }

    #[test]
    fn opens_at_click_position() {
        let mut menu = menu();
        assert!(!menu.is_open());
        assert!(menu.open(&ctx(100.0, 100.0)));
        assert!(menu.is_open());

        let mut cmd_buf = Vec::new();
        menu.draw(&mut cmd_buf, WINDOW_WIDTH, WINDOW_HEIGHT);
        // Background, border strips and one text per item.
        assert!(
            cmd_buf
                .iter()
                .any(|cmd| matches!(cmd, DrawCommand::Fill { .. }))
        );
        assert_eq!(
            cmd_buf
                .iter()
                .filter(|cmd| matches!(cmd, DrawCommand::DrawText { .. }))
                .count(),
            2
        );
        assert!(cmd_buf.iter().any(|cmd| matches!(
            cmd,
            DrawCommand::DrawText { text, .. } if text == "Back"
        )));
    }

    #[test]
    fn empty_items_decline_open() {
        let mut empty =
            BasicContextMenu::with_items_and_measurer(Vec::new(), Arc::new(FallbackTextMeasurer));
        assert!(!empty.open(&ctx(100.0, 100.0)));
        assert!(!empty.is_open());
    }

    #[test]
    fn press_and_release_on_row_selects_action_and_closes() {
        let mut menu = menu();
        menu.open(&ctx(100.0, 100.0));

        let (x, y) = row_center(0);
        let result = menu.pointer_event(WINDOW_WIDTH, WINDOW_HEIGHT, PointerEvent::Down { x, y });
        assert!(result.consumed);
        assert_eq!(result.action, ChromeAction::None);
        assert!(menu.is_open(), "menu must stay open until release");

        let result = menu.pointer_event(WINDOW_WIDTH, WINDOW_HEIGHT, PointerEvent::Up { x, y });
        assert!(result.consumed);
        assert_eq!(result.action, ChromeAction::Back);
        assert!(!menu.is_open());
    }

    #[test]
    fn release_on_different_row_keeps_menu_open_without_action() {
        let mut menu = menu();
        menu.open(&ctx(100.0, 100.0));

        let down = row_center(0);
        let up = row_center(1);
        menu.pointer_event(
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            PointerEvent::Down {
                x: down.0,
                y: down.1,
            },
        );

        let result = menu.pointer_event(
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            PointerEvent::Up { x: up.0, y: up.1 },
        );
        assert!(result.consumed);
        assert_eq!(result.action, ChromeAction::None);
        assert!(
            menu.is_open(),
            "a release that misses the armed row must not select or close"
        );
    }

    #[test]
    fn release_of_the_opening_press_neither_selects_nor_closes() {
        let mut menu = menu();
        menu.open(&ctx(100.0, 100.0));

        // The right-button release that completes the opening press.
        let result = menu.pointer_event(
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            PointerEvent::Up {
                x: row_center(0).0,
                y: row_center(0).1,
            },
        );
        assert!(result.consumed);
        assert_eq!(result.action, ChromeAction::None);
        assert!(
            menu.is_open(),
            "the menu must stay open after opening press"
        );

        // A fresh click on a row then selects it.
        let (x, y) = row_center(1);
        menu.pointer_event(WINDOW_WIDTH, WINDOW_HEIGHT, PointerEvent::Down { x, y });
        let result = menu.pointer_event(WINDOW_WIDTH, WINDOW_HEIGHT, PointerEvent::Up { x, y });
        assert_eq!(result.action, ChromeAction::Reload);
        assert!(!menu.is_open());
    }

    #[test]
    fn outside_press_dismisses_and_consumes() {
        let mut menu = menu();
        menu.open(&ctx(100.0, 100.0));

        let result = menu.pointer_event(
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            PointerEvent::Down { x: 10.0, y: 10.0 },
        );
        assert!(
            result.consumed,
            "the dismissing press must not reach the page"
        );
        assert_eq!(result.action, ChromeAction::None);
        assert!(!menu.is_open());
    }

    #[test]
    fn move_updates_hover_and_outside_clears_it() {
        let mut menu = menu();
        menu.open(&ctx(100.0, 100.0));
        let (x, y) = row_center(1);

        menu.pointer_event(WINDOW_WIDTH, WINDOW_HEIGHT, PointerEvent::Move { x, y });
        assert_eq!(menu.hovered, Some(1));

        menu.pointer_event(
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            PointerEvent::Move { x: 5.0, y: 5.0 },
        );
        assert_eq!(menu.hovered, None);
    }

    #[test]
    fn closed_menu_draws_nothing_and_ignores_events() {
        let mut menu = menu();

        let mut cmd_buf = Vec::new();
        menu.draw(&mut cmd_buf, WINDOW_WIDTH, WINDOW_HEIGHT);
        assert!(cmd_buf.is_empty());

        let result = menu.pointer_event(
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            PointerEvent::Down { x: 100.0, y: 100.0 },
        );
        assert_eq!(result, MenuEventResult::none());
    }

    #[test]
    fn origin_is_clamped_into_window_bounds() {
        let mut menu = menu();
        menu.open(&ctx(5000.0, 5000.0));

        let mut cmd_buf = Vec::new();
        menu.draw(&mut cmd_buf, WINDOW_WIDTH, WINDOW_HEIGHT);

        for rect in cmd_buf.iter().filter_map(|cmd| match cmd {
            DrawCommand::Fill { path, .. } => path.bounding_box(),
            _ => None,
        }) {
            assert!(rect.x >= 0.0 && rect.y >= 0.0);
            assert!(rect.x + rect.width <= WINDOW_WIDTH + 0.001);
            assert!(rect.y + rect.height <= WINDOW_HEIGHT + 0.001);
        }
    }

    #[test]
    fn needs_repaint_flag_is_consumed_once() {
        let mut menu = menu();
        // Consume the flag set by construction.
        assert!(menu.needs_repaint());

        menu.open(&ctx(10.0, 10.0));
        assert!(menu.needs_repaint());
        assert!(!menu.needs_repaint());
    }

    #[test]
    fn close_without_action_resets_state() {
        let mut menu = menu();
        menu.open(&ctx(100.0, 100.0));
        let (x, y) = row_center(0);
        menu.pointer_event(WINDOW_WIDTH, WINDOW_HEIGHT, PointerEvent::Down { x, y });
        assert_eq!(menu.pressed_row, Some(0));

        menu.close();
        assert!(!menu.is_open());
        assert_eq!(menu.pressed_row, None);
        assert_eq!(menu.hovered, None);
    }
}
