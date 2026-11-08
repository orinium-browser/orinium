//! Checkbox component for `input[type="checkbox"]`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ui_layout::Style;

use crate::engine::layouter::types::{Color, FontWeight, TextFlowStyle, TextStyle};
use crate::engine::renderer_model::{Brush, DrawCommand, FillRule, Paint, rect_path};
use crate::engine::ui::custom_node::{ContentSize, CustomNode, PointerEvent};

pub type OnCheckedChange = dyn Fn(bool) + Send + Sync;

const DEFAULT_SIZE: f32 = 13.0;

pub struct InputCheckboxComponent {
    checked: AtomicBool,
    disabled: bool,
    hovered: AtomicBool,
    dirty: AtomicBool,
    on_change: Option<Arc<OnCheckedChange>>,
}

impl std::fmt::Debug for InputCheckboxComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputCheckboxComponent")
            .field("checked", &self.checked.load(Ordering::Relaxed))
            .field("disabled", &self.disabled)
            .finish_non_exhaustive()
    }
}

impl InputCheckboxComponent {
    pub fn new(checked: bool, disabled: bool) -> Self {
        Self {
            checked: AtomicBool::new(checked),
            disabled,
            hovered: AtomicBool::new(false),
            dirty: AtomicBool::new(true),
            on_change: None,
        }
    }

    pub fn with_on_change(checked: bool, disabled: bool, on_change: Arc<OnCheckedChange>) -> Self {
        let mut this = Self::new(checked, disabled);
        this.on_change = Some(on_change);
        this
    }

    fn toggle(&self) {
        if self.disabled {
            return;
        }
        let checked = !self.checked.load(Ordering::Relaxed);
        self.checked.store(checked, Ordering::Relaxed);
        self.dirty.store(true, Ordering::Relaxed);
        if let Some(cb) = &self.on_change {
            cb(checked);
        }
    }
}

impl CustomNode for InputCheckboxComponent {
    fn draw_sized(
        &self,
        cmd_buf: &mut Vec<DrawCommand>,
        _text_style: &TextStyle,
        text_flow_style: &TextFlowStyle,
        _style: &Style,
        size: ContentSize,
    ) {
        let checked = self.checked.load(Ordering::Relaxed);
        let border = if self.disabled {
            Color(170, 170, 170, 255)
        } else if self.hovered.load(Ordering::Relaxed) {
            Color(70, 70, 70, 255)
        } else {
            Color(118, 118, 118, 255)
        };
        let bg = if self.disabled {
            Color(235, 235, 235, 255)
        } else {
            Color(255, 255, 255, 255)
        };

        cmd_buf.push(DrawCommand::Fill {
            path: rect_path(0.0, 0.0, size.width, size.height),
            rule: FillRule::NonZero,
            paint: Paint {
                brush: Brush::Solid(bg),
                opacity: 1.0,
            },
        });
        for path in [
            rect_path(0.0, 0.0, size.width, 1.0),
            rect_path(0.0, size.height - 1.0, size.width, 1.0),
            rect_path(0.0, 0.0, 1.0, size.height),
            rect_path(size.width - 1.0, 0.0, 1.0, size.height),
        ] {
            cmd_buf.push(DrawCommand::Fill {
                path,
                rule: FillRule::NonZero,
                paint: Paint {
                    brush: Brush::Solid(border),
                    opacity: 1.0,
                },
            });
        }

        if checked {
            let mut style = TextStyle::default();
            style.font_weight = FontWeight(700);
            style.color = if self.disabled {
                Color(120, 120, 120, 255)
            } else {
                Color(30, 30, 30, 255)
            };
            let mut flow = *text_flow_style;
            flow.font_size = (size.height * 0.85).max(10.0);
            cmd_buf.push(DrawCommand::DrawText {
                x: (size.width * 0.12).max(0.0),
                y: (size.height * 0.02).max(0.0),
                text: "✓".into(),
                style,
                flow_style: flow,
            });
        }
    }

    fn intrinsic_size(&self) -> ContentSize {
        ContentSize {
            width: DEFAULT_SIZE,
            height: DEFAULT_SIZE,
        }
    }

    fn on_pointer_event(&self, event: PointerEvent) -> bool {
        if self.disabled {
            return false;
        }
        match event {
            PointerEvent::Move { .. } => {
                if !self.hovered.swap(true, Ordering::Relaxed) {
                    self.dirty.store(true, Ordering::Relaxed);
                }
                true
            }
            PointerEvent::Down { .. } => true,
            PointerEvent::Up { .. } => {
                self.toggle();
                true
            }
            PointerEvent::Leave => {
                if self.hovered.swap(false, Ordering::Relaxed) {
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

    fn set_checked(&self, checked: bool) {
        if self.checked.swap(checked, Ordering::Relaxed) != checked {
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
        Some("checkbox")
    }

    fn is_disabled(&self) -> bool {
        self.disabled
    }

    fn accepts_text_input(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_on_pointer_up_and_reports_change() {
        use std::sync::Mutex;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let cb_seen = Arc::clone(&seen);
        let c = InputCheckboxComponent::with_on_change(
            false,
            false,
            Arc::new(move |v| cb_seen.lock().unwrap().push(v)),
        );
        c.on_pointer_event(PointerEvent::Down { x: 1.0, y: 1.0 });
        c.on_pointer_event(PointerEvent::Up { x: 1.0, y: 1.0 });
        assert_eq!(c.role(), Some("checkbox"));
        assert_eq!(&*seen.lock().unwrap(), &[true]);
    }
}
