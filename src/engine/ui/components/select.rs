//! Dropdown component for the HTML `<select>` element.
//!
//! The control renders like a combo box: the currently selected option's
//! label plus a drop-down arrow. Clicking the box opens a popup (top-layer
//! overlay) listing every option; clicking a row selects it and reports the
//! new value through the DOM write-back channel.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use ui_layout::Style;

use crate::engine::bridge::text::{self, TextAttribute, TextMeasureRequest};
use crate::engine::layouter::types::{Background, Color, FontWeight, TextFlowStyle, TextStyle};
use crate::engine::renderer_model::{Brush, DrawCommand, FillRule, Paint, Path, Rect, rect_path};
use crate::engine::ui::custom_node::{ContentSize, CustomNode, PointerEvent, Popup};

/// Row height of the box and of each dropdown option.
const ROW_HEIGHT: f32 = 28.0;
/// Height of an `<optgroup>` header row inside the dropdown.
const GROUP_ROW_HEIGHT: f32 = 20.0;
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
/// Background of a selected option row.
const SELECTED_BG: Color = Color(210, 210, 210, 255);
/// Text color of a selected option row.
const SELECTED_COLOR: Color = Color(40, 40, 40, 255);
/// Text color for disabled controls and disabled options.
const DISABLED_COLOR: Color = Color(150, 150, 150, 255);
/// Text color for `<optgroup>` header rows.
const GROUP_COLOR: Color = Color(110, 110, 110, 255);

/// One `<option>` inside a `<select>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectOption {
    /// The option's `value` attribute (falls back to its text content).
    pub value: String,
    /// The option's visible label.
    pub label: String,
    /// Whether the option carries the `selected` attribute.
    pub selected: bool,
    /// Whether the option (or its `<optgroup>`) carries the `disabled`
    /// attribute. Disabled options are shown grayed out and cannot be picked.
    pub disabled: bool,
    /// Label of the containing `<optgroup>`, when the option is grouped.
    pub group: Option<String>,
}

/// A row rendered inside the dropdown: either an option or an `<optgroup>`
/// header. Group headers are informational and never selectable.
#[derive(Debug, Clone, Copy)]
enum PopupRow<'a> {
    Option(usize),
    Group(&'a str),
}

/// Callback invoked when the select's value changes.
pub type OnSelectChange = dyn Fn(&str) + Send + Sync;

/// An HTML `<select>` rendered by the engine.
pub struct SelectComponent {
    options: Vec<SelectOption>,
    measurer: Arc<dyn text::TextMeasurer>,
    selected: Mutex<Vec<usize>>,
    open: AtomicBool,
    hovered: AtomicBool,
    hover_index: AtomicI32,
    dirty: AtomicBool,
    last_size: Mutex<ContentSize>,
    on_change: Option<Arc<OnSelectChange>>,
    disabled: bool,
    multiple: bool,
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
    /// `disabled` mirrors the element's `disabled` attribute: a disabled
    /// select ignores input and never opens its popup.
    /// `multiple` mirrors the element's `multiple` attribute: multiple
    /// options can be selected at once (toggled from the popup) and the
    /// reported value is a comma-separated list.
    pub fn new(
        options: Vec<SelectOption>,
        value: &str,
        measurer: Arc<dyn text::TextMeasurer>,
        disabled: bool,
        multiple: bool,
    ) -> Self {
        Self {
            selected: Mutex::new(initial_selection(&options, value, multiple)),
            options,
            measurer,
            open: AtomicBool::new(false),
            hovered: AtomicBool::new(false),
            hover_index: AtomicI32::new(-1),
            dirty: AtomicBool::new(true),
            last_size: Mutex::new(ContentSize::zero()),
            on_change: None,
            disabled,
            multiple,
        }
    }

    /// Creates a select with a value-change callback for DOM sync.
    pub fn with_on_change(
        options: Vec<SelectOption>,
        value: &str,
        measurer: Arc<dyn text::TextMeasurer>,
        on_change: Arc<OnSelectChange>,
        disabled: bool,
        multiple: bool,
    ) -> Self {
        let mut select = Self::new(options, value, measurer, disabled, multiple);
        select.on_change = Some(on_change);
        select
    }

