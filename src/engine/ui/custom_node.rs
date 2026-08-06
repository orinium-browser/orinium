//! [`CustomNode`] trait for replaced elements that delegate rendering.

use ui_layout::Style;

use crate::engine::layouter::types::{Background, TextStyle};
use crate::engine::renderer_model::DrawCommand;

use super::input_text_types::InputTextEvent;

/// Platform-neutral pointer event delivered to a custom node.
///
/// Coordinates are relative to the node's content box (see the
/// [`CustomNode`] coordinate system). The engine translates global
/// coordinates before dispatching.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointerEvent {
    /// Pointer moved to a new position inside the node.
    Move { x: f32, y: f32 },
    /// A pointer button was pressed inside the node.
    Down { x: f32, y: f32 },
    /// A pointer button was released inside the node.
    Up { x: f32, y: f32 },
    /// The pointer left the node's bounds.
    Leave,
}

/// A size expressed in the content-box coordinate system.
///
/// Used by the [`CustomNode`] trait so callers can distinguish content-box
/// dimensions from border-box ones at the type level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContentSize {
    /// Content-box width in pixels.
    pub width: f32,
    /// Content-box height in pixels.
    pub height: f32,
}

impl ContentSize {
    /// A zero-sized content box.
    pub fn zero() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
        }
    }
}

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
pub trait CustomNode: std::fmt::Debug + Send + Sync + 'static {
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
        size: ContentSize,
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

    /// Optional background override.
    ///
    /// When `Some`, the returned background replaces the CSS background for
    /// this element's box-model background.
    fn background(&self) -> Option<Background> {
        None
    }

    /// Intrinsic (content-box) size in pixels.
    ///
    /// The layout engine uses this to size the element when no explicit
    /// width/height is set via CSS.
    fn intrinsic_size(&self) -> ContentSize;

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
    fn handle_text_input(&self, _event: InputTextEvent) -> bool {
        false
    }

    /// Returns whether an IME preedit string is active.
    fn is_composing(&self) -> bool {
        false
    }

    /// Dispatches a platform-neutral pointer event.
    ///
    /// Coordinates are relative to the content box. Returns `true` when the
    /// node consumed the event.
    fn on_pointer_event(&self, _event: PointerEvent) -> bool {
        false
    }

    /// Updates the hover state for this node.
    fn set_hovered(&self, _hovered: bool) {}

    /// Returns whether this node is currently hovered.
    fn is_hovered(&self) -> bool {
        false
    }

    /// Whether this node changed its visual state since the last check.
    ///
    /// Consumes the flag: calling it again without an intervening state
    /// change returns `false`. The engine uses this to skip full redraws
    /// when no custom node is dirty.
    fn needs_repaint(&self) -> bool {
        false
    }

    /// Screen rectangle of the active IME composition underline, in content-box
    /// coordinates `(x, y, width, height)`. `None` when nothing is composing.
    fn composition_rect(&self) -> Option<(f32, f32, f32, f32)> {
        None
    }

    /// Accessibility role for this node (e.g. `"button"`, `"textbox"`, `"img"`).
    fn role(&self) -> Option<&'static str> {
        None
    }

    /// Accessibility label (accessible name).
    fn label(&self) -> Option<String> {
        None
    }

    /// Current value for editable/stateful nodes.
    fn value(&self) -> Option<String> {
        None
    }

    /// Whether this node is disabled and must not receive input.
    fn is_disabled(&self) -> bool {
        false
    }
}
