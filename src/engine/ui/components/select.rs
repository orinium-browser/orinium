//! Dropdown component for the HTML `<select>` element.
//!
//! The control renders like a combo box: the currently selected option's
//! label plus a drop-down arrow. Clicking the box opens a popup (top-layer
//! overlay) listing every option; clicking a row selects it and reports the
//! new value through the DOM write-back channel.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use ui_layout::Style;

use crate::engine::bridge::text::{self, TextMeasureRequest};
use crate::engine::layouter::types::{Background, Color, FontWeight, TextStyle};
use crate::engine::renderer_model::{Brush, DrawCommand, FillRule, Paint, Path, Rect, rect_path};
use crate::engine::ui::custom_node::{ContentSize, CustomNode, PointerEvent, Popup};

/// Row height of the box and of each dropdown option.
const ROW_HEIGHT: f32 = 28.0;
/// Minimum select width when nothing can be measured.
const MIN_WIDTH: f32 = 120.0;
/// Left/right inset of the box label and option rows.
const INLINE_PADDING: f32 = 6.0;
/// Width reserved for the drop-down arrow.
const ARROW_WIDTH: f32 = 24.0;
/// Box border color.
const BORDER_COLOR: Color = Color(150, 150, 150, 255);
/// Popup background color.
const POPUP_BG: Color = Color(255, 255, 255, 255);
/// Background of the option row under the cursor.
const HIGHLIGHT_BG: Color = Color(209, 231, 255, 255);

/// One `<option>` inside a `<select>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectOption {
    /// The option's `value` attribute (falls back to its text content).
    pub value: String,
    /// The option's visible label.
    pub label: String,
    /// Whether the option carries the `selected` attribute.
    pub selected: bool,
}

/// Callback invoked when the select's value changes.
pub type OnSelectChange = dyn Fn(&str) + Send + Sync;

/// An HTML `<select>` rendered by the engine.
pub struct SelectComponent {
    options: Vec<SelectOption>,
    measurer: Arc<dyn text::TextMeasurer<TextStyle>>,
    selected: Mutex<usize>,
    open: AtomicBool,
    hovered: AtomicBool,
    hover_index: AtomicI32,
    dirty: AtomicBool,
    last_size: Mutex<ContentSize>,
    on_change: Option<Arc<OnSelectChange>>,
}

impl std::fmt::Debug for SelectComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectComponent")
            .field("options", &self.options)
            .field("selected", &self.selected.lock().unwrap())
            .field("open", &self.open)
            .finish_non_exhaustive()
    }
}

impl SelectComponent {
    /// Creates a select with the given options. `value` is the element's
    /// `value` attribute; the matching option (or the first `selected` one,
    /// falling back to the first option) is initially selected.
    pub fn new(
        options: Vec<SelectOption>,
        value: &str,
        measurer: Arc<dyn text::TextMeasurer<TextStyle>>,
    ) -> Self {
        Self {
            selected: Mutex::new(initial_index(&options, value)),
            options,
            measurer,
            open: AtomicBool::new(false),
            hovered: AtomicBool::new(false),
            hover_index: AtomicI32::new(-1),
            dirty: AtomicBool::new(true),
            last_size: Mutex::new(ContentSize::zero()),
            on_change: None,
        }
    }

    /// Creates a select with a value-change callback for DOM sync.
    pub fn with_on_change(
        options: Vec<SelectOption>,
        value: &str,
        measurer: Arc<dyn text::TextMeasurer<TextStyle>>,
        on_change: Arc<OnSelectChange>,
    ) -> Self {
        let mut select = Self::new(options, value, measurer);
        select.on_change = Some(on_change);
        select
    }

    /// Returns the currently selected option's value.
    pub fn selected_value(&self) -> String {
        let options = &self.options;
        let index = *self.selected.lock().unwrap();
        options
            .get(index)
            .map_or_else(String::new, |option| option.value.clone())
    }

    fn toggle(&self) {
        let opened = !self.open.load(Ordering::Relaxed);
        self.open.store(opened, Ordering::Relaxed);
        if opened {
            self.hover_index.store(-1, Ordering::Relaxed);
        }
        self.dirty.store(true, Ordering::Relaxed);
    }

    fn select(&self, index: usize) {
        if index >= self.options.len() {
            return;
        }
        let mut selected = self.selected.lock().unwrap();
        *selected = index;
        let value = self.options[index].value.clone();
        drop(selected);
        self.open.store(false, Ordering::Relaxed);
        self.dirty.store(true, Ordering::Relaxed);
        if let Some(ref on_change) = self.on_change {
            on_change(&value);
        }
    }