    /// Returns the currently selected option's value. For a multiple select
    /// this is the selected values joined with commas.
    pub fn selected_value(&self) -> String {
        let selected = self.selected.lock().unwrap();
        let values: Vec<&str> = selected
            .iter()
            .filter_map(|&index| self.options.get(index))
            .map(|option| option.value.as_str())
            .collect();
        if self.multiple {
            values.join(",")
        } else {
            values.first().copied().unwrap_or("").to_string()
        }
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
        if index >= self.options.len() || self.options[index].disabled {
            return;
        }
        let value = {
            let mut selected = self.selected.lock().unwrap();
            if self.multiple {
                if let Some(pos) = selected.iter().position(|&i| i == index) {
                    selected.remove(pos);
                } else {
                    let pos = selected
                        .iter()
                        .position(|&i| i > index)
                        .unwrap_or(selected.len());
                    selected.insert(pos, index);
                }
                drop(selected);
                self.dirty.store(true, Ordering::Relaxed);
            } else {
                *selected = vec![index];
                drop(selected);
                self.open.store(false, Ordering::Relaxed);
                self.dirty.store(true, Ordering::Relaxed);
            }
            self.selected_value()
        };
        if let Some(ref on_change) = self.on_change {
            on_change(&value);
        }
    }

    /// Rows to render in the dropdown, in document order. A group header is
    /// inserted whenever an option's group differs from the previous option's.
    fn popup_rows(&self) -> Vec<PopupRow<'_>> {
        let mut rows = Vec::new();
        let mut last_group: Option<&str> = None;
        for (i, option) in self.options.iter().enumerate() {
            let group = option.group.as_deref();
            if group != last_group {
                if let Some(label) = group {
                    rows.push(PopupRow::Group(label));
                }
                last_group = group;
            }
            rows.push(PopupRow::Option(i));
        }
        rows
    }

    /// The row index whose vertical span contains `y`, or `None` if `y` falls
    /// outside every row.
    fn row_at(&self, rows: &[PopupRow<'_>], y: f32) -> Option<usize> {
        let mut acc = 0.0f32;
        for (i, row) in rows.iter().enumerate() {
            let h = Self::row_height(row);
            if y >= acc && y < acc + h {
                return Some(i);
            }
            acc += h;
        }
        None
    }

    fn row_height(row: &PopupRow<'_>) -> f32 {
        match row {
            PopupRow::Group(_) => GROUP_ROW_HEIGHT,
            PopupRow::Option(_) => ROW_HEIGHT,
        }
    }

    fn popup_height(&self, rows: &[PopupRow<'_>]) -> f32 {
        rows.iter().map(Self::row_height).sum()
    }

    /// Distinct `<optgroup>` labels in document order.
    fn group_labels(&self) -> Vec<&str> {
        let mut labels = Vec::new();
        let mut last: Option<&str> = None;
        for option in &self.options {
            let group = option.group.as_deref();
            if group != last {
                if let Some(label) = group {
                    labels.push(label);
                }
                last = group;
            }
        }
        labels
    }

    fn measure_label(&self, label: &str, style: &TextStyle, flow_style: TextFlowStyle) -> f32 {
        self.measurer
            .measure(&TextMeasureRequest {
                text: label.to_string(),
                attribute: TextAttribute {
                    style: style.clone(),
                    flow_style,
                },
            })
            .map(|fragments| fragments.iter().map(|f| f.width).sum())
            .unwrap_or(0.0)
    }

    /// The widest option label plus the box chrome (arrow + padding).
    fn widest_label(&self, style: &TextStyle) -> f32 {
        self.options
            .iter()
            .map(|option| self.measure_label(&option.label, style, TextFlowStyle::default()))
            .fold(MIN_WIDTH, f32::max)
    }

    fn popup_width(&self) -> f32 {
        let box_width = self.get_last_size().width;
        let labels = self
            .options
            .iter()
            .map(|option| {
                self.measure_label(
                    &option.label,
                    &TextStyle::default(),
                    TextFlowStyle::default(),
                )
            })
            .chain(self.group_labels().iter().map(|label| {
                self.measure_label(label, &TextStyle::default(), TextFlowStyle::default())
            }))
            .fold(MIN_WIDTH, f32::max);
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

    /// Draws the dropdown/list rows starting at `y_offset`, with `width` used
    /// for the hover highlight. Group headers and disabled options reuse the
    /// popup styling.
    fn push_rows(
        &self,
        cmd_buf: &mut Vec<DrawCommand>,
        rows: &[PopupRow<'_>],
        y_offset: f32,
        width: f32,
        text_style: &TextStyle,
        text_flow_style: &TextFlowStyle,
    ) {
        let selected = self.selected.lock().unwrap();
        let hover = self.hover_index.load(Ordering::Relaxed);
        let font_size = text_flow_style.font_size;

        let mut y = y_offset;
        for row in rows {
            match row {
                PopupRow::Group(label) => {
                    let mut style = text_style.clone();
                    let mut flow_style = *text_flow_style;
                    flow_style.font_size = (font_size * 0.85).max(10.0);
                    style.font_weight = FontWeight(700);
                    style.color = GROUP_COLOR;
                    push_text(
                        cmd_buf,
                        INLINE_PADDING,
                        y + ((GROUP_ROW_HEIGHT - flow_style.font_size) * 0.5).max(0.0),
                        label,
                        &style,
                        flow_style,
                    );
                    y += GROUP_ROW_HEIGHT;
                }
                PopupRow::Option(i) => {
                    let option = &self.options[*i];
                    let is_selected = selected.contains(i);
                    if is_selected {
                        Self::push_fill(cmd_buf, rect_path(0.0, y, width, ROW_HEIGHT), SELECTED_BG);
                    }
                    if *i as i32 == hover && !option.disabled {
                        Self::push_fill(
                            cmd_buf,
                            rect_path(0.0, y, width, ROW_HEIGHT),
                            HIGHLIGHT_BG,
                        );
                    }
                    let mut style = text_style.clone();
                    if is_selected {
                        style.font_weight = FontWeight(700);
                        style.color = SELECTED_COLOR;
                    }
                    if option.disabled {
                        style.color = DISABLED_COLOR;
                    }
                    push_text(
                        cmd_buf,
                        INLINE_PADDING,
                        y + ((ROW_HEIGHT - font_size) * 0.5).max(0.0),
                        &option.label,
                        &style,
                        *text_flow_style,
                    );
                    y += ROW_HEIGHT;
                }
            }
        }
    }

    /// Pointer handling for a multiple select rendered as a list box: hover
    /// highlights the option under the cursor and a click toggles it.
    fn on_list_pointer_event(&self, event: PointerEvent) -> bool {
        let rows = self.popup_rows();
        match event {
            PointerEvent::Move { y, .. } => {
                self.set_hovered(true);
                let hover = self
                    .row_at(&rows, y)
                    .and_then(|row| match rows[row] {
                        PopupRow::Option(i) if !self.options[i].disabled => Some(i as i32),
                        _ => None,
                    })
                    .unwrap_or(-1);
                if self.hover_index.swap(hover, Ordering::Relaxed) != hover {
                    self.dirty.store(true, Ordering::Relaxed);
                }
                true
            }
            PointerEvent::Down { y, .. } => {
                if let Some(PopupRow::Option(i)) = self.row_at(&rows, y).map(|row| rows[row])
                    && !self.options[i].disabled
                {
                    self.select(i);
                }
                true
            }
            PointerEvent::Up { .. } => false,
            PointerEvent::Leave => {
                self.set_hovered(false);
                if self.hover_index.swap(-1, Ordering::Relaxed) != -1 {
                    self.dirty.store(true, Ordering::Relaxed);
                }
                false
            }
        }
    }
}

fn push_text(
    cmd_buf: &mut Vec<DrawCommand>,
    x: f32,
    y: f32,
    text: &str,
    style: &TextStyle,
    flow_style: TextFlowStyle,
) {
    cmd_buf.push(DrawCommand::DrawText {
        x,
        y,
        text: text.into(),
        style: style.clone(),
        flow_style,
    });
}

impl CustomNode for SelectComponent {
    fn draw_sized(
        &self,
        cmd_buf: &mut Vec<DrawCommand>,
        text_style: &TextStyle,
        text_flow_style: &TextFlowStyle,
        _style: &Style,
        size: ContentSize,
    ) {
        {
            *self.last_size.lock().unwrap() = size;
        }

        if self.multiple {
            let rows = self.popup_rows();
            Self::push_border(cmd_buf, 0.0, 0.0, size.width, size.height);
            self.push_rows(cmd_buf, &rows, 0.0, size.width, text_style, text_flow_style);
            return;
        }

        let label = {
            let selected = self.selected.lock().unwrap();
            selected
                .iter()
                .filter_map(|&i| self.options.get(i))
                .map(|option| option.label.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };

        let mut box_style = text_style.clone();
        if self.disabled {
            box_style.color = DISABLED_COLOR;
        }

        Self::push_border(cmd_buf, 0.0, 0.0, size.width, size.height);

        let font_size = text_flow_style.font_size;
        let text_y = ((size.height - font_size) * 0.5).max(0.0);
        let arrow_x = (size.width - INLINE_PADDING - font_size).max(0.0);

        // Drop-down arrow on the right.
        push_text(cmd_buf, arrow_x, text_y, "▾", &box_style, *text_flow_style);

        // The label is clipped so it never runs under the arrow.
        let label_width = (arrow_x - INLINE_PADDING).max(0.0);
        if label_width > 0.0 {
            cmd_buf.push(DrawCommand::PushClip {
                path: rect_path(INLINE_PADDING, 0.0, label_width, size.height),
                rule: FillRule::NonZero,
            });
        }
        push_text(
            cmd_buf,
            INLINE_PADDING,
            text_y,
            &label,
            &box_style,
            *text_flow_style,
        );
        if label_width > 0.0 {
            cmd_buf.push(DrawCommand::PopClip);
        }
    }

    fn background(&self) -> Option<Background> {
        if self.disabled {
            return Some(Background::Color(Color(230, 230, 230, 255)));
        }
        if self.multiple {
            return Some(Background::Color(POPUP_BG));
        }
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
        if self.multiple {
            let rows = self.popup_rows();
            return ContentSize {
                width: self.widest_label(&TextStyle::default())
                    + INLINE_PADDING * 2.0
                    + ARROW_WIDTH,
                height: rows.iter().map(Self::row_height).sum(),
            };
        }
        ContentSize {
            width: self.widest_label(&TextStyle::default()) + INLINE_PADDING * 2.0 + ARROW_WIDTH,
            height: ROW_HEIGHT,
        }
    }

    fn on_pointer_event(&self, event: PointerEvent) -> bool {
        if self.disabled {
            return false;
        }
        if self.multiple {
            return self.on_list_pointer_event(event);
        }
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
        if self.disabled {
            return false;
        }
        let rows = self.popup_rows();
        let width = self.popup_width();
        let height = self.popup_height(&rows);
        let in_popup = |x: f32, y: f32| x >= 0.0 && x <= width && y >= 0.0 && y <= height;
        match event {
            PointerEvent::Move { x, y } => {
                let option_index = if in_popup(x, y) {
                    self.row_at(&rows, y)
                        .and_then(|row| match rows[row] {
                            PopupRow::Option(i) if !self.options[i].disabled => Some(i as i32),
                            _ => None,
                        })
                        .unwrap_or(-1)
                } else {
                    -1
                };
                if self.hover_index.swap(option_index, Ordering::Relaxed) != option_index {
                    self.dirty.store(true, Ordering::Relaxed);
                }
                true
            }
            PointerEvent::Down { x, y } if in_popup(x, y) => {
                if let Some(PopupRow::Option(i)) = self.row_at(&rows, y).map(|row| rows[row])
                    && !self.options[i].disabled
                {
                    self.select(i);
                }
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

    fn popup(&self, text_style: &TextStyle, text_flow_style: &TextFlowStyle) -> Option<Popup> {
        if self.multiple
            || !self.open.load(Ordering::Relaxed)
            || self.options.is_empty()
            || self.disabled
        {
            return None;
        }

        let rows = self.popup_rows();
        let (box_height, width, height) = {
            let size = self.get_last_size();
            let width = self.popup_width();
            let height = self.popup_height(&rows);
            (size.height, width, height)
        };

        let mut commands = Vec::new();
        Self::push_fill(
            &mut commands,
            rect_path(0.0, box_height, width, height),
            POPUP_BG,
        );
        Self::push_border(&mut commands, 0.0, box_height, width, height);
        self.push_rows(
            &mut commands,
            &rows,
            box_height,
            width,
            text_style,
            text_flow_style,
        );

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
        Some(if self.multiple { "listbox" } else { "combobox" })
    }

    fn label(&self) -> Option<String> {
        let selected = self.selected.lock().unwrap();
        Some(
            selected
                .iter()
                .filter_map(|&i| self.options.get(i))
                .map(|option| option.label.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        )
    }

    fn value(&self) -> Option<String> {
        Some(self.selected_value())
    }

    fn is_disabled(&self) -> bool {
        self.disabled
    }
}

/// Resolves the initially selected options.
///
/// For a multiple select this is every option whose value appears in the
/// comma-separated `value` (for DOM write-back round-trips), else every option
/// carrying the `selected` attribute, which may be empty. For a single select
/// it is the option matching `value`, else the first `selected` one, else the
/// first option.
fn initial_selection(options: &[SelectOption], value: &str, multiple: bool) -> Vec<usize> {
    if !value.is_empty() {
        let requested: Vec<&str> = if multiple {
            value.split(',').map(str::trim).collect()
        } else {
            vec![value]
        };
        let mut indices = Vec::new();
        for requested_value in requested {
            if let Some(index) = options
                .iter()
                .position(|option| option.value == requested_value)
                && !indices.contains(&index)
            {
                indices.push(index);
            }
        }
        if !indices.is_empty() {
            return indices;
        }
    }
    if multiple {
        options
            .iter()
            .enumerate()
            .filter(|(_, option)| option.selected)
            .map(|(i, _)| i)
            .collect()
    } else {
        vec![
            options
                .iter()
                .position(|option| option.selected)
                .unwrap_or(0)
                .min(options.len().saturating_sub(1)),
        ]
    }
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
                disabled: false,
                group: None,
            },
            SelectOption {
                value: "b".into(),
                label: "Bravo".into(),
                selected: false,
                disabled: false,
                group: None,
            },
            SelectOption {
                value: "c".into(),
                label: "Charlie".into(),
                selected: false,
                disabled: false,
                group: None,
            },
        ]
    }

    fn measurer() -> Arc<dyn text::TextMeasurer> {
        Arc::new(FallbackTextMeasurer)
    }

    fn component() -> SelectComponent {
        SelectComponent::new(options(), "", measurer(), false, false)
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
        let select = SelectComponent::new(options(), "c", measurer(), false, false);
        assert_eq!(select.selected_value(), "c");
    }

    #[test]
    fn box_click_toggles_popup() {
        let select = component();
        select.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });
        assert!(
            select
                .popup(&TextStyle::default(), &TextFlowStyle::default())
                .is_some()
        );
        select.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });
        assert!(
            select
                .popup(&TextStyle::default(), &TextFlowStyle::default())
                .is_none()
        );
    }

    #[test]
    fn popup_row_click_selects_and_closes() {
        let select = component();
        select.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });
        assert!(
            select
                .popup(&TextStyle::default(), &TextFlowStyle::default())
                .is_some()
        );
        // Popup events are expressed relative to the popup's own top-left.
        select.on_popup_pointer_event(PointerEvent::Down {
            x: 10.0,
            y: ROW_HEIGHT * 2.0,
        });
        assert_eq!(select.selected_value(), "c");
        assert!(
            select
                .popup(&TextStyle::default(), &TextFlowStyle::default())
                .is_none()
        );
    }

    #[test]
    fn popup_hover_tracks_row_and_clears_outside() {
        let select = component();
        select.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });
        assert!(
            select
                .popup(&TextStyle::default(), &TextFlowStyle::default())
                .is_some()
        );

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
        assert!(
            select
                .popup(&TextStyle::default(), &TextFlowStyle::default())
                .is_none()
        );
        let empty = SelectComponent::new(Vec::new(), "", measurer(), false, false);
        empty.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });
        assert!(
            empty
                .popup(&TextStyle::default(), &TextFlowStyle::default())
                .is_none()
        );
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
        let popup = select
            .popup(&TextStyle::default(), &TextFlowStyle::default())
            .unwrap();
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
        let select = SelectComponent::with_on_change(options(), "", measurer(), cb, false, false);

        select.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });
        assert!(
            select
                .popup(&TextStyle::default(), &TextFlowStyle::default())
                .is_some()
        );
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
            disabled: false,
            group: None,
        });
        let select = SelectComponent::new(wide, "", measurer(), false, false);
        let size = select.intrinsic_size();
        assert!(size.width >= MIN_WIDTH);
        assert!(size.height == ROW_HEIGHT);
    }

    #[test]
    fn disabled_select_ignores_input_and_never_opens() {
        let select = SelectComponent::new(options(), "", measurer(), true, false);
        assert!(select.is_disabled());
        assert_eq!(
            select.background(),
            Some(Background::Color(Color(230, 230, 230, 255)))
        );

        // Consume the initial dirty flag from construction.
        assert!(select.needs_repaint());

        select.on_pointer_event(PointerEvent::Move { x: 5.0, y: 5.0 });
        select.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });
        assert!(!select.open.load(Ordering::Relaxed));
        assert!(
            select
                .popup(&TextStyle::default(), &TextFlowStyle::default())
                .is_none()
        );

        // Disabled events never mark the component dirty.
        assert!(!select.needs_repaint());
        select.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });
        assert!(!select.needs_repaint());
    }

    #[test]
    fn disabled_option_is_not_selectable() {
        let mut opts = options();
        opts[1].disabled = true;
        let select = SelectComponent::new(opts, "", measurer(), false, false);

        select.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });
        assert!(
            select
                .popup(&TextStyle::default(), &TextFlowStyle::default())
                .is_some()
        );

        // Hovering the disabled row must not highlight it.
        select.on_popup_pointer_event(PointerEvent::Move {
            x: 10.0,
            y: ROW_HEIGHT + 2.0,
        });
        assert_eq!(select.hover_index.load(Ordering::Relaxed), -1);

        // Clicking it must not change the selection or close the popup.
        select.on_popup_pointer_event(PointerEvent::Down {
            x: 10.0,
            y: ROW_HEIGHT + 2.0,
        });
        assert_eq!(select.selected_value(), "a");
        assert!(select.open.load(Ordering::Relaxed));
    }

    fn grouped_options() -> Vec<SelectOption> {
        vec![
            SelectOption {
                value: "a".into(),
                label: "Apple".into(),
                selected: true,
                disabled: false,
                group: Some("Fruits".into()),
            },
            SelectOption {
                value: "b".into(),
                label: "Banana".into(),
                selected: false,
                disabled: false,
                group: Some("Fruits".into()),
            },
            SelectOption {
                value: "c".into(),
                label: "Carrot".into(),
                selected: false,
                disabled: false,
                group: Some("Veggies".into()),
            },
        ]
    }

    #[test]
    fn optgroup_renders_headers_and_sizes_rows() {
        let select = SelectComponent::new(grouped_options(), "", measurer(), false, false);
        select.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });

        let rows = select.popup_rows();
        assert!(matches!(rows[0], PopupRow::Group("Fruits")));
        assert!(matches!(rows[1], PopupRow::Option(0)));
        assert!(matches!(rows[2], PopupRow::Option(1)));
        assert!(matches!(rows[3], PopupRow::Group("Veggies")));
        assert!(matches!(rows[4], PopupRow::Option(2)));

        let popup = select
            .popup(&TextStyle::default(), &TextFlowStyle::default())
            .unwrap();
        let expected_height = 2.0 * GROUP_ROW_HEIGHT + 3.0 * ROW_HEIGHT;
        assert!((popup.rect.height - expected_height).abs() < 1e-3);
        assert!(
            popup
                .commands
                .iter()
                .any(|cmd| matches!(cmd, DrawCommand::DrawText { text, .. } if text == "Fruits"))
        );

        // Group headers must not be clickable: clicking one keeps the popup
        // open and preserves the selection.
        select.on_popup_pointer_event(PointerEvent::Move { x: 10.0, y: 2.0 });
        assert_eq!(select.hover_index.load(Ordering::Relaxed), -1);
        select.on_popup_pointer_event(PointerEvent::Down { x: 10.0, y: 2.0 });
        assert_eq!(select.selected_value(), "a");
        assert!(select.open.load(Ordering::Relaxed));
    }

    #[test]
    fn optgroup_row_offsets_map_to_the_right_option() {
        let select = SelectComponent::new(grouped_options(), "", measurer(), false, false);
        select.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });

        // Banana sits below the Fruits header + Apple row.
        let banana_y = GROUP_ROW_HEIGHT + ROW_HEIGHT + 2.0;
        select.on_popup_pointer_event(PointerEvent::Move {
            x: 10.0,
            y: banana_y,
        });
        assert_eq!(select.hover_index.load(Ordering::Relaxed), 1);
        select.on_popup_pointer_event(PointerEvent::Down {
            x: 10.0,
            y: banana_y,
        });
        assert_eq!(select.selected_value(), "b");
    }

    fn multiple_options() -> Vec<SelectOption> {
        vec![
            SelectOption {
                value: "a".into(),
                label: "Alpha".into(),
                selected: true,
                disabled: false,
                group: None,
            },
            SelectOption {
                value: "b".into(),
                label: "Bravo".into(),
                selected: false,
                disabled: false,
                group: None,
            },
            SelectOption {
                value: "c".into(),
                label: "Charlie".into(),
                selected: true,
                disabled: false,
                group: None,
            },
        ]
    }

    #[test]
    fn multiple_initializes_from_selected_attributes() {
        let select = SelectComponent::new(multiple_options(), "", measurer(), false, true);
        assert_eq!(select.role(), Some("listbox"));
        assert_eq!(select.value(), Some("a,c".to_string()));
        assert_eq!(select.label(), Some("Alpha, Charlie".to_string()));
        // No popup: the list is rendered inline.
        assert!(
            select
                .popup(&TextStyle::default(), &TextFlowStyle::default())
                .is_none()
        );
    }

    #[test]
    fn multiple_grows_vertically_with_rows() {
        let select = SelectComponent::new(multiple_options(), "", measurer(), false, true);
        let size = select.intrinsic_size();
        assert_eq!(size.height, 3.0 * ROW_HEIGHT);

        let grouped = SelectComponent::new(grouped_options(), "", measurer(), false, true);
        let grouped_size = grouped.intrinsic_size();
        assert_eq!(
            grouped_size.height,
            2.0 * GROUP_ROW_HEIGHT + 3.0 * ROW_HEIGHT
        );
    }

    #[test]
    fn multiple_click_toggles_selection_without_popup() {
        let select = SelectComponent::new(multiple_options(), "", measurer(), false, true);

        // Click Bravo (row 1). It gets selected and the list stays inline.
        select.on_pointer_event(PointerEvent::Down {
            x: 5.0,
            y: ROW_HEIGHT + 2.0,
        });
        assert_eq!(select.selected_value(), "a,b,c");
        assert!(
            select
                .popup(&TextStyle::default(), &TextFlowStyle::default())
                .is_none()
        );

        // Click Alpha (row 0) to deselect it.
        select.on_pointer_event(PointerEvent::Down { x: 5.0, y: 2.0 });
        assert_eq!(select.selected_value(), "b,c");
        assert!(
            select
                .popup(&TextStyle::default(), &TextFlowStyle::default())
                .is_none()
        );
    }

    #[test]
    fn multiple_reports_changes_through_callback() {
        use std::sync::Mutex as StdMutex;
        let received: Arc<StdMutex<Vec<String>>> = Arc::new(StdMutex::new(Vec::new()));
        let received_clone = Arc::clone(&received);
        let cb: Arc<OnSelectChange> = Arc::new(move |value: &str| {
            received_clone.lock().unwrap().push(value.to_string());
        });
        let select =
            SelectComponent::with_on_change(multiple_options(), "", measurer(), cb, false, true);

        select.on_pointer_event(PointerEvent::Down {
            x: 5.0,
            y: ROW_HEIGHT + 2.0,
        });
        assert_eq!(*received.lock().unwrap(), vec!["a,b,c".to_string()]);
        select.on_pointer_event(PointerEvent::Down {
            x: 5.0,
            y: ROW_HEIGHT + 2.0,
        });
        assert_eq!(
            *received.lock().unwrap(),
            vec!["a,b,c".to_string(), "a,c".to_string()]
        );
    }

    #[test]
    fn multiple_value_attribute_restores_selection() {
        let select = SelectComponent::new(multiple_options(), "b,c", measurer(), false, true);
        assert_eq!(select.selected_value(), "b,c");

        // The `selected` attribute is used when no value is provided.
        let defaulted = SelectComponent::new(multiple_options(), "", measurer(), false, true);
        assert_eq!(defaulted.selected_value(), "a,c");
    }

    #[test]
    fn multiple_disabled_option_is_not_selectable() {
        let mut opts = multiple_options();
        opts[1].disabled = true;
        let select = SelectComponent::new(opts, "", measurer(), false, true);

        select.on_pointer_event(PointerEvent::Move {
            x: 5.0,
            y: ROW_HEIGHT + 2.0,
        });
        assert_eq!(select.hover_index.load(Ordering::Relaxed), -1);

        select.on_pointer_event(PointerEvent::Down {
            x: 5.0,
            y: ROW_HEIGHT + 2.0,
        });
        assert_eq!(select.selected_value(), "a,c");
    }

    #[test]
    fn multiple_draws_list_rows_without_arrow() {
        let select = SelectComponent::new(multiple_options(), "", measurer(), false, true);
        let mut commands = Vec::new();
        select.draw_sized(
            &mut commands,
            &TextStyle::default(),
            &TextFlowStyle::default(),
            &Style::default(),
            select.intrinsic_size(),
        );
        assert!(
            commands
                .iter()
                .any(|cmd| matches!(cmd, DrawCommand::DrawText { text, .. } if text == "Alpha"))
        );
        assert!(
            commands
                .iter()
                .any(|cmd| matches!(cmd, DrawCommand::DrawText { text, .. } if text == "Charlie"))
        );
        // The drop-down arrow is only drawn for single selects.
        assert!(
            !commands
                .iter()
                .any(|cmd| matches!(cmd, DrawCommand::DrawText { text, .. } if text == "▾"))
        );
    }

    #[test]
    fn multiple_selected_rows_are_highlighted() {
        let select = SelectComponent::new(multiple_options(), "", measurer(), false, true);
        let mut commands = Vec::new();
        select.draw_sized(
            &mut commands,
            &TextStyle::default(),
            &TextFlowStyle::default(),
            &Style::default(),
            select.intrinsic_size(),
        );

        // Selected rows (Alpha, Charlie) get a full-width fill with the row
        // height; border strips have other heights and are filtered out.
        let fills = commands.iter().filter_map(|cmd| match cmd {
            DrawCommand::Fill { path, .. } => path
                .bounding_box()
                .filter(|rect| rect.height == ROW_HEIGHT)
                .map(|rect| rect.y),
            _ => None,
        });
        assert_eq!(fills.collect::<Vec<_>>(), vec![0.0, ROW_HEIGHT * 2.0]);
    }
}
