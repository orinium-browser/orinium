//! Editable single-line text input with IME composition state.

use std::cell::RefCell;

use smol_str::SmolStr;

use crate::engine::bridge::text::{self, FallbackTextMeasurer, TextMeasureRequest};
use crate::engine::layouter::types::{Color, TextStyle};
use crate::engine::renderer_model::{Brush, DrawCommand, FillRule, Paint, rect_path};
use crate::engine::ui::custom_node::CustomNode;

const DEFAULT_WIDTH: f32 = 200.0;
const DEFAULT_HEIGHT: f32 = 28.0;
const INLINE_PADDING: f32 = 4.0;

/// Editing keys understood by engine-owned text inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextInputKey {
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
}

/// Platform-neutral text and IME input delivered to a custom node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextInputEvent {
    Insert(String),
    Preedit(String),
    Commit(String),
    Key(TextInputKey),
    CancelComposition,
}

/// Mutable editing state for a single-line text input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextInputState {
    pub value: String,
    pub preedit: String,
    pub caret: usize,
    pub focused: bool,
}

/// An HTML text input rendered by the engine.
#[derive(Debug)]
pub struct TextInputComponent {
    state: RefCell<TextInputState>,
    placeholder: SmolStr,
}

impl TextInputComponent {
    /// Creates a text input with an initial value and placeholder.
    pub fn new(value: impl Into<String>, placeholder: impl Into<SmolStr>) -> Self {
        let value = value.into();
        let caret = value.len();
        Self {
            state: RefCell::new(TextInputState {
                value,
                preedit: String::new(),
                caret,
                focused: false,
            }),
            placeholder: placeholder.into(),
        }
    }

    /// Returns a copy of the current editing state.
    pub fn state(&self) -> TextInputState {
        self.state.borrow().clone()
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

    fn handle_key(state: &mut TextInputState, key: TextInputKey) {
        match key {
            TextInputKey::Backspace if state.caret > 0 => {
                let previous = Self::previous_boundary(&state.value, state.caret);
                state.value.replace_range(previous..state.caret, "");
                state.caret = previous;
            }
            TextInputKey::Delete if state.caret < state.value.len() => {
                let next = Self::next_boundary(&state.value, state.caret);
                state.value.replace_range(state.caret..next, "");
            }
            TextInputKey::Left => {
                state.caret = Self::previous_boundary(&state.value, state.caret);
            }
            TextInputKey::Right => {
                state.caret = Self::next_boundary(&state.value, state.caret);
            }
            TextInputKey::Home => state.caret = 0,
            TextInputKey::End => state.caret = state.value.len(),
            TextInputKey::Backspace | TextInputKey::Delete => {}
        }
    }
}

impl CustomNode for TextInputComponent {
    fn draw(&self, cmd_buf: &mut Vec<DrawCommand>, text_style: &TextStyle) {
        self.draw_sized(cmd_buf, text_style, self.intrinsic_size());
    }

    fn draw_sized(&self, cmd_buf: &mut Vec<DrawCommand>, text_style: &TextStyle, size: (f32, f32)) {
        let state = self.state.borrow();
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
            &FallbackTextMeasurer::default(), // Stub implementation
            cmd_buf,
            display_text,
            INLINE_PADDING,
            ((size.1 - style.font_size) * 0.5).max(0.0),
            &style,
            caret,
            preedit,
            text_style.color,
            INLINE_PADDING,
            (size.1 - INLINE_PADDING * 2.0).max(0.0),
            (size.1 - 3.0).max(0.0),
        );
    }

    fn background_color(&self) -> Option<Color> {
        Some(Color(255, 255, 255, 255))
    }

    fn intrinsic_size(&self) -> (f32, f32) {
        (DEFAULT_WIDTH, DEFAULT_HEIGHT)
    }

    fn accepts_text_input(&self) -> bool {
        true
    }

    fn set_focused(&self, focused: bool) {
        let mut state = self.state.borrow_mut();
        state.focused = focused;
        if !focused {
            state.preedit.clear();
        }
    }

    fn is_focused(&self) -> bool {
        self.state.borrow().focused
    }

    fn handle_text_input(&self, event: TextInputEvent) -> bool {
        let mut state = self.state.borrow_mut();
        match event {
            TextInputEvent::Insert(text) | TextInputEvent::Commit(text) => {
                let text: String = text
                    .chars()
                    .filter(|character| !character.is_control())
                    .collect();
                let caret = state.caret;
                state.value.insert_str(caret, &text);
                state.caret += text.len();
                state.preedit.clear();
            }
            TextInputEvent::Preedit(text) => state.preedit = text,
            TextInputEvent::Key(key) => {
                state.preedit.clear();
                Self::handle_key(&mut state, key);
            }
            TextInputEvent::CancelComposition => state.preedit.clear(),
        }
        true
    }

    fn is_composing(&self) -> bool {
        !self.state.borrow().preedit.is_empty()
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

    // Handle offsets at the end of the text.
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

    #[test]
    fn preedit_is_replaced_by_ime_commit() {
        let input = TextInputComponent::new("abc", "");
        input.handle_text_input(TextInputEvent::Preedit("にほ".into()));
        assert_eq!(input.state().preedit, "にほ");

        input.handle_text_input(TextInputEvent::Commit("日本".into()));
        let state = input.state();
        assert_eq!(state.value, "abc日本");
        assert!(state.preedit.is_empty());
    }

    #[test]
    fn editing_uses_utf8_character_boundaries() {
        let input = TextInputComponent::new("a日b", "");
        input.handle_text_input(TextInputEvent::Key(TextInputKey::Left));
        input.handle_text_input(TextInputEvent::Key(TextInputKey::Backspace));
        assert_eq!(input.state().value, "ab");
    }
}
