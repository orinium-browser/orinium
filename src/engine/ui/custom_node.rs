//! [`CustomNode`] trait for replaced elements that delegate rendering.

use crate::engine::layouter::types::{Color, TextStyle};
use crate::engine::renderer_model::DrawCommand;

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
}
