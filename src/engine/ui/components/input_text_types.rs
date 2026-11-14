//! Text input event types and editing state.

/// Editing keys understood by engine-owned text inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputTextKey {
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
}

/// Platform-neutral text and IME input delivered to a custom node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputTextEvent {
    Insert(String),
    Preedit(String),
    Commit(String),
    Key(InputTextKey),
    Enter,
    Undo,
    Redo,
    Paste(String),
    CancelComposition,
    /// Select a range `[start, end)` of byte offsets into `value`.
    /// `None` clears the selection.
    Select {
        start: Option<usize>,
        end: Option<usize>,
    },
    Copy,
    Cut,
    SelectAll,
}

/// Mutable editing state for a single-line text input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputTextState {
    pub value: String,
    pub preedit: String,
    pub caret: usize,
    /// Selection start byte offset into `value`, or `None` if no selection.
    pub selection_start: Option<usize>,
    /// Selection end byte offset into `value`, or `None` if no selection.
    pub selection_end: Option<usize>,
    pub focused: bool,
}
