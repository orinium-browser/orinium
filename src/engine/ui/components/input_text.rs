//! Editable single-line text input with IME composition state.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use smol_str::SmolStr;
use ui_layout::Style;

use crate::engine::bridge::text::{self, TextMeasureRequest};
use crate::engine::layouter::types::{Color, TextFlowStyle, TextStyle};
use crate::engine::renderer_model::{Brush, DrawCommand, FillRule, Paint, rect_path};
use crate::engine::ui::components::input_text_types::{
    InputTextEvent, InputTextKey, InputTextState,
};
use crate::engine::ui::custom_node::{ContentSize, CustomNode};

/// Caret blink interval in milliseconds.
const CARET_BLINK_INTERVAL_MS: u64 = 500;

/// Callback invoked when the text input's value changes.
pub type OnValueChange = dyn Fn(&str) + Send + Sync;

/// Snapshot of the editing state used for undo/redo.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EditSnapshot {
    value: String,
    caret: usize,
    preedit: String,
}

/// An HTML text input rendered by the engine.
pub struct InputTextComponent {
    state: Mutex<InputTextState>,
    placeholder: SmolStr,
    measurer: Arc<dyn text::TextMeasurer>,
    undo_stack: Mutex<Vec<EditSnapshot>>,
    redo_stack: Mutex<Vec<EditSnapshot>>,
    dirty: AtomicBool,
    blink_visible: AtomicBool,
    blink_last_toggle: Mutex<Option<Instant>>,
    on_value_change: Option<Arc<OnValueChange>>,
    on_enter: Option<Arc<OnValueChange>>,
    on_copy: Option<Arc<OnValueChange>>,
    on_cut: Option<Arc<OnValueChange>>,
}

impl std::fmt::Debug for InputTextComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputTextComponent")
            .field("state", &self.state.lock().unwrap())
            .field("placeholder", &self.placeholder)
            .finish_non_exhaustive()
    }
}

impl InputTextComponent {
    /// Creates a text input with an initial value and placeholder.
    pub fn new(
        value: impl Into<String>,
        placeholder: impl Into<SmolStr>,
        measurer: Arc<dyn text::TextMeasurer>,
    ) -> Self {
        let value = value.into();
        let caret = value.len();
        Self {
            state: Mutex::new(InputTextState {
                value,
                preedit: String::new(),
                caret,
                focused: false,
                selection_start: None,
                selection_end: None,
            }),
            placeholder: placeholder.into(),
            measurer,
            undo_stack: Mutex::new(Vec::new()),
            redo_stack: Mutex::new(Vec::new()),
            dirty: AtomicBool::new(true),
            blink_visible: AtomicBool::new(true),
            blink_last_toggle: Mutex::new(None),
            on_value_change: None,
            on_enter: None,
            on_copy: None,
            on_cut: None,
        }
    }

    /// Creates a text input with a value change callback for DOM sync.
    pub fn with_on_change(
        value: impl Into<String>,
        placeholder: impl Into<SmolStr>,
        measurer: Arc<dyn text::TextMeasurer>,
        on_value_change: Arc<OnValueChange>,
    ) -> Self {
        let mut input = Self::new(value, placeholder, measurer);
        input.on_value_change = Some(on_value_change);
        input
    }

    /// Creates a text input that navigates when the user presses Enter.
    pub fn with_on_enter(
        value: impl Into<String>,
        placeholder: impl Into<SmolStr>,
        measurer: Arc<dyn text::TextMeasurer>,
        on_enter: Arc<OnValueChange>,
    ) -> Self {
        let mut input = Self::new(value, placeholder, measurer);
        input.on_enter = Some(on_enter);
        input
    }

