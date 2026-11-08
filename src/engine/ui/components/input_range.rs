//! Range slider component for `input[type="range"]`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ui_layout::Style;

use crate::engine::layouter::types::{Color, TextFlowStyle, TextStyle};
use crate::engine::renderer_model::{Brush, DrawCommand, FillRule, Paint, ellipse_path, rect_path};
use crate::engine::ui::custom_node::{ContentSize, CustomNode, PointerEvent};

pub type OnRangeChange = dyn Fn(&str) + Send + Sync;

const DEFAULT_WIDTH: f32 = 160.0;
const DEFAULT_HEIGHT: f32 = 16.0;
const TRACK_HEIGHT: f32 = 4.0;

pub struct InputRangeComponent {
    value: Mutex<f64>,
    min: f64,
    max: f64,
    step: f64,
    disabled: bool,
    dragging: AtomicBool,
    hovered: AtomicBool,
    dirty: AtomicBool,
    on_change: Option<Arc<OnRangeChange>>,
}

impl std::fmt::Debug for InputRangeComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputRangeComponent")
            .field("value", &*self.value.lock().unwrap())
            .field("min", &self.min)
            .field("max", &self.max)
            .field("step", &self.step)
            .field("disabled", &self.disabled)
            .finish_non_exhaustive()
    }
}

impl InputRangeComponent {
    pub fn new(
        value: Option<f64>,
        min: Option<f64>,
        max: Option<f64>,
        step: Option<f64>,
        disabled: bool,
    ) -> Self {
        let mut min = min.unwrap_or(0.0);
        let mut max = max.unwrap_or(100.0);
        if !min.is_finite() {
            min = 0.0;
        }
        if !max.is_finite() || max < min {
            max = if min < 100.0 { 100.0 } else { min };
        }
        let step = step.filter(|s| s.is_finite() && *s > 0.0).unwrap_or(1.0);
        let default_value = (min + max) * 0.5;
        let value = value.filter(|v| v.is_finite()).unwrap_or(default_value);
        Self {
            value: Mutex::new(Self::sanitize(min, max, step, value)),
            min,
            max,
            step,
            disabled,
            dragging: AtomicBool::new(false),
            hovered: AtomicBool::new(false),
            dirty: AtomicBool::new(true),
            on_change: None,
        }
    }

    pub fn with_on_change(
        value: Option<f64>,
        min: Option<f64>,
        max: Option<f64>,
        step: Option<f64>,
        disabled: bool,
        on_change: Arc<OnRangeChange>,
    ) -> Self {
        let mut this = Self::new(value, min, max, step, disabled);
        this.on_change = Some(on_change);
        this
    }

    fn sanitize(min: f64, max: f64, step: f64, value: f64) -> f64 {
        let clamped = value.clamp(min, max);
        if step <= 0.0 || !step.is_finite() {
            return clamped;
        }
        let steps = ((clamped - min) / step).round();
        let snapped = min + steps * step;
        snapped.clamp(min, max)
    }

    fn format_value(value: f64) -> String {
        if (value.fract()).abs() < 1e-9 {
            format!("{:.0}", value)
        } else {
            let mut s = format!("{value:.10}");
            while s.contains('.') && s.ends_with('0') {
                s.pop();
            }
            if s.ends_with('.') {
                s.pop();
            }
            s
        }
    }

    fn set_from_x(&self, x: f32, width: f32) -> bool {
        let thumb_r = (DEFAULT_HEIGHT * 0.38).max(5.0);
        let left = thumb_r;
        let right = (width - thumb_r).max(left);
        let span = (right - left).max(f32::EPSILON);
        let t = ((x - left) / span).clamp(0.0, 1.0) as f64;
        let raw = self.min + (self.max - self.min) * t;
        let next = Self::sanitize(self.min, self.max, self.step, raw);
        let mut value = self.value.lock().unwrap();
        if (*value - next).abs() < 1e-9 {
            return false;
        }
        *value = next;
        drop(value);
        self.dirty.store(true, Ordering::Relaxed);
        if let Some(cb) = &self.on_change {
            cb(&Self::format_value(next));
        }
        true
    }
}

