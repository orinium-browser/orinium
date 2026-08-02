use std::cell::Cell;
use std::sync::Arc;

use ui_layout::Style;

use crate::engine::{
    bridge::text::{self, TextMeasureRequest},
    layouter::types::{Background, Color, TextStyle},
    renderer_model::DrawCommand,
    ui::custom_node::{ContentSize, CustomNode, PointerEvent},
};

/// Default button size when the label cannot be measured.
const DEFAULT_BUTTON_SIZE: (f32, f32) = (120.0, 36.0);

/// An HTML button rendered by the engine.
pub struct ButtonComponent {
    pub label: String,
    pub button_color: Color,
    pub label_color: Color,
    measurer: Arc<dyn text::TextMeasurer<TextStyle>>,
    hovered: Cell<bool>,
    pressed: Cell<bool>,
    dirty: Cell<bool>,
}

impl std::fmt::Debug for ButtonComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ButtonComponent")
            .field("label", &self.label)
            .field("button_color", &self.button_color)
            .field("label_color", &self.label_color)
            .field("hovered", &self.hovered)
            .field("pressed", &self.pressed)
            .finish_non_exhaustive()
    }
}

impl ButtonComponent {
    pub fn new(
        label: impl Into<String>,
        button_color: Color,
        label_color: Color,
        measurer: Arc<dyn text::TextMeasurer<TextStyle>>,
    ) -> Self {
        Self {
            label: label.into(),
            button_color,
            label_color,
            measurer,
            hovered: Cell::new(false),
            pressed: Cell::new(false),
            dirty: Cell::new(true),
        }
    }
}

impl CustomNode for ButtonComponent {
    fn draw_sized(
        &self,
        cmd_buf: &mut Vec<DrawCommand>,
        text_style: &TextStyle,
        _style: &Style,
        size: ContentSize,
    ) {
        let mut style = text_style.clone();
        style.color = self.label_color;
        let y = ((size.height - style.font_size) * 0.5).max(0.0);
        cmd_buf.push(DrawCommand::DrawText {
            text: self.label.as_str().into(),
            x: 0.0,
            y,
            style,
        });
    }

    fn background(&self) -> Option<Background> {
        let base = self.button_color;
        let color = if self.pressed.get() {
            shade(base, -30)
        } else if self.hovered.get() {
            shade(base, 20)
        } else {
            base
        };
        Some(Background::Color(color))
    }

    fn intrinsic_size(&self) -> ContentSize {
        let text_style = TextStyle::default();
        let measured = self.measurer.measure(&TextMeasureRequest {
            text: self.label.clone(),
            style: text_style,
        });
        let (label_width, label_height) = measured.map_or_else(
            |_| DEFAULT_BUTTON_SIZE,
            |fragments| {
                let width: f32 = fragments.iter().map(|f| f.width).sum();
                let height = fragments.iter().map(|f| f.height).fold(0.0, f32::max);
                (width, height)
            },
        );
        ContentSize {
            width: label_width + 24.0,
            height: label_height.max(24.0) + 12.0,
        }
    }

    fn on_pointer_event(&self, event: PointerEvent) -> bool {
        match event {
            PointerEvent::Move { .. } => {
                let was_hovered = self.hovered.replace(true);
                if !was_hovered {
                    self.dirty.set(true);
                }
                true
            }
            PointerEvent::Down { .. } => {
                self.hovered.set(true);
                self.pressed.set(true);
                self.dirty.set(true);
                true
            }
            PointerEvent::Up { .. } => {
                let clicked = self.pressed.replace(false);
                self.hovered.set(false);
                self.dirty.set(true);
                clicked
            }
            PointerEvent::Leave => {
                let was_hovered = self.hovered.replace(false);
                let was_pressed = self.pressed.replace(false);
                if was_hovered || was_pressed {
                    self.dirty.set(true);
                }
                false
            }
        }
    }

    fn set_hovered(&self, hovered: bool) {
        if self.hovered.replace(hovered) != hovered {
            self.dirty.set(true);
        }
    }