    /// Replaces the current value and moves the caret to the end.
    ///
    /// Used to sync the URL bar with the active tab after navigation.
    pub fn set_value(&self, value: impl Into<String>) {
        let value = value.into();
        let mut state = self.state.lock().unwrap();
        state.value = value;
        state.caret = state.value.len();
        state.preedit.clear();
        state.selection_start = None;
        state.selection_end = None;
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// Returns a copy of the current editing state.
    pub fn state(&self) -> InputTextState {
        self.state.lock().unwrap().clone()
    }

    fn previous_boundary(value: &str, caret: usize) -> usize {
        value[..caret]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    fn next_boundary(value: &str, caret: usize) -> usize {
        value[caret..]
            .char_indices()
            .nth(1)
            .map_or(value.len(), |(offset, _)| caret + offset)
    }

    /// Clamp `caret` to a valid UTF-8 character boundary within `[0, value.len()]`.
    ///
    /// If `caret` is beyond the string length it is clamped to `value.len()`.
    /// If `caret` falls inside a multi-byte character it is moved to the
    /// preceding character boundary. Returns `0` for an empty string.
    fn clamp_caret(value: &str, caret: usize) -> usize {
        let len = value.len();
        if caret >= len {
            return len;
        }
        let mut last_boundary = 0;
        for (i, _) in value.char_indices() {
            if i > caret {
                break;
            }
            last_boundary = i;
        }
        last_boundary
    }

    fn handle_key(state: &mut InputTextState, key: InputTextKey) {
        // If a selection is active, Backspace/Delete removes the selected range
        // instead of a single character.
        if let (Some(start), Some(end)) = (state.selection_start, state.selection_end) {
            match key {
                InputTextKey::Backspace | InputTextKey::Delete => {
                    let start = Self::clamp_caret(&state.value, start);
                    let end = Self::clamp_caret(&state.value, end);
                    if end > start {
                        state.value.replace_range(start..end, "");
                    }
                    state.selection_start = None;
                    state.selection_end = None;
                    state.caret = Self::clamp_caret(&state.value, start);
                    return;
                }
                _ => {}
            }
        }

        match key {
            InputTextKey::Backspace if state.caret > 0 => {
                let previous = Self::previous_boundary(&state.value, state.caret);
                state.value.replace_range(previous..state.caret, "");
                state.caret = Self::clamp_caret(&state.value, previous);
            }
            InputTextKey::Delete if state.caret < state.value.len() => {
                let next = Self::next_boundary(&state.value, state.caret);
                state.value.replace_range(state.caret..next, "");
                state.caret = Self::clamp_caret(&state.value, state.caret);
            }
            InputTextKey::Left => {
                state.caret = Self::clamp_caret(
                    &state.value,
                    Self::previous_boundary(&state.value, state.caret),
                );
            }
            InputTextKey::Right => {
                state.caret =
                    Self::clamp_caret(&state.value, Self::next_boundary(&state.value, state.caret));
            }
            InputTextKey::Home => state.caret = 0,
            InputTextKey::End => state.caret = state.value.len(),
            InputTextKey::Backspace | InputTextKey::Delete => {}
        }
        state.caret = Self::clamp_caret(&state.value, state.caret);
    }

    fn snapshot(&self) -> EditSnapshot {
        let state = self.state.lock().unwrap();
        EditSnapshot {
            value: state.value.clone(),
            caret: state.caret,
            preedit: state.preedit.clone(),
        }
    }

    fn push_undo(&self, snapshot: EditSnapshot) {
        self.undo_stack.lock().unwrap().push(snapshot);
        self.redo_stack.lock().unwrap().clear();
    }

    fn undo(&self) {
        let Some(snapshot) = self.undo_stack.lock().unwrap().pop() else {
            return;
        };
        self.redo_stack.lock().unwrap().push(self.snapshot());
        let mut state = self.state.lock().unwrap();
        state.value = snapshot.value;
        state.caret = Self::clamp_caret(&state.value, snapshot.caret);
        state.preedit = snapshot.preedit;
        self.dirty.store(true, Ordering::Relaxed);
    }

    fn redo(&self) {
        let Some(snapshot) = self.redo_stack.lock().unwrap().pop() else {
            return;
        };
        self.undo_stack.lock().unwrap().push(self.snapshot());
        let mut state = self.state.lock().unwrap();
        state.value = snapshot.value;
        state.caret = Self::clamp_caret(&state.value, snapshot.caret);
        state.preedit = snapshot.preedit;
        self.dirty.store(true, Ordering::Relaxed);
    }
}

impl CustomNode for InputTextComponent {
    fn draw_sized(
        &self,
        cmd_buf: &mut Vec<DrawCommand>,
        text_style: &TextStyle,
        text_flow_style: &TextFlowStyle,
        _style: &Style,
        size: ContentSize,
    ) {
        let state = self.state.lock().unwrap();
        let mut style = text_style.clone();
        let (display_text, placeholder) = if state.value.is_empty() && state.preedit.is_empty() {
            (self.placeholder.as_str().to_owned(), true)
        } else {
            (
                format!(
                    "{}{}{}",
                    &state.value[..state.caret],
                    state.preedit,
                    &state.value[state.caret..]
                ),
                false,
            )
        };
        if placeholder {
            style.color = Color(128, 128, 128, 255);
        }

        let preedit =
            (!state.preedit.is_empty()).then_some((state.caret, state.caret + state.preedit.len()));
        // Caret blink animation: toggle visibility every CARET_BLINK_INTERVAL_MS
        // while focused. When unfocused the caret is hidden.
        let caret = if state.focused {
            let now = Instant::now();
            let mut last_toggle = self.blink_last_toggle.lock().unwrap();
            let should_toggle = match *last_toggle {
                Some(last)
                    if now.duration_since(last)
                        >= Duration::from_millis(CARET_BLINK_INTERVAL_MS) =>
                {
                    *last_toggle = Some(now);
                    true
                }
                None => {
                    *last_toggle = Some(now);
                    true
                }
                _ => false,
            };
            if should_toggle {
                self.blink_visible.store(
                    !self.blink_visible.load(Ordering::Relaxed),
                    Ordering::Relaxed,
                );
            }
            if self.blink_visible.load(Ordering::Relaxed) {
                Some(state.caret + state.preedit.len())
            } else {
                None
            }
        } else {
            None
        };

        draw_text_input(
            &*self.measurer,
            cmd_buf,
            display_text,
            0.0,
            ((size.height - text_flow_style.font_size) * 0.5).max(0.0),
            &style,
            text_flow_style,
            caret,
            preedit,
            text_style.color,
            0.0,
            size.height.max(0.0),
            (size.height - 1.0).max(0.0),
        );
    }

    fn intrinsic_size(&self) -> ContentSize {
        ContentSize {
            width: 200.0,
            height: 28.0,
        }
    }

    fn accepts_text_input(&self) -> bool {
        true
    }

    fn set_focused(&self, focused: bool) {
        let mut state = self.state.lock().unwrap();
        state.focused = focused;
        self.dirty.store(true, Ordering::Relaxed);
        if !focused {
            state.preedit.clear();
        }
    }

    fn is_focused(&self) -> bool {
        self.state.lock().unwrap().focused
    }

    fn handle_text_input(&self, event: InputTextEvent) -> bool {
        match event {
            InputTextEvent::Insert(text)
            | InputTextEvent::Commit(text)
            | InputTextEvent::Paste(text) => {
                let text: String = text
                    .chars()
                    .filter(|character| !character.is_control())
                    .collect();
                if text.is_empty() {
                    return true;
                }
                self.push_undo(self.snapshot());
                let mut state = self.state.lock().unwrap();
                // If a selection is active, replace it with the inserted text.
                if let (Some(start), Some(end)) = (state.selection_start, state.selection_end) {
                    let start = Self::clamp_caret(&state.value, start);
                    let end = Self::clamp_caret(&state.value, end);
                    if end > start {
                        state.value.replace_range(start..end, "");
                        state.selection_start = None;
                        state.selection_end = None;
                        state.caret = Self::clamp_caret(&state.value, start);
                    }
                }
                let caret = Self::clamp_caret(&state.value, state.caret);
                state.value.insert_str(caret, &text);
                state.caret = Self::clamp_caret(&state.value, caret + text.len());
                state.preedit.clear();
                let value = state.value.clone();
                drop(state);
                self.dirty.store(true, Ordering::Relaxed);
                if let Some(ref cb) = self.on_value_change {
                    cb(&value);
                }
            }
            InputTextEvent::Preedit(text) => {
                let mut state = self.state.lock().unwrap();
                if state.preedit != text {
                    state.preedit = text;
                    self.dirty.store(true, Ordering::Relaxed);
                }
            }
            InputTextEvent::Key(key) => {
                let changed = matches!(key, InputTextKey::Backspace | InputTextKey::Delete);
                if changed {
                    self.push_undo(self.snapshot());
                }
                let mut state = self.state.lock().unwrap();
                state.preedit.clear();
                Self::handle_key(&mut state, key);
                let value = state.value.clone();
                drop(state);
                self.dirty.store(true, Ordering::Relaxed);
                if changed && let Some(ref cb) = self.on_value_change {
                    cb(&value);
                }
            }
            InputTextEvent::Enter => {
                let mut state = self.state.lock().unwrap();
                if !state.preedit.is_empty() {
                    state.preedit.clear();
                    self.dirty.store(true, Ordering::Relaxed);
                }
                let value = state.value.clone();
                drop(state);
                if let Some(ref cb) = self.on_enter {
                    cb(&value);
                }
            }
            InputTextEvent::Undo => {
                self.undo();
            }
            InputTextEvent::Redo => {
                self.redo();
            }
            InputTextEvent::CancelComposition => {
                let mut state = self.state.lock().unwrap();
                if !state.preedit.is_empty() {
                    state.preedit.clear();
                    self.dirty.store(true, Ordering::Relaxed);
                }
            }
            InputTextEvent::Select { start, end } => {
                let mut state = self.state.lock().unwrap();
                let len = state.value.len();
                let start = start.map(|s| Self::clamp_caret(&state.value, s));
                let end = end.map(|e| Self::clamp_caret(&state.value, e));
                match (start, end) {
                    (Some(s), Some(e)) if s <= e => {
                        state.selection_start = Some(s);
                        state.selection_end = Some(e);
                        state.caret = Self::clamp_caret(&state.value, e);
                    }
                    (Some(s), None) => {
                        state.selection_start = Some(s);
                        state.selection_end = Some(len);
                        state.caret = Self::clamp_caret(&state.value, len);
                    }
                    (None, Some(e)) => {
                        state.selection_start = Some(0);
                        state.selection_end = Some(e);
                        state.caret = Self::clamp_caret(&state.value, e);
                    }
                    _ => {
                        // Both None or invalid: clear selection.
                        state.selection_start = None;
                        state.selection_end = None;
                    }
                }
                self.dirty.store(true, Ordering::Relaxed);
            }
            InputTextEvent::Copy => {
                let state = self.state.lock().unwrap();
                if let (Some(start), Some(end)) = (state.selection_start, state.selection_end) {
                    let start = Self::clamp_caret(&state.value, start);
                    let end = Self::clamp_caret(&state.value, end);
                    if end > start
                        && let Some(ref cb) = self.on_copy
                    {
                        cb(&state.value[start..end]);
                    }
                }
            }
            InputTextEvent::Cut => {
                self.push_undo(self.snapshot());
                let mut state = self.state.lock().unwrap();
                if let (Some(start), Some(end)) = (state.selection_start, state.selection_end) {
                    let start = Self::clamp_caret(&state.value, start);
                    let end = Self::clamp_caret(&state.value, end);
                    if end > start {
                        let selected = state.value[start..end].to_string();
                        state.value.replace_range(start..end, "");
                        state.selection_start = None;
                        state.selection_end = None;
                        state.caret = Self::clamp_caret(&state.value, start);
                        self.dirty.store(true, Ordering::Relaxed);
                        if let Some(ref cb) = self.on_copy {
                            cb(&selected);
                        }
                        if let Some(ref cb) = self.on_cut {
                            cb(&selected);
                        }
                        return true;
                    }
                }
                state.selection_start = None;
                state.selection_end = None;
                self.dirty.store(true, Ordering::Relaxed);
            }
            InputTextEvent::SelectAll => {
                let mut state = self.state.lock().unwrap();
                let len = state.value.len();
                if len > 0 {
                    state.selection_start = Some(0);
                    state.selection_end = Some(len);
                    state.caret = len;
                } else {
                    state.selection_start = None;
                    state.selection_end = None;
                    state.caret = 0;
                }
                self.dirty.store(true, Ordering::Relaxed);
            }
        }
        true
    }

    fn is_composing(&self) -> bool {
        !self.state.lock().unwrap().preedit.is_empty()
    }

    fn needs_repaint(&self) -> bool {
        // When focused, always request repaint so the caret blink animation
        // continues. When unfocused, only repaint on explicit dirtiness.
        if self.is_focused() {
            return true;
        }
        self.dirty.swap(false, Ordering::Relaxed)
    }

    fn composition_rect(&self) -> Option<(f32, f32, f32, f32)> {
        let state = self.state.lock().unwrap();
        if state.preedit.is_empty() {
            return None;
        }
        let display_text = format!(
            "{}{}{}",
            &state.value[..state.caret],
            state.preedit,
            &state.value[state.caret..]
        );
        let style = TextStyle::default();
        let Ok(fragments) = self.measurer.measure(&TextMeasureRequest {
            text: display_text,
            attribute: text::TextAttribute {
                style,
                flow_style: TextFlowStyle::default(),
            },
        }) else {
            return None;
        };
        let mut byte_offset = 0;
        let mut width = 0.0;
        let mut start_x = None;
        let mut preedit_width = 0.0;
        let preedit_start = state.caret;
        let preedit_end = state.caret + state.preedit.len();
        for fragment in &fragments {
            let frag_start = byte_offset;
            let frag_end = byte_offset + fragment.text.len();
            if start_x.is_none() && frag_start >= preedit_start {
                start_x = Some(width);
            }
            if frag_end <= preedit_end && frag_start < preedit_end {
                preedit_width += fragment.width;
            }
            byte_offset = frag_end;
            width += fragment.width;
        }
        let start_x = start_x.unwrap_or(width);
        Some((start_x, 0.0, preedit_width.max(0.0), 28.0_f32.max(0.0)))
    }

    fn role(&self) -> Option<&'static str> {
        Some("textbox")
    }

    fn label(&self) -> Option<String> {
        (!self.placeholder.is_empty()).then(|| self.placeholder.to_string())
    }

    fn value(&self) -> Option<String> {
        Some(self.state.lock().unwrap().value.clone())
    }
}

/// Draw text input decorations such as caret and IME preedit underline.
#[allow(clippy::too_many_arguments)]
fn draw_text_input(
    measurer: &dyn text::TextMeasurer,
    cmd_buf: &mut Vec<DrawCommand>,
    text: String,
    x: f32,
    y: f32,
    style: &TextStyle,
    flow_style: &TextFlowStyle,
    caret: Option<usize>,
    preedit: Option<(usize, usize)>,
    decoration_color: Color,
    caret_top: f32,
    caret_height: f32,
    underline_y: f32,
) {
    let Ok(fragments) = measurer.measure(&TextMeasureRequest {
        text: text.clone(),
        attribute: text::TextAttribute {
            style: style.clone(),
            flow_style: *flow_style,
        },
    }) else {
        return;
    };

    let mut caret_x = None;
    let mut preedit_start_x = None;
    let mut preedit_end_x = None;

    let mut byte_offset = 0;
    let mut width = 0.0;

    for fragment in &fragments {
        if let Some(caret_pos) = caret
            && caret_x.is_none()
            && caret_pos <= byte_offset
        {
            caret_x = Some(width);
        }

        if let Some((start, end)) = preedit {
            if preedit_start_x.is_none() && start <= byte_offset {
                preedit_start_x = Some(width);
            }

            if preedit_end_x.is_none() && end <= byte_offset {
                preedit_end_x = Some(width);
            }
        }

        byte_offset += fragment.text.len();
        width += fragment.width;
    }

    if let Some(caret_pos) = caret
        && caret_x.is_none()
        && caret_pos <= byte_offset
    {
        caret_x = Some(width);
    }

    if let Some((start, end)) = preedit {
        if preedit_start_x.is_none() && start <= byte_offset {
            preedit_start_x = Some(width);
        }

        if preedit_end_x.is_none() && end <= byte_offset {
            preedit_end_x = Some(width);
        }
    }

    let paint = Paint {
        brush: Brush::Solid(decoration_color),
        opacity: 1.0,
    };

    if let (Some(start_x), Some(end_x)) = (preedit_start_x, preedit_end_x) {
        cmd_buf.push(DrawCommand::Fill {
            path: rect_path(x + start_x, underline_y, (end_x - start_x).max(0.0), 1.0),
            rule: FillRule::NonZero,
            paint: paint.clone(),
        });
    }

    if let Some(caret_x) = caret_x {
        cmd_buf.push(DrawCommand::Fill {
            path: rect_path(x + caret_x, caret_top, 1.0, caret_height),
            rule: FillRule::NonZero,
            paint,
        });
    }

    cmd_buf.push(DrawCommand::DrawText {
        x,
        y,
        text: text.into(),
        style: style.clone(),
        flow_style: *flow_style,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::engine::bridge::text::FallbackTextMeasurer;

    fn make_component(value: &str, placeholder: &str) -> InputTextComponent {
        InputTextComponent::new(value, placeholder, Arc::new(FallbackTextMeasurer))
    }

    #[test]
    fn preedit_is_replaced_by_ime_commit() {
        let input = make_component("abc", "");
        input.handle_text_input(InputTextEvent::Preedit("にほ".into()));
        assert_eq!(input.state().preedit, "にほ");

        input.handle_text_input(InputTextEvent::Commit("日本".into()));
        let state = input.state();
        assert_eq!(state.value, "abc日本");
        assert!(state.preedit.is_empty());
    }

    #[test]
    fn editing_uses_utf8_character_boundaries() {
        let input = make_component("a日b", "");
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Left));
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Backspace));
        assert_eq!(input.state().value, "ab");
    }

    #[test]
    fn undo_redo_restores_value_and_caret() {
        let input = make_component("", "");
        input.handle_text_input(InputTextEvent::Insert("abc".into()));
        assert_eq!(input.state().value, "abc");

        input.handle_text_input(InputTextEvent::Undo);
        assert_eq!(input.state().value, "");

        input.handle_text_input(InputTextEvent::Redo);
        assert_eq!(input.state().value, "abc");
    }

    #[test]
    fn paste_inserts_at_caret() {
        let input = make_component("ab", "");
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Left));
        input.handle_text_input(InputTextEvent::Paste("XY".into()));
        assert_eq!(input.state().value, "aXYb");
    }

    #[test]
    fn enter_keeps_value_and_clears_preedit() {
        let input = make_component("abc", "");
        input.handle_text_input(InputTextEvent::Preedit("にほ".into()));
        input.handle_text_input(InputTextEvent::Enter);
        let state = input.state();
        assert_eq!(state.value, "abc");
        assert!(state.preedit.is_empty());
    }

    #[test]
    fn role_and_value_for_accessibility() {
        let input = make_component("hello", "Name");
        assert_eq!(input.role(), Some("textbox"));
        assert_eq!(input.label(), Some("Name".to_string()));
        assert_eq!(input.value(), Some("hello".to_string()));
    }

    #[test]
    fn on_value_change_callback_fires() {
        use std::sync::{Arc, Mutex};
        let received: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let received_clone = Arc::clone(&received);
        let cb: Arc<OnValueChange> = Arc::new(move |v: &str| {
            received_clone.lock().unwrap().push(v.to_string());
        });
        let input = InputTextComponent::with_on_change("", "", Arc::new(FallbackTextMeasurer), cb);

        input.handle_text_input(InputTextEvent::Insert("hello".into()));
        assert_eq!(*received.lock().unwrap(), vec!["hello"]);

        input.handle_text_input(InputTextEvent::Insert(" world".into()));
        assert_eq!(*received.lock().unwrap(), vec!["hello", "hello world"]);
    }

    #[test]
    fn on_enter_callback_fires_with_current_value() {
        use std::sync::{Arc, Mutex};
        let received: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let received_clone = Arc::clone(&received);
        let cb: Arc<OnValueChange> = Arc::new(move |v: &str| {
            received_clone.lock().unwrap().push(v.to_string());
        });
        let input = InputTextComponent::with_on_enter("", "", Arc::new(FallbackTextMeasurer), cb);

        input.handle_text_input(InputTextEvent::Insert("https://example.com".into()));
        input.handle_text_input(InputTextEvent::Enter);
        assert_eq!(*received.lock().unwrap(), vec!["https://example.com"]);
    }

    #[test]
    fn set_value_updates_value_and_caret() {
        let input = make_component("old", "");
        input.set_value("new value");
        let state = input.state();
        assert_eq!(state.value, "new value");
        assert_eq!(state.caret, "new value".len());
        assert!(input.needs_repaint());
    }

    #[test]
    fn caret_clamped_after_backspace_at_start() {
        let input = make_component("abc", "");
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Backspace));
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Backspace));
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Backspace));
        let state = input.state();
        assert_eq!(state.value, "");
        assert_eq!(state.caret, 0);
    }

    #[test]
    fn caret_clamped_after_delete_at_end() {
        let input = make_component("abc", "");
        // Delete at end does nothing (nothing after caret)
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Delete));
        let state = input.state();
        assert_eq!(state.value, "abc");
        assert_eq!(state.caret, 3);

        // Move left, then delete removes characters
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Left));
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Delete));
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Left));
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Delete));
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Left));
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Delete));
        let state = input.state();
        assert_eq!(state.value, "");
        assert_eq!(state.caret, 0);
    }

    #[test]
    fn caret_clamped_after_set_value_shortens_text() {
        let input = make_component("hello world", "");
        input.set_value("hi");
        let state = input.state();
        assert_eq!(state.value, "hi");
        assert_eq!(state.caret, 2);
    }

    #[test]
    fn caret_clamped_after_undo_from_stale_snapshot() {
        let input = make_component("abc", "");
        input.handle_text_input(InputTextEvent::Insert("日".into()));
        assert_eq!(input.state().value, "abc日");
        assert_eq!(input.state().caret, 6);

        // Undo should restore "abc" with caret clamped to 3
        input.handle_text_input(InputTextEvent::Undo);
        let state = input.state();
        assert_eq!(state.value, "abc");
        assert_eq!(state.caret, 3);
    }

    #[test]
    fn caret_clamped_after_redo_from_stale_snapshot() {
        let input = make_component("abc", "");
        input.handle_text_input(InputTextEvent::Insert("日".into()));
        input.handle_text_input(InputTextEvent::Undo);
        let state = input.state();
        assert_eq!(state.value, "abc");
        assert_eq!(state.caret, 3);

        // Redo should restore "abc日" with caret at 6
        input.handle_text_input(InputTextEvent::Redo);
        let state = input.state();
        assert_eq!(state.value, "abc日");
        assert_eq!(state.caret, 6);
    }

    #[test]
    fn caret_stays_at_utf8_boundary_with_multibyte_chars() {
        let input = make_component("a日b", "");
        // Move caret left from end (6) to before 'b' (4)
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Left));
        assert_eq!(input.state().caret, 4);
        // Move left again to before '日' (1)
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Left));
        assert_eq!(input.state().caret, 1);
        // Move left again to before 'a' (0)
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Left));
        assert_eq!(input.state().caret, 0);
        // Move right to before '日' (1)
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Right));
        assert_eq!(input.state().caret, 1);
        // Move right to before 'b' (4)
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Right));
        assert_eq!(input.state().caret, 4);
        // Move right to end (5)
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Right));
        assert_eq!(input.state().caret, 5);
    }

    #[test]
    fn select_all_selects_entire_text() {
        let input = make_component("abc", "");
        input.handle_text_input(InputTextEvent::SelectAll);
        let state = input.state();
        assert_eq!(state.selection_start, Some(0));
        assert_eq!(state.selection_end, Some(3));
        assert_eq!(state.caret, 3);
    }

    #[test]
    fn select_all_on_empty_clears_selection() {
        let input = make_component("", "");
        input.handle_text_input(InputTextEvent::SelectAll);
        let state = input.state();
        assert_eq!(state.selection_start, None);
        assert_eq!(state.selection_end, None);
        assert_eq!(state.caret, 0);
    }

    #[test]
    fn select_none_clears_selection() {
        let input = make_component("abc", "");
        input.handle_text_input(InputTextEvent::SelectAll);
        assert!(input.state().selection_start.is_some());
        input.handle_text_input(InputTextEvent::Select {
            start: None,
            end: None,
        });
        let state = input.state();
        assert_eq!(state.selection_start, None);
        assert_eq!(state.selection_end, None);
    }

    #[test]
    fn typing_with_selection_replaces_selection() {
        let input = make_component("abc", "");
        input.handle_text_input(InputTextEvent::SelectAll);
        input.handle_text_input(InputTextEvent::Insert("X".into()));
        let state = input.state();
        assert_eq!(state.value, "X");
        assert_eq!(state.caret, 1);
        assert_eq!(state.selection_start, None);
        assert_eq!(state.selection_end, None);
    }

    #[test]
    fn backspace_with_selection_deletes_selection() {
        let input = make_component("abc", "");
        input.handle_text_input(InputTextEvent::SelectAll);
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Backspace));
        let state = input.state();
        assert_eq!(state.value, "");
        assert_eq!(state.caret, 0);
        assert_eq!(state.selection_start, None);
        assert_eq!(state.selection_end, None);
    }

    #[test]
    fn delete_with_selection_deletes_selection() {
        let input = make_component("abc", "");
        input.handle_text_input(InputTextEvent::SelectAll);
        input.handle_text_input(InputTextEvent::Key(InputTextKey::Delete));
        let state = input.state();
        assert_eq!(state.value, "");
        assert_eq!(state.caret, 0);
        assert_eq!(state.selection_start, None);
        assert_eq!(state.selection_end, None);
    }

    #[test]
    fn copy_fires_callback_with_selected_text() {
        use std::sync::{Arc, Mutex};
        let received: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let received_clone = Arc::clone(&received);
        let cb: Arc<OnValueChange> = Arc::new(move |v: &str| {
            received_clone.lock().unwrap().push(v.to_string());
        });
        let mut input = make_component("abc", "");
        input.on_copy = Some(cb);
        input.handle_text_input(InputTextEvent::SelectAll);
        input.handle_text_input(InputTextEvent::Copy);
        assert_eq!(*received.lock().unwrap(), vec!["abc"]);
    }

    #[test]
    fn cut_fires_callbacks_and_removes_selection() {
        use std::sync::{Arc, Mutex};
        let received: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let received_clone = Arc::clone(&received);
        let cb: Arc<OnValueChange> = Arc::new(move |v: &str| {
            received_clone.lock().unwrap().push(v.to_string());
        });
        let mut input = make_component("abc", "");
        input.on_copy = Some(cb.clone());
        input.on_cut = Some(cb);
        input.handle_text_input(InputTextEvent::SelectAll);
        input.handle_text_input(InputTextEvent::Cut);
        assert_eq!(*received.lock().unwrap(), vec!["abc", "abc"]);
        let state = input.state();
        assert_eq!(state.value, "");
        assert_eq!(state.caret, 0);
        assert_eq!(state.selection_start, None);
        assert_eq!(state.selection_end, None);
    }

    #[test]
    fn set_value_clears_selection() {
        let input = make_component("abc", "");
        input.handle_text_input(InputTextEvent::SelectAll);
        assert!(input.state().selection_start.is_some());
        input.set_value("xyz");
        let state = input.state();
        assert_eq!(state.value, "xyz");
        assert_eq!(state.caret, 3);
        assert_eq!(state.selection_start, None);
        assert_eq!(state.selection_end, None);
    }

    #[test]
    fn partial_selection_insert_replaces_range() {
        let input = make_component("hello", "");
        input.handle_text_input(InputTextEvent::Select {
            start: Some(1),
            end: Some(4),
        });
        input.handle_text_input(InputTextEvent::Insert("X".into()));
        let state = input.state();
        assert_eq!(state.value, "hXo");
        assert_eq!(state.caret, 2);
        assert_eq!(state.selection_start, None);
        assert_eq!(state.selection_end, None);
    }
}
