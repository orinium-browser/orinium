//! [`CustomNode`] trait for replaced elements that delegate rendering.

use ui_layout::Style;

use crate::engine::layouter::types::{Color, TextStyle};
use crate::engine::renderer_model::DrawCommand;

use super::text_input_types::TextInputEvent;

/// Trait for custom/replaced elements that produce their own draw commands.
///
/// # Coordinate System
/// Commands must be emitted in the content-box coordinate space:
/// `(0, 0)` = top-left of the content box. The parent's transform/clip
/// stack handles positioning.
///
/// # Lifecycle
/// - `draw_sized()` is called every frame during `generate_draw_commands`.
/// - Event handling (focus, IME) is dispatched through `engine::input`.
pub trait CustomNode: std::fmt::Debug + 'static {
    /// Emit draw commands fitted to the resolved content-box `size`.
    ///
    /// `text_style` carries the inherited CSS text properties (color,
    /// font-size, font-weight, etc.) resolved for this element.
    /// `style` carries the resolved `ui_layout::Style` (CSS width/height,
    /// box-sizing, etc.) and `size` is the resolved content-box size.
    ///
    /// This is the primary drawing entry point.
    fn draw_sized(
        &self,
        cmd_buf: &mut Vec<DrawCommand>,
        text_style: &TextStyle,
        style: &Style,
        size: (f32, f32),
    );

    /// Emit draw commands using the intrinsic content-box size.
    ///
    /// Defaults to [`draw_sized`](Self::draw_sized) with the intrinsic size
    /// and a default style. Components that only draw at their intrinsic size
    /// may override this instead.
    fn draw(&self, cmd_buf: &mut Vec<DrawCommand>, text_style: &TextStyle) {
        self.draw_sized(
            cmd_buf,
            text_style,
            &Style::default(),
            self.intrinsic_size(),
        );
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