    fn is_hovered(&self) -> bool {
        self.hovered.get()
    }

    fn needs_repaint(&self) -> bool {
        self.dirty.replace(false)
    }

    fn role(&self) -> Option<&'static str> {
        Some("button")
    }

    fn label(&self) -> Option<String> {
        Some(self.label.clone())
    }

    fn is_disabled(&self) -> bool {
        false
    }
}

fn shade(Color(r, g, b, a): Color, amount: i16) -> Color {
    let clamp = |channel: u8| (channel as i16 + amount).clamp(0, 255) as u8;
    Color(clamp(r), clamp(g), clamp(b), a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::bridge::text::FallbackTextMeasurer;

    fn component() -> ButtonComponent {
        ButtonComponent::new(
            "OK",
            Color(200, 200, 200, 255),
            Color(0, 0, 0, 255),
            Arc::new(FallbackTextMeasurer),
        )
    }

    #[test]
    fn intrinsic_size_measured_from_label() {
        let button = component();
        let size = button.intrinsic_size();
        // Fallback measurer produces a non-zero width for "OK".
        assert!(size.width > 24.0);
        assert!(size.height >= 24.0);
    }

    #[test]
    fn pointer_click_requires_down_then_up() {
        let button = component();
        assert!(!button.is_hovered());

        assert!(button.on_pointer_event(PointerEvent::Move { x: 5.0, y: 5.0 }));
        assert!(button.is_hovered());

        assert!(button.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 }));
        assert!(button.on_pointer_event(PointerEvent::Up { x: 5.0, y: 5.0 }));

        // Up without a prior Down must not report a click.
        assert!(!button.on_pointer_event(PointerEvent::Up { x: 5.0, y: 5.0 }));
    }

    #[test]
    fn leave_clears_hover_and_press() {
        let button = component();
        button.on_pointer_event(PointerEvent::Down { x: 0.0, y: 0.0 });
        button.on_pointer_event(PointerEvent::Leave);
        assert!(!button.is_hovered());
        // A later Up must not report a click.
        assert!(!button.on_pointer_event(PointerEvent::Up { x: 0.0, y: 0.0 }));
    }

    #[test]
    fn hover_state_via_set_hovered() {
        let button = component();
        button.set_hovered(true);
        assert!(button.is_hovered());
        button.set_hovered(false);
        assert!(!button.is_hovered());
    }

    #[test]
    fn background_changes_with_state() {
        let button = component();
        let normal = button.background();
        button.on_pointer_event(PointerEvent::Down { x: 0.0, y: 0.0 });
        let pressed = button.background();
        assert_ne!(normal, pressed);
    }

    #[test]
    fn exposes_role_and_label() {
        let button = component();
        assert_eq!(button.role(), Some("button"));
        assert_eq!(button.label(), Some("OK".to_string()));
        assert!(!button.is_disabled());
    }

    #[test]
    fn needs_repaint_tracks_visual_state_changes() {
        let button = component();
        // Fresh component is dirty.
        assert!(button.needs_repaint());
        assert!(!button.needs_repaint());

        // Moving into the button (hover on) marks it dirty.
        button.on_pointer_event(PointerEvent::Move { x: 5.0, y: 5.0 });
        assert!(button.needs_repaint());
        assert!(!button.needs_repaint());

        // Repeated moves while already hovered do not mark it dirty.
        button.on_pointer_event(PointerEvent::Move { x: 6.0, y: 6.0 });
        assert!(!button.needs_repaint());

        // Leave clears hover and marks it dirty again.
        button.on_pointer_event(PointerEvent::Leave);
        assert!(button.needs_repaint());
    }

    #[test]
    fn set_hovered_marks_dirty_on_change() {
        let button = component();
        button.needs_repaint();

        button.set_hovered(true);
        assert!(button.needs_repaint());
        button.set_hovered(true);
        assert!(!button.needs_repaint());
    }
}