    fn row_for(popup_y: f32) -> usize {
        (popup_y / ROW_HEIGHT).floor().max(0.0) as usize
    }

    fn measure_label(&self, label: &str, style: &TextStyle) -> f32 {
        self.measurer
            .measure(&TextMeasureRequest {
                text: label.to_string(),
                style: style.clone(),
            })
            .map(|fragments| fragments.iter().map(|f| f.width).sum())
            .unwrap_or(0.0)
    }

    /// The widest option label plus the box chrome (arrow + padding).
    fn widest_label(&self, style: &TextStyle) -> f32 {
        self.options
            .iter()
            .map(|option| self.measure_label(&option.label, style))
            .fold(MIN_WIDTH, f32::max)
    }

    fn popup_width(&self) -> f32 {
        let box_width = self.get_last_size().width;
        let labels = self.widest_label(&TextStyle::default());
        box_width.max(labels + INLINE_PADDING * 2.0 + ARROW_WIDTH)
    }

    fn push_border(cmd_buf: &mut Vec<DrawCommand>, x: f32, y: f32, width: f32, height: f32) {
        let paint = Paint {
            brush: Brush::Solid(BORDER_COLOR),
            opacity: 1.0,
        };
        for rect in [
            rect_path(x, y, width, 1.0),
            rect_path(x, y + height - 1.0, width, 1.0),
            rect_path(x, y, 1.0, height),
            rect_path(x + width - 1.0, y, 1.0, height),
        ] {
            cmd_buf.push(DrawCommand::Fill {
                path: rect,
                rule: FillRule::NonZero,
                paint: paint.clone(),
            });
        }
    }

    fn push_fill(cmd_buf: &mut Vec<DrawCommand>, path: Path, color: Color) {
        cmd_buf.push(DrawCommand::Fill {
            path,
            rule: FillRule::NonZero,
            paint: Paint {
                brush: Brush::Solid(color),
                opacity: 1.0,
            },
        });
    }

    fn get_last_size(&self) -> ContentSize {
        *self.last_size.lock().unwrap()
    }
}

fn push_text(cmd_buf: &mut Vec<DrawCommand>, x: f32, y: f32, text: &str, style: &TextStyle) {
    cmd_buf.push(DrawCommand::DrawText {
        x,
        y,
        text: text.into(),
        style: style.clone(),
    });
}

impl CustomNode for SelectComponent {
    fn draw_sized(
        &self,
        cmd_buf: &mut Vec<DrawCommand>,
        text_style: &TextStyle,
        _style: &Style,
        size: ContentSize,
    ) {
        {
            *self.last_size.lock().unwrap() = size;
        }

        let index = *self.selected.lock().unwrap();
        let label = self
            .options
            .get(index)
            .map_or("", |option| option.label.as_str());

        Self::push_border(cmd_buf, 0.0, 0.0, size.width, size.height);

        let font_size = text_style.font_size;
        let text_y = ((size.height - font_size) * 0.5).max(0.0);
        let arrow_x = (size.width - INLINE_PADDING - font_size).max(0.0);

        // Drop-down arrow on the right.
        push_text(cmd_buf, arrow_x, text_y, "▾", text_style);

        // The label is clipped so it never runs under the arrow.
        let label_width = (arrow_x - INLINE_PADDING).max(0.0);
        if label_width > 0.0 {
            cmd_buf.push(DrawCommand::PushClip {
                path: rect_path(INLINE_PADDING, 0.0, label_width, size.height),
                rule: FillRule::NonZero,
            });
        }
        push_text(cmd_buf, INLINE_PADDING, text_y, label, text_style);
        if label_width > 0.0 {
            cmd_buf.push(DrawCommand::PopClip);
        }
    }

    fn background(&self) -> Option<Background> {
        let color = if self.open.load(Ordering::Relaxed) {
            Color(220, 220, 220, 255)
        } else if self.hovered.load(Ordering::Relaxed) {
            Color(240, 240, 240, 255)
        } else {
            Color(250, 250, 250, 255)
        };
        Some(Background::Color(color))
    }

    fn intrinsic_size(&self) -> ContentSize {
        ContentSize {
            width: self.widest_label(&TextStyle::default()) + INLINE_PADDING * 2.0 + ARROW_WIDTH,
            height: ROW_HEIGHT,
        }
    }

    fn on_pointer_event(&self, event: PointerEvent) -> bool {
        match event {
            PointerEvent::Move { .. } => {
                self.set_hovered(true);
                true
            }
            PointerEvent::Down { .. } => {
                self.toggle();
                true
            }
            PointerEvent::Up { .. } => false,
            PointerEvent::Leave => {
                self.set_hovered(false);
                false
            }
        }
    }

