//! [`CustomNode`] trait for replaced elements that delegate rendering.

use crate::engine::layouter::types::{Color, TextStyle};
use crate::engine::renderer_model::DrawCommand;

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

/// Trait for custom/replaced elements that produce their own draw commands.
///
/// # Coordinate System
/// Commands must be emitted in the content-box coordinate space:
/// `(0, 0)` = top-left of the content box. The parent's transform/clip
/// stack handles positioning.
///
/// # Lifecycle (Phase 1)
/// - `draw()` is called every frame during `generate_draw_commands`.
/// - Event handling is deferred to a later phase.
pub trait CustomNode: std::fmt::Debug + 'static {
    /// Emit draw commands into `cmd_buf`.
    ///
    /// `text_style` carries the inherited CSS text properties (color,
    /// font-size, font-weight, etc.) resolved for this element.
    ///
    /// Called once per frame for each visible custom node.
    fn draw(&self, cmd_buf: &mut Vec<DrawCommand>, text_style: &TextStyle);

    /// Emit draw commands fitted to the resolved content-box `size`.
    ///
    /// Replaced elements that support CSS sizing can override this method.
    /// The default preserves the original intrinsic-size drawing behavior.
    fn draw_sized(
        &self,
        cmd_buf: &mut Vec<DrawCommand>,
        text_style: &TextStyle,
        _size: (f32, f32),
    ) {
        self.draw(cmd_buf, text_style);
    }

    /// Optional background color override.
    ///
    /// When `Some`, the returned color replaces the CSS `background-color`
    /// for this element's box-model background.
    fn background_color(&self) -> Option<Color> {
        None
    }

    /// Intrinsic (content-box) size in pixels `(width, height)`.
    ///
    /// The layout engine uses this to size the element when no explicit
    /// width/height is set via CSS.
    fn intrinsic_size(&self) -> (f32, f32);

    /// Whether one resolved dimension should scale the other dimension using
    /// the node's intrinsic aspect ratio.
    fn preserves_intrinsic_aspect_ratio(&self) -> bool {
        false
    }

    /// Whether this node can receive keyboard and IME text input.
    fn accepts_text_input(&self) -> bool {
        false
    }

    /// Updates keyboard focus for this node.
    fn set_focused(&self, _focused: bool) {}

    /// Returns whether this node currently owns keyboard focus.
    fn is_focused(&self) -> bool {
        false
    }

    /// Applies a platform-neutral text editing event.
    fn handle_text_input(&self, _event: TextInputEvent) -> bool {
        false
    }

    /// Returns whether an IME preedit string is active.
    fn is_composing(&self) -> bool {
        false
    }
}
