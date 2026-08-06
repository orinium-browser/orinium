//! Editable single-line text input with IME composition state.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use smol_str::SmolStr;
use ui_layout::Style;

use crate::engine::bridge::text::{self, TextMeasureRequest};
use crate::engine::layouter::types::{Background, Color, TextStyle};
use crate::engine::renderer_model::{Brush, DrawCommand, FillRule, Paint, rect_path};
use crate::engine::ui::components::input_text_types::{
    InputTextEvent, InputTextKey, InputTextState,
};
use crate::engine::ui::custom_node::{ContentSize, CustomNode};

/// Callback invoked when the text input's value changes.
pub type OnValueChange = dyn Fn(&str) + Send + Sync;

const INLINE_PADDING: f32 = 4.0;

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
    measurer: Arc<dyn text::TextMeasurer<TextStyle>>,
    undo_stack: Mutex<Vec<EditSnapshot>>,
    redo_stack: Mutex<Vec<EditSnapshot>>,
    dirty: AtomicBool,
    on_value_change: Option<Arc<OnValueChange>>,
    on_enter: Option<Arc<OnValueChange>>,
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
        measurer: Arc<dyn text::TextMeasurer<TextStyle>>,
    ) -> Self {
        let value = value.into();
        let caret = value.len();
        Self {
            state: Mutex::new(InputTextState {
                value,
                preedit: String::new(),
                caret,
                focused: false,
            }),
            placeholder: placeholder.into(),
            measurer,
            undo_stack: Mutex::new(Vec::new()),
            redo_stack: Mutex::new(Vec::new()),
            dirty: AtomicBool::new(true),
            on_value_change: None,
            on_enter: None,
        }
    }

    /// Creates a text input with a value change callback for DOM sync.
    pub fn with_on_change(
        value: impl Into<String>,
        placeholder: impl Into<SmolStr>,
        measurer: Arc<dyn text::TextMeasurer<TextStyle>>,
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
        measurer: Arc<dyn text::TextMeasurer<TextStyle>>,
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

    fn handle_key(state: &mut InputTextState, key: InputTextKey) {
        match key {
            InputTextKey::Backspace if state.caret > 0 => {
                let previous = Self::previous_boundary(&state.value, state.caret);
                state.value.replace_range(previous..state.caret, "");
                state.caret = previous;
            }
            InputTextKey::Delete if state.caret < state.value.len() => {
                let next = Self::next_boundary(&state.value, state.caret);
                state.value.replace_range(state.caret..next, "");
            }
            InputTextKey::Left => {
                state.caret = Self::previous_boundary(&state.value, state.caret);
            }
            InputTextKey::Right => {
                state.caret = Self::next_boundary(&state.value, state.caret);
            }
            InputTextKey::Home => state.caret = 0,
            InputTextKey::End => state.caret = state.value.len(),
            InputTextKey::Backspace | InputTextKey::Delete => {}
        }
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
        state.caret = snapshot.caret;
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
        state.caret = snapshot.caret;
        state.preedit = snapshot.preedit;
        self.dirty.store(true, Ordering::Relaxed);
    }
}

impl CustomNode for InputTextComponent {
    fn draw_sized(
        &self,
        cmd_buf: &mut Vec<DrawCommand>,
        text_style: &TextStyle,
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
        let caret = state.focused.then_some(state.caret + state.preedit.len());

        draw_text_input(
            &*self.measurer,
            cmd_buf,
            display_text,
            INLINE_PADDING,
            ((size.height - style.font_size) * 0.5).max(0.0),
            &style,
            caret,
            preedit,
            text_style.color,
            INLINE_PADDING,
            (size.height - INLINE_PADDING * 2.0).max(0.0),
            (size.height - 3.0).max(0.0),
        );
    }

    fn background(&self) -> Option<Background> {
        Some(Background::Color(Color(255, 255, 255, 255)))
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
                let caret = state.caret;
                state.value.insert_str(caret, &text);
                state.caret += text.len();
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
        }
        true
    }

    fn is_composing(&self) -> bool {
        !self.state.lock().unwrap().preedit.is_empty()
    }

    fn needs_repaint(&self) -> bool {
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
            style,
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
        Some((start_x, 0.0, preedit_width.max(0.0), 1.0))
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
    measurer: &dyn text::TextMeasurer<TextStyle>,
    cmd_buf: &mut Vec<DrawCommand>,
    text: String,
    x: f32,
    y: f32,
    style: &TextStyle,
    caret: Option<usize>,
    preedit: Option<(usize, usize)>,
    decoration_color: Color,
    caret_top: f32,
    caret_height: f32,
    underline_y: f32,
) {
    let Ok(fragments) = measurer.measure(&TextMeasureRequest {
        text: text.clone(),
        style: style.clone(),
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
    fn background_is_white() {
        let input = make_component("", "");
        assert_eq!(
            input.background(),
            Some(Background::Color(Color(255, 255, 255, 255)))
        );
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
}