    fn set_hovered(&self, hovered: bool) {
        if self.hovered.swap(hovered, Ordering::Relaxed) != hovered {
            self.dirty.store(true, Ordering::Relaxed);
        }
    }

    fn is_hovered(&self) -> bool {
        self.hovered.load(Ordering::Relaxed)
    }

    fn on_popup_pointer_event(&self, event: PointerEvent) -> bool {
        let width = self.popup_width();
        let height = ROW_HEIGHT * self.options.len() as f32;
        let in_popup = |x: f32, y: f32| x >= 0.0 && x <= width && y >= 0.0 && y <= height;
        match event {
            PointerEvent::Move { x, y } => {
                let row = if in_popup(x, y) {
                    Self::row_for(y)
                } else {
                    usize::MAX
                };
                let row = (row < self.options.len())
                    .then_some(row as i32)
                    .unwrap_or(-1);
                if self.hover_index.swap(row, Ordering::Relaxed) != row {
                    self.dirty.store(true, Ordering::Relaxed);
                }
                true
            }
            PointerEvent::Down { x, y } if in_popup(x, y) => {
                self.select(Self::row_for(y));
                true
            }
            PointerEvent::Up { .. } => false,
            PointerEvent::Leave => {
                if self.hover_index.swap(-1, Ordering::Relaxed) != -1 {
                    self.dirty.store(true, Ordering::Relaxed);
                }
                false
            }
            PointerEvent::Down { .. } => false,
        }
    }

    fn dismiss_popup(&self) {
        if self.open.swap(false, Ordering::Relaxed) {
            self.dirty.store(true, Ordering::Relaxed);
        }
    }

    fn popup(&self, text_style: &TextStyle) -> Option<Popup> {
        if !self.open.load(Ordering::Relaxed) || self.options.is_empty() {
            return None;
        }

        let (box_height, width, height) = {
            let size = self.get_last_size();
            let width = self.popup_width();
            let height = ROW_HEIGHT * self.options.len() as f32;
            (size.height, width, height)
        };

        let mut commands = Vec::new();
        Self::push_fill(
            &mut commands,
            rect_path(0.0, box_height, width, height),
            POPUP_BG,
        );
        Self::push_border(&mut commands, 0.0, box_height, width, height);

        let selected = *self.selected.lock().unwrap();
        let hover = self.hover_index.load(Ordering::Relaxed);
        let font_size = text_style.font_size;

        for (i, option) in self.options.iter().enumerate() {
            let row_y = box_height + i as f32 * ROW_HEIGHT;
            if i as i32 == hover {
                Self::push_fill(
                    &mut commands,
                    rect_path(0.0, row_y, width, ROW_HEIGHT),
                    HIGHLIGHT_BG,
                );
            }
            let mut style = text_style.clone();
            if i == selected {
                style.font_weight = FontWeight(700);
            }
            push_text(
                &mut commands,
                INLINE_PADDING,
                row_y + ((ROW_HEIGHT - font_size) * 0.5).max(0.0),
                &option.label,
                &style,
            );
        }

        Some(Popup {
            rect: Rect {
                x: 0.0,
                y: box_height,
                width,
                height,
            },
            commands,
        })
    }

    fn needs_repaint(&self) -> bool {
        self.dirty.swap(false, Ordering::Relaxed)
    }

    fn role(&self) -> Option<&'static str> {
        Some("combobox")
    }

    fn label(&self) -> Option<String> {
        let index = *self.selected.lock().unwrap();
        self.options.get(index).map(|option| option.label.clone())
    }

    fn value(&self) -> Option<String> {
        Some(self.selected_value())
    }
}

