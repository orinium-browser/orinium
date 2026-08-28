use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ui_layout::Style;

use crate::layouter::types::TextFlowStyle;
use crate::{
    bridge::text::{self, TextMeasureRequest},
    layouter::types::{Color, TextStyle},
    renderer_model::{Brush, DrawCommand, FillRule, Paint, rect_path},
    ui::custom_node::{ContentSize, CustomNode, PointerEvent},
};

/// Default button size when the label cannot be measured.
const DEFAULT_BUTTON_SIZE: (f32, f32) = (120.0, 36.0);

/// An HTML button rendered by the engine.
pub struct ButtonComponent {
    pub label: String,
    pub button_color: Color,
    pub label_color: Color,
    measurer: Arc<dyn text::TextMeasurer>,
    measured_cache: Mutex<Option<(f32, f32)>>,
    hovered: AtomicBool,
    pressed: AtomicBool,
    dirty: AtomicBool,
    label_dirty: AtomicBool,
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
        measurer: Arc<dyn text::TextMeasurer>,
    ) -> Self {
        Self {
            label: label.into(),
            button_color,
            label_color,
            measurer,
            measured_cache: Mutex::new(None),
            hovered: AtomicBool::new(false),
            pressed: AtomicBool::new(false),
            dirty: AtomicBool::new(true),
            label_dirty: AtomicBool::new(false),
        }
    }
}

impl CustomNode for ButtonComponent {
    fn draw_sized(
        &self,
        cmd_buf: &mut Vec<DrawCommand>,
        text_style: &TextStyle,
        text_flow_style: &TextFlowStyle,
        _style: &Style,
        size: ContentSize,
    ) {
        let base = self.button_color;
        let bg = if self.pressed.load(Ordering::Relaxed) {
            shade(base, -30)
        } else if self.hovered.load(Ordering::Relaxed) {
            shade(base, 20)
        } else {
            base
        };
        if bg.3 > 0 {
            cmd_buf.push(DrawCommand::Fill {
                path: rect_path(0.0, 0.0, size.width, size.height),
                rule: FillRule::NonZero,
                paint: Paint {
                    brush: Brush::Solid(bg),
                    opacity: 1.0,
                },
            });
        }

        let mut style = text_style.clone();
        style.color = self.label_color;
        let y = ((size.height - text_flow_style.font_size) * 0.5).max(0.0);
        cmd_buf.push(DrawCommand::DrawText {
            x: 0.0,
            y,
            text: self.label.as_str().into(),
            style,
            flow_style: *text_flow_style,
        });
    }

    fn intrinsic_size(&self) -> ContentSize {
        let text_style = TextStyle::default();
        let cached_size = self.measured_cache.lock().ok().and_then(|cache| *cache);

        let (label_width, label_height) = if !self.label_dirty.load(Ordering::Relaxed)
            && let Some((label_width, label_height)) = cached_size
        {
            (label_width, label_height)
        } else {
            let measured = self.measurer.measure(&TextMeasureRequest {
                text: self.label.clone(),
                attribute: text::TextAttribute {
                    style: text_style,
                    flow_style: TextFlowStyle::default(),
                },
            });
            let (label_width, label_height) = measured.map_or_else(
                |_| DEFAULT_BUTTON_SIZE,
                |fragments| {
                    let width: f32 = fragments.iter().map(|f| f.width).sum();
                    let height = fragments.iter().map(|f| f.height).fold(0.0, f32::max);
                    (width, height)
                },
            );

            if let Ok(mut cache) = self.measured_cache.lock() {
                *cache = Some((label_width, label_height));
            }

            (label_width, label_height)
        };

        // Intrinsic size is the pure label extent.  CSS padding/border is
        // added by the bridge, so it must not be baked in here (otherwise a
        // styled button would pad its content twice).
        ContentSize {
            width: label_width,
            height: label_height,
        }
    }

    fn on_pointer_event(&self, event: PointerEvent) -> bool {
        match event {
            PointerEvent::Move { .. } => {
                let was_hovered = self.hovered.swap(true, Ordering::Relaxed);
                if !was_hovered {
                    self.dirty.store(true, Ordering::Relaxed);
                }
                true
            }
            PointerEvent::Down { .. } => {
                self.hovered.store(true, Ordering::Relaxed);
                self.pressed.store(true, Ordering::Relaxed);
                self.dirty.store(true, Ordering::Relaxed);
                true
            }
            PointerEvent::Up { .. } => {
                let clicked = self.pressed.swap(false, Ordering::Relaxed);
                self.hovered.store(false, Ordering::Relaxed);
                self.dirty.store(true, Ordering::Relaxed);
                clicked
            }
            PointerEvent::Leave => {
                let was_hovered = self.hovered.swap(false, Ordering::Relaxed);
                let was_pressed = self.pressed.swap(false, Ordering::Relaxed);
                if was_hovered || was_pressed {
                    self.dirty.store(true, Ordering::Relaxed);
                }
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

    fn needs_repaint(&self) -> bool {
        self.dirty.swap(false, Ordering::Relaxed)
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
    use crate::bridge::text::FallbackTextMeasurer;

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
        // Intrinsic size is the pure label extent; CSS padding is applied by
        // the bridge on top of it.
        assert!(size.width > 0.0);
        assert!(size.height > 0.0);
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
        use crate::layouter::types::TextFlowStyle;
        let button = component();
        let text_style = TextStyle::default();
        let flow_style = TextFlowStyle::default();

        let mut cmds = Vec::new();
        button.draw_sized(
            &mut cmds,
            &text_style,
            &flow_style,
            &Style::default(),
            ContentSize {
                width: 80.0,
                height: 30.0,
            },
        );
        let normal = match &cmds[0] {
            DrawCommand::Fill {
                paint:
                    Paint {
                        brush: Brush::Solid(c),
                        ..
                    },
                ..
            } => *c,
            other => panic!("expected Fill, got {other:?}"),
        };

        button.on_pointer_event(PointerEvent::Down { x: 0.0, y: 0.0 });
        let mut cmds = Vec::new();
        button.draw_sized(
            &mut cmds,
            &text_style,
            &flow_style,
            &Style::default(),
            ContentSize {
                width: 80.0,
                height: 30.0,
            },
        );
        let pressed = match &cmds[0] {
            DrawCommand::Fill {
                paint:
                    Paint {
                        brush: Brush::Solid(c),
                        ..
                    },
                ..
            } => *c,
            other => panic!("expected Fill, got {other:?}"),
        };

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