impl CustomNode for InputRangeComponent {
    fn draw_sized(
        &self,
        cmd_buf: &mut Vec<DrawCommand>,
        _text_style: &TextStyle,
        _text_flow_style: &TextFlowStyle,
        _style: &Style,
        size: ContentSize,
    ) {
        let value = *self.value.lock().unwrap();
        let span = (self.max - self.min).max(f64::EPSILON);
        let t = ((value - self.min) / span).clamp(0.0, 1.0) as f32;
        let thumb_r = (size.height * 0.38).max(5.0);
        let left = thumb_r;
        let right = (size.width - thumb_r).max(left);
        let cx = left + (right - left) * t;
        let cy = size.height * 0.5;
        let track_y = cy - TRACK_HEIGHT * 0.5;
        let track_w = (size.width - thumb_r * 2.0).max(0.0);

        let track = if self.disabled {
            Color(210, 210, 210, 255)
        } else {
            Color(180, 180, 180, 255)
        };
        let fill = if self.disabled {
            Color(160, 160, 160, 255)
        } else {
            Color(90, 140, 220, 255)
        };
        let thumb = if self.disabled {
            Color(190, 190, 190, 255)
        } else if self.dragging.load(Ordering::Relaxed) || self.hovered.load(Ordering::Relaxed) {
            Color(245, 245, 245, 255)
        } else {
            Color(255, 255, 255, 255)
        };
        let thumb_border = if self.disabled {
            Color(150, 150, 150, 255)
        } else {
            Color(118, 118, 118, 255)
        };

        if track_w > 0.0 {
            cmd_buf.push(DrawCommand::Fill {
                path: rect_path(left, track_y, track_w, TRACK_HEIGHT),
                rule: FillRule::NonZero,
                paint: Paint {
                    brush: Brush::Solid(track),
                    opacity: 1.0,
                },
            });
            let filled = ((cx - left).max(0.0)).min(track_w);
            if filled > 0.0 {
                cmd_buf.push(DrawCommand::Fill {
                    path: rect_path(left, track_y, filled, TRACK_HEIGHT),
                    rule: FillRule::NonZero,
                    paint: Paint {
                        brush: Brush::Solid(fill),
                        opacity: 1.0,
                    },
                });
            }
        }

        cmd_buf.push(DrawCommand::Fill {
            path: ellipse_path(cx, cy, thumb_r, thumb_r),
            rule: FillRule::NonZero,
            paint: Paint {
                brush: Brush::Solid(thumb_border),
                opacity: 1.0,
            },
        });
        cmd_buf.push(DrawCommand::Fill {
            path: ellipse_path(cx, cy, (thumb_r - 1.0).max(0.0), (thumb_r - 1.0).max(0.0)),
            rule: FillRule::NonZero,
            paint: Paint {
                brush: Brush::Solid(thumb),
                opacity: 1.0,
            },
        });
    }

    fn intrinsic_size(&self) -> ContentSize {
        ContentSize {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
        }
    }

    fn on_pointer_event(&self, event: PointerEvent) -> bool {
        if self.disabled {
            return false;
        }
        match event {
            PointerEvent::Move { x, .. } => {
                if !self.hovered.swap(true, Ordering::Relaxed) {
                    self.dirty.store(true, Ordering::Relaxed);
                }
                if self.dragging.load(Ordering::Relaxed) {
                    let _ = self.set_from_x(x, DEFAULT_WIDTH);
                }
                true
            }
            PointerEvent::Down { x, .. } => {
                self.dragging.store(true, Ordering::Relaxed);
                let _ = self.set_from_x(x, DEFAULT_WIDTH);
                true
            }
            PointerEvent::Up { .. } => {
                self.dragging.store(false, Ordering::Relaxed);
                self.dirty.store(true, Ordering::Relaxed);
                true
            }
            PointerEvent::Leave => {
                self.dragging.store(false, Ordering::Relaxed);
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
        Some("slider")
    }

    fn value(&self) -> Option<String> {
        Some(Self::format_value(*self.value.lock().unwrap()))
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
    fn range_reports_value_and_change() {
        use std::sync::Mutex as StdMutex;
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let cb = Arc::clone(&seen);
        let c = InputRangeComponent::with_on_change(
            Some(0.0),
            Some(0.0),
            Some(100.0),
            Some(1.0),
            false,
            Arc::new(move |v: &str| cb.lock().unwrap().push(v.to_string())),
        );
        c.on_pointer_event(PointerEvent::Down { x: 80.0, y: 8.0 });
        c.on_pointer_event(PointerEvent::Up { x: 80.0, y: 8.0 });
        assert_eq!(c.role(), Some("slider"));
        assert!(c.value().is_some());
        assert!(!seen.lock().unwrap().is_empty());
    }
}