/// Resolves the initially selected option: the one matching `value`, else the
/// first option carrying the `selected` attribute, else the first option.
fn initial_index(options: &[SelectOption], value: &str) -> usize {
    if !value.is_empty()
        && let Some(index) = options.iter().position(|option| option.value == value)
    {
        return index;
    }
    options
        .iter()
        .position(|option| option.selected)
        .unwrap_or(0)
        .min(options.len().saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::bridge::text::FallbackTextMeasurer;

    fn options() -> Vec<SelectOption> {
        vec![
            SelectOption {
                value: "a".into(),
                label: "Alpha".into(),
                selected: true,
            },
            SelectOption {
                value: "b".into(),
                label: "Bravo".into(),
                selected: false,
            },
            SelectOption {
                value: "c".into(),
                label: "Charlie".into(),
                selected: false,
            },
        ]
    }

    fn measurer() -> Arc<dyn text::TextMeasurer<TextStyle>> {
        Arc::new(FallbackTextMeasurer)
    }

    fn component() -> SelectComponent {
        SelectComponent::new(options(), "", measurer())
    }

    #[test]
    fn starts_closed_with_selected_option() {
        let select = component();
        assert!(!select.open.load(Ordering::Relaxed));
        assert_eq!(select.selected_value(), "a");
        assert_eq!(select.label(), Some("Alpha".to_string()));
        assert_eq!(select.role(), Some("combobox"));
    }

    #[test]
    fn value_attribute_overrides_selected_attribute() {
        let select = SelectComponent::new(options(), "c", measurer());
        assert_eq!(select.selected_value(), "c");
    }

    #[test]
    fn box_click_toggles_popup() {
        let select = component();
        select.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });
        assert!(select.popup(&TextStyle::default()).is_some());
        select.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });
        assert!(select.popup(&TextStyle::default()).is_none());
    }

    #[test]
    fn popup_row_click_selects_and_closes() {
        let select = component();
        select.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });
        assert!(select.popup(&TextStyle::default()).is_some());
        // Popup events are expressed relative to the popup's own top-left.
        select.on_popup_pointer_event(PointerEvent::Down {
            x: 10.0,
            y: ROW_HEIGHT * 2.0,
        });
        assert_eq!(select.selected_value(), "c");
        assert!(select.popup(&TextStyle::default()).is_none());
    }

    #[test]
    fn popup_hover_tracks_row_and_clears_outside() {
        let select = component();
        select.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });
        assert!(select.popup(&TextStyle::default()).is_some());

        select.on_popup_pointer_event(PointerEvent::Move { x: 10.0, y: 2.0 });
        assert_eq!(select.hover_index.load(Ordering::Relaxed), 0);
        select.on_popup_pointer_event(PointerEvent::Move {
            x: 10.0,
            y: ROW_HEIGHT + 2.0,
        });
        assert_eq!(select.hover_index.load(Ordering::Relaxed), 1);
        // Outside the popup clears the highlight.
        select.on_popup_pointer_event(PointerEvent::Move { x: 10.0, y: -5.0 });
        assert_eq!(select.hover_index.load(Ordering::Relaxed), -1);
    }

    #[test]
    fn popup_is_empty_when_closed_or_optionless() {
        let select = component();
        assert!(select.popup(&TextStyle::default()).is_none());
        let empty = SelectComponent::new(Vec::new(), "", measurer());
        empty.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });
        assert!(empty.popup(&TextStyle::default()).is_none());
    }

    #[test]
    fn dismiss_popup_closes() {
        let select = component();
        select.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });
        assert!(select.open.load(Ordering::Relaxed));
        select.dismiss_popup();
        assert!(!select.open.load(Ordering::Relaxed));
        assert!(select.needs_repaint());
    }

    #[test]
    fn popup_draws_background_highlight_and_option_text() {
        let select = component();
        select.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });
        let popup = select.popup(&TextStyle::default()).unwrap();
        assert!(!popup.commands.is_empty());
        assert!(
            popup
                .commands
                .iter()
                .any(|cmd| matches!(cmd, DrawCommand::DrawText { text, .. } if text == "Alpha"))
        );
        assert!(
            popup
                .commands
                .iter()
                .any(|cmd| matches!(cmd, DrawCommand::Fill { .. }))
        );
    }

    #[test]
    fn on_change_reports_new_value() {
        use std::sync::Mutex as StdMutex;
        let received: Arc<StdMutex<Vec<String>>> = Arc::new(StdMutex::new(Vec::new()));
        let received_clone = Arc::clone(&received);
        let cb: Arc<OnSelectChange> = Arc::new(move |value: &str| {
            received_clone.lock().unwrap().push(value.to_string());
        });
        let select = SelectComponent::with_on_change(options(), "", measurer(), cb);

        select.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });
        assert!(select.popup(&TextStyle::default()).is_some());
        select.on_popup_pointer_event(PointerEvent::Down {
            x: 10.0,
            y: ROW_HEIGHT,
        });
        assert_eq!(*received.lock().unwrap(), vec!["b".to_string()]);
    }

    #[test]
    fn intrinsic_size_fits_wide_option() {
        let mut wide = options();
        wide.push(SelectOption {
            value: "wide".into(),
            label: "A very long option label".into(),
            selected: false,
        });
        let select = SelectComponent::new(wide, "", measurer());
        let size = select.intrinsic_size();
        assert!(size.width >= MIN_WIDTH);
        assert!(size.height == ROW_HEIGHT);
    }
}
