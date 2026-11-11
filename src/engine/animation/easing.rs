//! Easing and interpolation helpers for CSS transitions/animations.

use crate::engine::animation::types::{Easing, StepPosition};
use crate::engine::css::values::{CssValue, Unit};
use crate::engine::layouter::resolve_css_color;
use crate::engine::layouter::types::ColorScheme;

/// Evaluate an easing function at progress `t` in `[0, 1]`.
pub fn apply_easing(t: f32, easing: Easing) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match easing {
        Easing::Linear => t,
        Easing::Ease => cubic_bezier(0.25, 0.1, 0.25, 1.0, t),
        Easing::EaseIn => cubic_bezier(0.42, 0.0, 1.0, 1.0, t),
        Easing::EaseOut => cubic_bezier(0.0, 0.0, 0.58, 1.0, t),
        Easing::EaseInOut => cubic_bezier(0.42, 0.0, 0.58, 1.0, t),
        Easing::CubicBezier(x1, y1, x2, y2) => cubic_bezier(x1, y1, x2, y2, t),
        Easing::Steps(steps, pos) => steps_easing(steps.max(1), pos, t),
    }
}

fn steps_easing(steps: u32, pos: StepPosition, t: f32) -> f32 {
    let n = steps as f32;
    if n <= 0.0 {
        return t;
    }
    let (jumps, offset): (f32, f32) = match pos {
        StepPosition::JumpEnd => (n, 0.0),
        StepPosition::JumpStart => (n, 1.0),
        StepPosition::JumpNone => ((n - 1.0).max(1.0), 0.0),
        StepPosition::JumpBoth => (n + 1.0, 1.0),
    };
    let step = ((t * jumps) + offset).floor().min(n);
    let denom = match pos {
        StepPosition::JumpNone => (n - 1.0).max(1.0),
        StepPosition::JumpEnd => n,
        StepPosition::JumpStart => n,
        StepPosition::JumpBoth => n,
    };
    (step / denom).clamp(0.0, 1.0)
}

fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    // Newton-Raphson to find x(t)=input
    let mut u = t;
    for _ in 0..8 {
        let x = sample_curve(x1, x2, u) - t;
        if x.abs() < 1e-5 {
            break;
        }
        let d = sample_derivative(x1, x2, u);
        if d.abs() < 1e-6 {
            break;
        }
        u = (u - x / d).clamp(0.0, 1.0);
    }
    sample_curve(y1, y2, u).clamp(0.0, 1.0)
}

fn sample_curve(a: f32, b: f32, t: f32) -> f32 {
    // (1-t)^3*0 + 3(1-t)^2*t*a + 3(1-t)*t^2*b + t^3*1
    let mt = 1.0 - t;
    3.0 * mt * mt * t * a + 3.0 * mt * t * t * b + t * t * t
}

fn sample_derivative(a: f32, b: f32, t: f32) -> f32 {
    let mt = 1.0 - t;
    3.0 * mt * mt * a + 6.0 * mt * t * (b - a) + 3.0 * t * t * (1.0 - b)
}

/// Interpolate between two CSS values for supported animatable properties.
/// Returns `None` when interpolation is not supported (caller should snap).
pub fn interpolate_value(from: &CssValue, to: &CssValue, t: f32, color_scheme: ColorScheme) -> Option<CssValue> {
    let t = t.clamp(0.0, 1.0);
    match (from, to) {
        (CssValue::Number(a), CssValue::Number(b)) => Some(CssValue::Number(a + (b - a) * t)),
        (CssValue::Length(a, ua), CssValue::Length(b, ub)) if ua == ub => {
            Some(CssValue::Length(a + (b - a) * t, *ua))
        }
        (CssValue::Length(_, Unit::Px), CssValue::Number(_)) | (CssValue::Number(_), CssValue::Length(_, Unit::Px)) => {
            let av = match from { CssValue::Length(v, _) => *v, CssValue::Number(v) => *v, _ => 0.0 };
            let bv = match to { CssValue::Length(v, _) => *v, CssValue::Number(v) => *v, _ => 0.0 };
            Some(CssValue::Length(av + (bv - av) * t, Unit::Px))
        }
        _ => {
            // Try color interpolation
            if let (Some(ca), Some(cb)) = (
                resolve_css_color("animation", from, color_scheme),
                resolve_css_color("animation", to, color_scheme),
            ) {
                let a = ca.to_linear_f32_array();
                let b = cb.to_linear_f32_array();
                let mixed = [
                    a[0] + (b[0] - a[0]) * t,
                    a[1] + (b[1] - a[1]) * t,
                    a[2] + (b[2] - a[2]) * t,
                    a[3] + (b[3] - a[3]) * t,
                ];
                let c = crate::engine::layouter::types::Color::from_linear_f32_array(mixed);
                // resolve_css_color accepts CssValue::Color as hex (with or without alpha).
                Some(CssValue::Color(format!("{:02x}{:02x}{:02x}{:02x}", c.0, c.1, c.2, c.3)))
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_linear_identity() {
        assert!((apply_easing(0.0, Easing::Linear) - 0.0).abs() < 1e-6);
        assert!((apply_easing(0.5, Easing::Linear) - 0.5).abs() < 1e-6);
        assert!((apply_easing(1.0, Easing::Linear) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn easing_ease_endpoints() {
        for e in [Easing::Ease, Easing::EaseIn, Easing::EaseOut, Easing::EaseInOut, Easing::CubicBezier(0.4, 0.0, 0.2, 1.0)] {
            assert!((apply_easing(0.0, e) - 0.0).abs() < 1e-3, "{e:?}");
            assert!((apply_easing(1.0, e) - 1.0).abs() < 1e-3, "{e:?}");
        }
    }

    #[test]
    fn interpolate_numbers_and_lengths() {
        let n = interpolate_value(&CssValue::Number(0.0), &CssValue::Number(10.0), 0.5, ColorScheme::Light).unwrap();
        assert_eq!(n, CssValue::Number(5.0));
        let l = interpolate_value(&CssValue::Length(0.0, Unit::Px), &CssValue::Length(100.0, Unit::Px), 0.25, ColorScheme::Light).unwrap();
        assert_eq!(l, CssValue::Length(25.0, Unit::Px));
    }
}
