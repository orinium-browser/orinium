//! Radio component for `input[type="radio"]`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ui_layout::Style;

use crate::engine::layouter::types::{Color, TextFlowStyle, TextStyle};
use crate::engine::renderer_model::{Brush, DrawCommand, FillRule, Paint, ellipse_path};
use crate::engine::ui::custom_node::{ContentSize, CustomNode, PointerEvent};

pub type OnRadioSelect = dyn Fn() + Send + Sync;

const DEFAULT_SIZE: f32 = 13.0;

pub struct InputRadioComponent {
    checked: AtomicBool,
    disabled: bool,
    hovered: AtomicBool,
    dirty: AtomicBool,
    on_select: Option<Arc<OnRadioSelect>>,
}

impl std::fmt::Debug for InputRadioComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputRadioComponent")
            .field("checked", &self.checked.load(Ordering::Relaxed))
            .field("disabled", &self.disabled)
            .finish_non_exhaustive()
    }
}

impl InputRadioComponent {
    pub fn new(checked: bool, disabled: bool) -> Self {
        Self {
            checked: AtomicBool::new(checked),
            disabled,
            hovered: AtomicBool::new(false),
            dirty: AtomicBool::new(true),
            on_select: None,
        }
    }

    pub fn with_on_select(checked: bool, disabled: bool, on_select: Arc<OnRadioSelect>) -> Self {
        let mut this = Self::new(checked, disabled);
        this.on_select = Some(on_select);
        this
    }

    fn select(&self) {
        if self.disabled {
            return;
        }
        self.checked.store(true, Ordering::Relaxed);
        self.dirty.store(true, Ordering::Relaxed);
        if let Some(cb) = &self.on_select {
            cb();
        }
    }
}

impl CustomNode for InputRadioComponent {
    fn draw_sized(
        &self,
        cmd_buf: &mut Vec<DrawCommand>,
        _text_style: &TextStyle,
        _text_flow_style: &TextFlowStyle,
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
        let cx = size.width * 0.5;
        let cy = size.height * 0.5;
        let r = (size.width.min(size.height) * 0.5).max(0.0);

        cmd_buf.push(DrawCommand::Fill {
            path: ellipse_path(cx, cy, r, r),
            rule: FillRule::NonZero,
            paint: Paint {
                brush: Brush::Solid(bg),
                opacity: 1.0,
            },
        });
        // Outer ring approximation via a thin filled border path is not directly
        // supported by path stroking in this pipeline; draw inner darkening for
        // state and rely on UA background. For checked state draw an inner dot.
        let ring = ellipse_path(cx, cy, (r - 0.5).max(0.0), (r - 0.5).max(0.0));
        cmd_buf.push(DrawCommand::Fill {
            path: ring,
            rule: FillRule::NonZero,
            paint: Paint {
                brush: Brush::Solid(border),
                opacity: 0.18,
            },
        });

        if checked {
            let dot_r = (r * 0.32).max(1.5);
            let dot = if self.disabled {
                Color(120, 120, 120, 255)
            } else {
                Color(30, 30, 30, 255)
            };
            cmd_buf.push(DrawCommand::Fill {
                path: ellipse_path(cx, cy, dot_r, dot_r),
                rule: FillRule::NonZero,
                paint: Paint {
                    brush: Brush::Solid(dot),
                    opacity: 1.0,
                },
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
                self.select();
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

    fn is_hovered(&self) -> bool {
        self.hovered.load(Ordering::Relaxed)
    }

    fn needs_repaint(&self) -> bool {
        self.dirty.swap(false, Ordering::Relaxed)
    }

    fn role(&self) -> Option<&'static str> {
        Some("radio")
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
    fn selects_on_pointer_up_and_reports() {
        use std::sync::Mutex;
        let seen = Arc::new(Mutex::new(0usize));
        let cb_seen = Arc::clone(&seen);
        let c = InputRadioComponent::with_on_select(
            false,
            false,
            Arc::new(move || *cb_seen.lock().unwrap() += 1),
        );
        c.on_pointer_event(PointerEvent::Down { x: 1.0, y: 1.0 });
        c.on_pointer_event(PointerEvent::Up { x: 1.0, y: 1.0 });
        assert_eq!(c.role(), Some("radio"));
        assert_eq!(*seen.lock().unwrap(), 1);
    }
}
