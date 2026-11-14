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

/// Extract the angle (in radians) from a `rotate()` function's parsed args.
/// Args shape: `vec![vec![Length(angle, unit)]]` (or a bare Number treated as radians).
fn transform_function_angle(args: &[Vec<CssValue>]) -> Option<f32> {
    let v = args.first()?.first()?;
    match *v {
        CssValue::Length(val, Unit::Deg) => Some(val.to_radians()),
        CssValue::Length(val, Unit::Rad) => Some(val),
        CssValue::Length(val, Unit::Grad) => Some(val * std::f32::consts::PI / 200.0),
        CssValue::Length(val, Unit::Turn) => Some(val * 2.0 * std::f32::consts::PI),
        CssValue::Number(n) => Some(n.to_radians()),
        _ => None,
    }
}

/// Extract `(sx, sy)` from a `scale()` function's parsed args.
/// Missing `sy` defaults to `sx` (CSS `scale(s)` === `scale(s, s)`).
fn transform_function_scale(args: &[Vec<CssValue>]) -> Option<(f32, f32)> {
    let flat: Vec<&CssValue> = args.iter().flatten().collect();
    let sx = match flat.first()? {
        CssValue::Number(n) => *n,
        _ => return None,
    };
    let sy = match flat.get(1).copied() {
        Some(CssValue::Number(n)) => *n,
        _ => sx,
    };
    Some((sx, sy))
}

/// Extract `(tx, ty)` from a translate-family function's parsed args.
///
/// - `translate(tx, ty?)` → `(tx, ty)` with `ty` defaulting to `0`
/// - `translateX(tx)` → `(tx, 0)`
/// - `translateY(ty)` → `(0, ty)`
///
/// Lengths are resolved to pixels; bare numbers are treated as pixels.
/// Percentage/viewport lengths are context-dependent and not interpolable here.
fn transform_function_translate(fn_name: &str, args: &[Vec<CssValue>]) -> Option<(f32, f32)> {
    let flat: Vec<&CssValue> = args.iter().flatten().collect();
    let to_px = |v: &CssValue| -> Option<f32> {
        match v {
            CssValue::Number(n) => Some(*n),
            CssValue::Length(val, Unit::Px) => Some(*val),
            _ => None,
        }
    };
    let first = flat.first()?;
    let tx_val = to_px(first)?;
    let ty_val = match flat.get(1).copied() {
        Some(CssValue::Number(n)) => *n,
        Some(CssValue::Length(val, Unit::Px)) => *val,
        _ => 0.0,
    };
    match fn_name {
        "translatex" => Some((tx_val, 0.0)),
        "translatey" => Some((0.0, tx_val)),
        "translate" => Some((tx_val, ty_val)),
        _ => None,
    }
}

/// Whether a `CssValue` could possibly resolve to a color. Used to avoid
/// calling `resolve_css_color` (and logging errors) on non-color values such
/// as transform functions and lists.
fn could_be_css_color(value: &CssValue) -> bool {
    match value {
        CssValue::Color(_) => true,
        CssValue::Keyword(_) => true,
        CssValue::Function(name, _) => matches!(
            name.to_ascii_lowercase().as_str(),
            "rgb"
                | "rgba"
                | "hsl"
                | "hsla"
                | "lab"
                | "lch"
                | "oklab"
                | "oklch"
                | "color-mix"
                | "light-dark"
                | "hwb"
                | "color"
        ),
        _ => false,
    }
}

/// Build an identity transform matching `value`'s transform functions, so
/// `none` ↔ `transform` transitions can interpolate instead of snapping.
fn transform_identity_like(value: &CssValue) -> Option<CssValue> {
    match value {
        CssValue::Function(name, _) => {
            let identity = match name.to_ascii_lowercase().as_str() {
                "translate" | "translatex" | "translatey" => CssValue::Function(
                    "translate".into(),
                    vec![vec![
                        CssValue::Length(0.0, Unit::Px),
                        CssValue::Length(0.0, Unit::Px),
                    ]],
                ),
                "scale" => CssValue::Function("scale".into(), vec![vec![CssValue::Number(1.0)]]),
                "rotate" => CssValue::Function(
                    "rotate".into(),
                    vec![vec![CssValue::Length(0.0, Unit::Deg)]],
                ),
                _ => return None,
            };
            Some(identity)
        }
        CssValue::List(items) => {
            let identities = items
                .iter()
                .map(transform_identity_like)
                .collect::<Option<Vec<_>>>()?;
            Some(CssValue::List(identities))
        }
        _ => None,
    }
}

/// Interpolate between two CSS values for supported animatable properties.
/// Returns `None` when interpolation is not supported (caller should snap).
pub fn interpolate_value(
    from: &CssValue,
    to: &CssValue,
    t: f32,
    color_scheme: ColorScheme,
) -> Option<CssValue> {
    let t = t.clamp(0.0, 1.0);
    if matches!(from, CssValue::Keyword(value) if value.eq_ignore_ascii_case("none"))
        && let Some(identity) = transform_identity_like(to)
    {
        return interpolate_value(&identity, to, t, color_scheme);
    }
    if matches!(to, CssValue::Keyword(value) if value.eq_ignore_ascii_case("none"))
        && let Some(identity) = transform_identity_like(from)
    {
        return interpolate_value(from, &identity, t, color_scheme);
    }
    match (from, to) {
        (CssValue::Number(a), CssValue::Number(b)) => Some(CssValue::Number(a + (b - a) * t)),
        (CssValue::Length(a, ua), CssValue::Length(b, ub)) if ua == ub => {
            Some(CssValue::Length(a + (b - a) * t, *ua))
        }
        (CssValue::Length(_, Unit::Px), CssValue::Number(_))
        | (CssValue::Number(_), CssValue::Length(_, Unit::Px)) => {
            let av = match from {
                CssValue::Length(v, _) => *v,
                CssValue::Number(v) => *v,
                _ => 0.0,
            };
            let bv = match to {
                CssValue::Length(v, _) => *v,
                CssValue::Number(v) => *v,
                _ => 0.0,
            };
            Some(CssValue::Length(av + (bv - av) * t, Unit::Px))
        }
        // CSS transform functions: interpolate matching transform functions.
        // Translate-family functions (translate/translateX/translateY) are
        // interchangeable and all resolve to a (tx, ty) pair.
        (CssValue::Function(from_name, from_args), CssValue::Function(to_name, to_args)) => {
            let f = from_name.to_ascii_lowercase();
            let g = to_name.to_ascii_lowercase();
            let same_group = match f.as_str() {
                "rotate" => g == "rotate",
                "scale" => g == "scale",
                "translate" | "translatex" | "translatey" => {
                    matches!(g.as_str(), "translate" | "translatex" | "translatey")
                }
                _ => false,
            };
            if !same_group {
                return None;
            }
            match f.as_str() {
                "rotate" => {
                    let a0 = transform_function_angle(from_args)?;
                    let a1 = transform_function_angle(to_args)?;
                    let interp = a0 + (a1 - a0) * t;
                    Some(CssValue::Function(
                        "rotate".into(),
                        vec![vec![CssValue::Length(interp.to_degrees(), Unit::Deg)]],
                    ))
                }
                "scale" => {
                    let (sx0, sy0) = transform_function_scale(from_args)?;
                    let (sx1, sy1) = transform_function_scale(to_args)?;
                    let sx = sx0 + (sx1 - sx0) * t;
                    let sy = sy0 + (sy1 - sy0) * t;
                    if (sx - sy).abs() < f32::EPSILON {
                        Some(CssValue::Function(
                            "scale".into(),
                            vec![vec![CssValue::Number(sx)]],
                        ))
                    } else {
                        Some(CssValue::Function(
                            "scale".into(),
                            vec![vec![CssValue::Number(sx)], vec![CssValue::Number(sy)]],
                        ))
                    }
                }
                "translate" | "translatex" | "translatey" => {
                    let (tx0, ty0) = transform_function_translate(&f, from_args)?;
                    let (tx1, ty1) = transform_function_translate(&g, to_args)?;
                    let tx = tx0 + (tx1 - tx0) * t;
                    let ty = ty0 + (ty1 - ty0) * t;
                    Some(CssValue::Function(
                        "translate".into(),
                        vec![vec![
                            CssValue::Length(tx, Unit::Px),
                            CssValue::Length(ty, Unit::Px),
                        ]],
                    ))
                }
                _ => None,
            }
        }
        // Transform function lists (e.g. `translateX(0) scale(1)` → `translateX(50) scale(1.5)`):
        // interpolate element-wise when both sides have the same number of functions.
        (CssValue::List(from_list), CssValue::List(to_list)) => {
            if from_list.is_empty() || from_list.len() != to_list.len() {
                return None;
            }
            let mut result = Vec::with_capacity(from_list.len());
            for (from_fn, to_fn) in from_list.iter().zip(to_list.iter()) {
                {
                    let v = interpolate_value(from_fn, to_fn, t, color_scheme)?;
                    result.push(v)
                }
            }
            Some(CssValue::List(result))
        }
        _ => {
            // Only attempt color resolution for values that could be colors.
            // Transform values (lists, rotate/scale/translate functions) are not colors;
            // calling resolve_css_color on them produces spurious error logs.
            // The caller handles None by snapping to an endpoint.
            if !could_be_css_color(from) || !could_be_css_color(to) {
                return None;
            }
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
                Some(CssValue::Color(format!(
                    "{:02x}{:02x}{:02x}{:02x}",
                    c.0, c.1, c.2, c.3
                )))
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
        for e in [
            Easing::Ease,
            Easing::EaseIn,
            Easing::EaseOut,
            Easing::EaseInOut,
            Easing::CubicBezier(0.4, 0.0, 0.2, 1.0),
        ] {
            assert!((apply_easing(0.0, e) - 0.0).abs() < 1e-3, "{e:?}");
            assert!((apply_easing(1.0, e) - 1.0).abs() < 1e-3, "{e:?}");
        }
    }

    #[test]
    fn interpolate_numbers_and_lengths() {
        let n = interpolate_value(
            &CssValue::Number(0.0),
            &CssValue::Number(10.0),
            0.5,
            ColorScheme::Light,
        )
        .unwrap();
        assert_eq!(n, CssValue::Number(5.0));
        let l = interpolate_value(
            &CssValue::Length(0.0, Unit::Px),
            &CssValue::Length(100.0, Unit::Px),
            0.25,
            ColorScheme::Light,
        )
        .unwrap();
        assert_eq!(l, CssValue::Length(25.0, Unit::Px));
    }

    fn rotate_fn(deg: f32) -> CssValue {
        CssValue::Function(
            "rotate".into(),
            vec![vec![CssValue::Length(deg, Unit::Deg)]],
        )
    }

    fn scale_fn(sx: f32, sy: f32) -> CssValue {
        CssValue::Function(
            "scale".into(),
            vec![vec![CssValue::Number(sx)], vec![CssValue::Number(sy)]],
        )
    }

    #[test]
    fn interpolate_rotate_degrees() {
        let v =
            interpolate_value(&rotate_fn(0.0), &rotate_fn(90.0), 0.5, ColorScheme::Light).unwrap();
        match v {
            CssValue::Function(name, args) if name == "rotate" => {
                let CssValue::Length(deg, Unit::Deg) = args[0][0] else {
                    panic!("expected Length deg, got {args:?}")
                };
                assert!((deg - 45.0).abs() < 1e-4, "got {deg}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn interpolate_rotate_wraps_turns_and_degs() {
        // 0.5turn = 180deg; interpolate to 90deg => 135deg
        let from = CssValue::Function(
            "rotate".into(),
            vec![vec![CssValue::Length(0.5, Unit::Turn)]],
        );
        let to = rotate_fn(90.0);
        let v = interpolate_value(&from, &to, 0.5, ColorScheme::Light).unwrap();
        match v {
            CssValue::Function(name, args) if name == "rotate" => {
                let CssValue::Length(deg, Unit::Deg) = args[0][0] else {
                    panic!("expected Length deg, got {args:?}")
                };
                assert!((deg - 135.0).abs() < 1e-4, "got {deg}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn interpolate_scale_uniform() {
        let v = interpolate_value(
            &scale_fn(1.0, 1.0),
            &scale_fn(2.0, 2.0),
            0.25,
            ColorScheme::Light,
        )
        .unwrap();
        match v {
            CssValue::Function(name, args) if name == "scale" => {
                assert_eq!(args.len(), 1);
                let CssValue::Number(s) = args[0][0] else {
                    panic!("expected Number, got {args:?}")
                };
                assert!((s - 1.25).abs() < 1e-4, "got {s}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn interpolate_scale_non_uniform() {
        let v = interpolate_value(
            &scale_fn(1.0, 2.0),
            &scale_fn(3.0, 4.0),
            0.5,
            ColorScheme::Light,
        )
        .unwrap();
        match v {
            CssValue::Function(name, args) if name == "scale" => {
                assert_eq!(args.len(), 2);
                let CssValue::Number(sx) = args[0][0] else {
                    panic!("expected Number, got {args:?}")
                };
                let CssValue::Number(sy) = args[1][0] else {
                    panic!("expected Number, got {args:?}")
                };
                assert!((sx - 2.0).abs() < 1e-4, "got {sx}");
                assert!((sy - 3.0).abs() < 1e-4, "got {sy}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn interpolate_mismatched_transform_functions_returns_none() {
        let v = interpolate_value(
            &rotate_fn(0.0),
            &scale_fn(1.0, 1.0),
            0.5,
            ColorScheme::Light,
        );
        assert!(v.is_none());
    }

    #[test]
    fn interpolate_single_arg_scale_defaults_sy() {
        let from = CssValue::Function("scale".into(), vec![vec![CssValue::Number(1.0)]]);
        let to = CssValue::Function("scale".into(), vec![vec![CssValue::Number(3.0)]]);
        let v = interpolate_value(&from, &to, 0.5, ColorScheme::Light).unwrap();
        match v {
            CssValue::Function(name, args) if name == "scale" => {
                assert_eq!(args.len(), 1);
                let CssValue::Number(s) = args[0][0] else {
                    panic!("expected Number, got {args:?}")
                };
                assert!((s - 2.0).abs() < 1e-4, "got {s}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    fn translate_fn(tx: f32, ty: f32) -> CssValue {
        CssValue::Function(
            "translate".into(),
            vec![vec![
                CssValue::Length(tx, Unit::Px),
                CssValue::Length(ty, Unit::Px),
            ]],
        )
    }

    fn translatex_fn(tx: f32) -> CssValue {
        CssValue::Function(
            "translateX".into(),
            vec![vec![CssValue::Length(tx, Unit::Px)]],
        )
    }

    fn translatey_fn(ty: f32) -> CssValue {
        CssValue::Function(
            "translateY".into(),
            vec![vec![CssValue::Length(ty, Unit::Px)]],
        )
    }

    #[test]
    fn interpolate_translate() {
        let v = interpolate_value(
            &translate_fn(0.0, 0.0),
            &translate_fn(100.0, 200.0),
            0.25,
            ColorScheme::Light,
        )
        .unwrap();
        match v {
            CssValue::Function(name, args) if name == "translate" => {
                assert_eq!(args.len(), 1);
                let CssValue::Length(x, Unit::Px) = args[0][0] else {
                    panic!("expected Length px, got {args:?}")
                };
                let CssValue::Length(y, Unit::Px) = args[0][1] else {
                    panic!("expected Length px, got {args:?}")
                };
                assert!((x - 25.0).abs() < 1e-4, "got {x}");
                assert!((y - 50.0).abs() < 1e-4, "got {y}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn interpolate_translatex() {
        let v = interpolate_value(
            &translatex_fn(0.0),
            &translatex_fn(100.0),
            0.5,
            ColorScheme::Light,
        )
        .unwrap();
        match v {
            CssValue::Function(name, args) if name == "translate" => {
                let CssValue::Length(x, Unit::Px) = args[0][0] else {
                    panic!("expected Length px, got {args:?}")
                };
                let CssValue::Length(y, Unit::Px) = args[0][1] else {
                    panic!("expected Length px, got {args:?}")
                };
                assert!((x - 50.0).abs() < 1e-4, "got {x}");
                assert_eq!(y, 0.0);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn interpolate_translatey() {
        let v = interpolate_value(
            &translatey_fn(0.0),
            &translatey_fn(100.0),
            0.5,
            ColorScheme::Light,
        )
        .unwrap();
        match v {
            CssValue::Function(name, args) if name == "translate" => {
                let CssValue::Length(x, Unit::Px) = args[0][0] else {
                    panic!("expected Length px, got {args:?}")
                };
                let CssValue::Length(y, Unit::Px) = args[0][1] else {
                    panic!("expected Length px, got {args:?}")
                };
                assert_eq!(x, 0.0);
                assert!((y - 50.0).abs() < 1e-4, "got {y}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn interpolate_translate_family_mixed_names() {
        // translateX(0) animating to translate(100, 200) should still interpolate
        let v = interpolate_value(
            &translatex_fn(0.0),
            &translate_fn(100.0, 200.0),
            0.5,
            ColorScheme::Light,
        )
        .unwrap();
        match v {
            CssValue::Function(name, args) if name == "translate" => {
                let CssValue::Length(x, Unit::Px) = args[0][0] else {
                    panic!("expected Length px, got {args:?}")
                };
                let CssValue::Length(y, Unit::Px) = args[0][1] else {
                    panic!("expected Length px, got {args:?}")
                };
                assert!((x - 50.0).abs() < 1e-4, "got {x}");
                assert!((y - 100.0).abs() < 1e-4, "got {y}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn interpolate_translate_vs_scale_returns_none() {
        let v = interpolate_value(
            &translate_fn(0.0, 0.0),
            &CssValue::Function("scale".into(), vec![vec![CssValue::Number(2.0)]]),
            0.5,
            ColorScheme::Light,
        );
        assert!(v.is_none());
    }

    fn transform_list(functions: Vec<CssValue>) -> CssValue {
        CssValue::List(functions)
    }

    #[test]
    fn interpolate_transform_list_element_wise() {
        let from = transform_list(vec![
            translatex_fn(0.0),
            CssValue::Function("scale".into(), vec![vec![CssValue::Number(1.0)]]),
        ]);
        let to = transform_list(vec![
            translatex_fn(100.0),
            CssValue::Function("scale".into(), vec![vec![CssValue::Number(3.0)]]),
        ]);
        let v = interpolate_value(&from, &to, 0.5, ColorScheme::Light).unwrap();
        match v {
            CssValue::List(items) => {
                assert_eq!(items.len(), 2);
                match &items[0] {
                    CssValue::Function(name, args) if name == "translate" => {
                        let CssValue::Length(x, Unit::Px) = args[0][0] else {
                            panic!("expected Length px, got {args:?}")
                        };
                        assert!((x - 50.0).abs() < 1e-4, "got {x}");
                    }
                    other => panic!("expected translate function, got {other:?}"),
                }
                match &items[1] {
                    CssValue::Function(name, args) if name == "scale" => {
                        let CssValue::Number(s) = args[0][0] else {
                            panic!("expected Number, got {args:?}")
                        };
                        assert!((s - 2.0).abs() < 1e-4, "got {s}");
                    }
                    other => panic!("expected scale function, got {other:?}"),
                }
            }
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn interpolate_transform_list_mismatched_length_returns_none() {
        let from = transform_list(vec![translatex_fn(0.0)]);
        let to = transform_list(vec![translatex_fn(0.0), translatex_fn(100.0)]);
        let v = interpolate_value(&from, &to, 0.5, ColorScheme::Light);
        assert!(v.is_none());
    }

    #[test]
    fn interpolate_transform_list_non_color_values_do_not_log_error() {
        // Both are non-color Lists; must return None without calling
        // resolve_css_color (which would log an error).
        let from = CssValue::Keyword("red".into());
        let to = transform_list(vec![translatex_fn(0.0), translatex_fn(100.0)]);
        let v = interpolate_value(&from, &to, 0.5, ColorScheme::Light);
        assert!(v.is_none());
    }

    #[test]
    fn interpolate_empty_transform_list_returns_none() {
        let from = CssValue::List(vec![]);
        let to = CssValue::List(vec![]);
        let v = interpolate_value(&from, &to, 0.5, ColorScheme::Light);
        assert!(v.is_none());
    }

    #[test]
    fn steps_easing_jump_end() {
        // 4 steps, jump-end: 0, 0.25, 0.5, 0.75, 1.0
        assert!((steps_easing(4, StepPosition::JumpEnd, 0.0) - 0.0).abs() < 1e-6);
        assert!((steps_easing(4, StepPosition::JumpEnd, 0.25) - 0.25).abs() < 1e-6);
        assert!((steps_easing(4, StepPosition::JumpEnd, 0.5) - 0.5).abs() < 1e-6);
        assert!((steps_easing(4, StepPosition::JumpEnd, 0.75) - 0.75).abs() < 1e-6);
        assert!((steps_easing(4, StepPosition::JumpEnd, 1.0) - 1.0).abs() < 1e-6);
        // Between steps: stays at previous step
        assert!((steps_easing(4, StepPosition::JumpEnd, 0.1) - 0.0).abs() < 1e-6);
        assert!((steps_easing(4, StepPosition::JumpEnd, 0.3) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn steps_easing_jump_start() {
        // 4 steps, jump-start: 0.25, 0.5, 0.75, 1.0, 1.0
        assert!((steps_easing(4, StepPosition::JumpStart, 0.0) - 0.25).abs() < 1e-6);
        assert!((steps_easing(4, StepPosition::JumpStart, 0.25) - 0.5).abs() < 1e-6);
        assert!((steps_easing(4, StepPosition::JumpStart, 0.5) - 0.75).abs() < 1e-6);
        assert!((steps_easing(4, StepPosition::JumpStart, 1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn steps_easing_jump_none() {
        // 4 steps, jump-none: 0, 1/3, 2/3, 1 (n-1 jumps, no jump at either end)
        assert!((steps_easing(4, StepPosition::JumpNone, 0.0) - 0.0).abs() < 1e-6);
        assert!((steps_easing(4, StepPosition::JumpNone, 0.25) - 0.0).abs() < 1e-6);
        assert!((steps_easing(4, StepPosition::JumpNone, 0.5) - 1.0 / 3.0).abs() < 1e-6);
        assert!((steps_easing(4, StepPosition::JumpNone, 1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn steps_easing_jump_both() {
        // 4 steps, jump-both: 1/4, 2/4, 3/4, 4/4 (jump at both ends)
        assert!((steps_easing(4, StepPosition::JumpBoth, 0.0) - 0.25).abs() < 1e-6);
        assert!((steps_easing(4, StepPosition::JumpBoth, 0.25) - 0.5).abs() < 1e-6);
        assert!((steps_easing(4, StepPosition::JumpBoth, 0.5) - 0.75).abs() < 1e-6);
        assert!((steps_easing(4, StepPosition::JumpBoth, 1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn steps_easing_apply_easing_integration() {
        // apply_easing wraps steps_easing
        let easing = Easing::Steps(4, StepPosition::JumpEnd);
        assert!((apply_easing(0.0, easing) - 0.0).abs() < 1e-6);
        assert!((apply_easing(0.25, easing) - 0.25).abs() < 1e-6);
        assert!((apply_easing(0.5, easing) - 0.5).abs() < 1e-6);
        assert!((apply_easing(0.75, easing) - 0.75).abs() < 1e-6);
        assert!((apply_easing(1.0, easing) - 1.0).abs() < 1e-6);
    }
}
