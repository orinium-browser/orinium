use crate::engine::css::values::{CssValue, Unit};
use crate::engine::layouter::types::{Color, ColorScheme};

fn mix_colors(colors: &[Color], weights: &[f32], space: &str) -> Color {
    let n = colors.len().min(weights.len());
    let mut acc = colors[0];
    let mut acc_w = weights[0];
    for i in 1..n {
        if weights[i] <= 0.0 {
            continue;
        }
        acc = match space {
            "lch" => mix_two_lch(acc, colors[i], acc_w, weights[i]),
            _ => mix_two_srgb(acc, colors[i], acc_w, weights[i]),
        };
        acc_w += weights[i];
    }
    acc
}

/// Interpolate between two colors in sRGB with premultiplied alpha.
fn mix_two_srgb(a: Color, b: Color, wa: f32, wb: f32) -> Color {
    let total = wa + wb;
    if total <= 0.0 {
        return Color(0, 0, 0, 0);
    }
    let f = wb / total;
    let al = a.to_linear_f32_array();
    let bl = b.to_linear_f32_array();
    let mut c = [0.0f32; 4];
    for i in 0..4 {
        c[i] = al[i] * al[3] * (1.0 - f) + bl[i] * bl[3] * f;
    }
    let alpha = al[3] * (1.0 - f) + bl[3] * f;
    if alpha > 0.0 {
        for c_i in &mut c[..3] {
            *c_i /= alpha;
        }
    }
    c[3] = alpha;
    Color::from_linear_f32_array(c)
}

/// Interpolate between two colors in LCH with premultiplied alpha.
fn mix_two_lch(a: Color, b: Color, wa: f32, wb: f32) -> Color {
    let total = wa + wb;
    if total <= 0.0 {
        return Color(0, 0, 0, 0);
    }
    let f = wb / total;
    let a_alpha = a.3 as f32 / 255.0;
    let b_alpha = b.3 as f32 / 255.0;
    let (al, ac, ah) = rgb_to_lch(a);
    let (bl, bc, bh) = rgb_to_lch(b);
    let lm = al * a_alpha * (1.0 - f) + bl * b_alpha * f;
    let cm = ac * a_alpha * (1.0 - f) + bc * b_alpha * f;
    let hm = lerp_hue(ah, bh, f);
    let alpha = a_alpha * (1.0 - f) + b_alpha * f;
    let (l, c) = if alpha > 0.0 {
        (lm / alpha, cm / alpha)
    } else {
        (lm, cm)
    };
    lch_to_color(l, c, hm, alpha)
}

/// Interpolate a hue angle along the shortest arc.
fn lerp_hue(a: f32, b: f32, t: f32) -> f32 {
    let mut d = b - a;
    if d > 180.0 {
        d -= 360.0;
    } else if d < -180.0 {
        d += 360.0;
    }
    (a + d * t).rem_euclid(360.0)
}

const D65_WHITE: (f32, f32, f32) = (0.95047, 1.0, 1.08883);

fn lab_f(t: f32) -> f32 {
    const EPS: f32 = 6.0 / 29.0;
    if t > EPS * EPS * EPS {
        t.cbrt()
    } else {
        t / (3.0 * EPS * EPS) + 4.0 / 29.0
    }
}

fn lab_f_inv(t: f32) -> f32 {
    const EPS: f32 = 6.0 / 29.0;
    if t > EPS {
        t * t * t
    } else {
        3.0 * EPS * EPS * (t - 4.0 / 29.0)
    }
}

/// Convert sRGB (0..1 channels) to CIE XYZ (D65).
fn srgb_to_xyz(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let r = Color::srgb_to_linear(r);
    let g = Color::srgb_to_linear(g);
    let b = Color::srgb_to_linear(b);
    let x = 0.4124564 * r + 0.3575761 * g + 0.1804375 * b;
    let y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b;
    let z = 0.0193339 * r + 0.119_192 * g + 0.9503041 * b;
    (x, y, z)
}

/// Convert CIE XYZ (D65) to sRGB (0..1 channels).
fn xyz_to_srgb(x: f32, y: f32, z: f32) -> (f32, f32, f32) {
    let r = 3.2404542 * x - 1.5371385 * y - 0.4985314 * z;
    let g = -0.969_266 * x + 1.8760108 * y + 0.0415560 * z;
    let b = 0.0556434 * x - 0.2040259 * y + 1.0572252 * z;
    (
        Color::linear_to_srgb(r),
        Color::linear_to_srgb(g),
        Color::linear_to_srgb(b),
    )
}

fn xyz_to_lab(x: f32, y: f32, z: f32) -> (f32, f32, f32) {
    let (wx, wy, wz) = D65_WHITE;
    let fx = lab_f(x / wx);
    let fy = lab_f(y / wy);
    let fz = lab_f(z / wz);
    let l = 116.0 * fy - 16.0;
    let a = 500.0 * (fx - fy);
    let b = 200.0 * (fy - fz);
    (l, a, b)
}

fn lab_to_xyz(l: f32, a: f32, b: f32) -> (f32, f32, f32) {
    let (wx, wy, wz) = D65_WHITE;
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    (wx * lab_f_inv(fx), wy * lab_f_inv(fy), wz * lab_f_inv(fz))
}

/// Convert an sRGB color to LCH (L 0..100, C >= 0, H 0..360).
fn rgb_to_lch(c: Color) -> (f32, f32, f32) {
    let (x, y, z) = srgb_to_xyz(c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0);
    let (l, a, b) = xyz_to_lab(x, y, z);
    let chroma = (a * a + b * b).sqrt();
    let hue = b.atan2(a).to_degrees().rem_euclid(360.0);
    (l, chroma, hue)
}

/// Convert LCH (L 0..100, C >= 0, H 0..360) back to an sRGB color.
fn lch_to_color(l: f32, chroma: f32, hue: f32, alpha: f32) -> Color {
    let hr = hue.to_radians();
    let a = chroma * hr.cos();
    let b = chroma * hr.sin();
    let (x, y, z) = lab_to_xyz(l, a, b);
    let (r, g, b) = xyz_to_srgb(x, y, z);
    Color(
        (r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (b.clamp(0.0, 1.0) * 255.0).round() as u8,
        (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

/// Parse `color-mix(in <space>, <color> [<percentage>], ...)`.
fn parse_color_mix(args: &[CssValue], name: &str, color_scheme: ColorScheme) -> Option<Color> {
    // args: [Keyword("in"), Keyword("<space>"), <color>..., ...]
    if !matches!(args.first(), Some(CssValue::Keyword(k)) if k.eq_ignore_ascii_case("in")) {
        return None;
    }
    let space = match args.get(1) {
        Some(CssValue::Keyword(k)) => k.to_ascii_lowercase(),
        _ => return None,
    };

    let mut colors: Vec<Color> = Vec::new();
    let mut weights: Vec<Option<f32>> = Vec::new();
    let mut i = 2;
    while i < args.len() {
        let color = resolve_css_color(name, &args[i], color_scheme)?;
        i += 1;
        let mut weight = None;
        if i < args.len() {
            match &args[i] {
                CssValue::Number(n) => {
                    weight = Some(*n);
                    i += 1;
                }
                CssValue::Length(p, Unit::Percent) => {
                    weight = Some(*p);
                    i += 1;
                }
                _ => {}
            }
        }
        colors.push(color);
        weights.push(weight);
    }

    if colors.len() < 2 {
        return None;
    }

    // Resolve missing weights to the remaining percentage.
    let specified_sum: f32 = weights.iter().flatten().sum();
    let missing = weights.iter().filter(|w| w.is_none()).count();
    let remainder = (100.0 - specified_sum).max(0.0);
    let mut resolved: Vec<f32> = Vec::with_capacity(colors.len());
    for weight in &weights {
        match weight {
            Some(v) => resolved.push(*v),
            None if missing > 0 => resolved.push(remainder / missing as f32),
            None => resolved.push(0.0),
        }
    }
    let total: f32 = resolved.iter().sum();
    let normalized: Vec<f32> = if total > 0.0 {
        resolved.iter().map(|v| v / total).collect()
    } else {
        vec![1.0 / colors.len() as f32; colors.len()]
    };

    Some(mix_colors(&colors, &normalized, &space))
}

/// Resolve a single CSS component value to a plain `f32`.
///
/// Handles `calc()`, `min()`, `max()`, `clamp()`, plain numbers,
/// and percentage lengths. Returns `None` for values that cannot
/// be reduced to a number (e.g. keyword colors, px lengths).
fn resolve_channel(value: &CssValue) -> Option<f32> {
    match value {
        CssValue::Number(n) => Some(*n),
        CssValue::Length(p, Unit::Percent) => Some(*p),
        CssValue::Function(fn_name, args) => resolve_channel_function(fn_name, args),
        CssValue::List(items) => {
            // A list acts like an inline calc expression: [a, +, b]
            // or a calc result with a unit suffix: [calc(100 / 2), "%"]
            if items.len() == 1 {
                return resolve_channel(&items[0]);
            }
            // If the last element is a bare "%" keyword, the preceding
            // elements form the numeric expression (the result is a percent).
            if let Some(CssValue::Keyword(pct)) = items.last()
                && pct == "%"
            {
                let expr: Vec<&CssValue> = items[..items.len() - 1].iter().collect();
                return if expr.len() == 1 {
                    resolve_channel(expr[0])
                } else {
                    resolve_channel_expr(&expr)
                };
            }
            resolve_channel_slice(items)
        }
        _ => None,
    }
}

/// Resolve the arguments of a CSS math function to a plain `f32`.
fn resolve_channel_function(fn_name: &str, args: &[Vec<CssValue>]) -> Option<f32> {
    match fn_name {
        "calc" => {
            // calc() has a single argument list: calc(a + b)
            let flat: Vec<&CssValue> = args.iter().flatten().collect();
            resolve_channel_expr(&flat)
        }
        "min" | "max" => {
            // Each comma-separated argument is an independent expression.
            let mut values = Vec::with_capacity(args.len());
            for arg_group in args {
                let flat: Vec<&CssValue> = arg_group.iter().collect();
                values.push(resolve_channel_expr(&flat)?);
            }
            match fn_name {
                "min" => values.into_iter().reduce(f32::min),
                _ => values.into_iter().reduce(f32::max),
            }
        }
        "clamp" => {
            // clamp(min, val, max) — each argument may be a calc expression.
            let refs: Vec<Vec<&CssValue>> = args.iter().map(|a| a.iter().collect()).collect();
            let min = resolve_channel_expr(&refs[0])?;
            let val = resolve_channel_expr(&refs[1])?;
            let max = resolve_channel_expr(&refs[2])?;
            Some(val.clamp(min, max))
        }
        _ => None,
    }
}

/// Evaluate a flat expression slice (e.g. `[a, *, b, +, c]`) to a single `f32`.
///
/// Applies standard operator precedence: `*` and `/` bind tighter than `+` and `-`.
fn resolve_channel_expr(components: &[&CssValue]) -> Option<f32> {
    // Parse multiplication / division first (higher precedence).
    fn parse_product<'a>(
        iter: &mut std::iter::Peekable<impl Iterator<Item = &'a CssValue>>,
    ) -> Option<f32> {
        let mut result = resolve_channel(iter.next()?)?;
        loop {
            let op = match iter.peek() {
                Some(CssValue::Keyword(k)) if k == "*" || k == "/" => iter.next()?,
                _ => break,
            };
            let rhs = resolve_channel(iter.next()?)?;
            result = match op {
                CssValue::Keyword(o) if o == "*" => result * rhs,
                CssValue::Keyword(o) if o == "/" => {
                    if rhs == 0.0 {
                        return None;
                    }
                    result / rhs
                }
                _ => return None,
            };
        }
        Some(result)
    }

    let mut iter = components.iter().copied().peekable();
    let mut result = parse_product(&mut iter)?;
    loop {
        let op = match iter.peek() {
            Some(CssValue::Keyword(k)) if k == "+" || k == "-" => iter.next()?,
            _ => break,
        };
        let rhs = parse_product(&mut iter)?;
        result = match op {
            CssValue::Keyword(o) if o == "+" => result + rhs,
            CssValue::Keyword(o) if o == "-" => result - rhs,
            _ => return None,
        };
    }
    Some(result)
}

/// Resolve a slice of `&CssValue` to a plain `f32`.
/// Used for flat inline expressions like `[Number(10), Keyword("+"), Number(3)]`.
fn resolve_channel_slice(items: &[CssValue]) -> Option<f32> {
    let refs: Vec<&CssValue> = items.iter().collect();
    resolve_channel_expr(&refs)
}

/// Normalize a percentage-range value from `resolve_channel` to 0.0–1.0.
///
/// CSS spec: plain numbers > 1.0 in hsl/hwb are treated as percentages
/// (e.g. `50` = 50% = 0.5). Values ≤ 1.0 are already in the 0.0–1.0 range.
fn normalize_hsl_percent(v: f32) -> f32 {
    if v > 1.0 {
        (v / 100.0).clamp(0.0, 1.0)
    } else {
        v.clamp(0.0, 1.0)
    }
}

/// Resolve a computed CssValue into a final RGBA Color.
///
/// Assumptions:
/// - This function is called *after* cascade and inheritance resolution.
/// - Keywords like `currentColor`, `inherit`, `initial`, `unset`
///   must NOT reach this stage.
/// - The returned Color is always absolute RGBA.
///
/// `color_scheme` is the element's used color scheme, used to resolve
/// `light-dark()`.
pub fn resolve_css_color(
    name: &str,
    css_color: &CssValue,
    color_scheme: ColorScheme,
) -> Option<Color> {
    fn keyword_color_to_color(_name: &str, keyword: &str) -> Option<Color> {
        // NOTE:
        // Keyword matching is case-insensitive according to CSS specs.
        // Keep this list limited to commonly used CSS Color Level 3 keywords.
        match keyword.to_ascii_lowercase().as_str() {
            // ===== Basic =====
            "black" => Some(Color(0, 0, 0, 255)),
            "silver" => Some(Color(192, 192, 192, 255)),
            "gray" | "grey" => Some(Color(128, 128, 128, 255)),
            "white" => Some(Color(255, 255, 255, 255)),

            // ===== Red =====
            "maroon" => Some(Color(128, 0, 0, 255)),
            "red" => Some(Color(255, 0, 0, 255)),
            "firebrick" => Some(Color(178, 34, 34, 255)),
            "crimson" => Some(Color(220, 20, 60, 255)),
            "indianred" => Some(Color(205, 92, 92, 255)),
            "lightcoral" => Some(Color(240, 128, 128, 255)),
            "salmon" => Some(Color(250, 128, 114, 255)),
            "darkred" => Some(Color(139, 0, 0, 255)),
            "darksalmon" => Some(Color(233, 150, 122, 255)),
            "lightsalmon" => Some(Color(255, 160, 122, 255)),

            // ===== Pink =====
            "pink" => Some(Color(255, 192, 203, 255)),
            "lightpink" => Some(Color(255, 182, 193, 255)),
            "hotpink" => Some(Color(255, 105, 180, 255)),
            "deeppink" => Some(Color(255, 20, 147, 255)),
            "palevioletred" => Some(Color(219, 112, 147, 255)),
            "magenta" | "fuchsia" => Some(Color(255, 0, 255, 255)),

            // ===== Orange =====
            "coral" => Some(Color(255, 127, 80, 255)),
            "tomato" => Some(Color(255, 99, 71, 255)),
            "orangered" => Some(Color(255, 69, 0, 255)),
            "orange" => Some(Color(255, 165, 0, 255)),
            "wheat" => Some(Color(245, 222, 179, 255)),

            // ===== Yellow =====
            "beige" => Some(Color(245, 245, 220, 255)),
            "gold" => Some(Color(255, 215, 0, 255)),
            "goldenrod" => Some(Color(218, 165, 32, 255)),
            "yellow" => Some(Color(255, 255, 0, 255)),
            "lightyellow" => Some(Color(255, 255, 224, 255)),
            "lemonchiffon" => Some(Color(255, 250, 205, 255)),
            "lightgoldenrodyellow" => Some(Color(250, 250, 210, 255)),
            "khaki" => Some(Color(240, 230, 140, 255)),
            "papayawhip" => Some(Color(255, 239, 213, 255)),
            "moccasin" => Some(Color(255, 228, 181, 255)),

            // ===== Green =====
            "green" => Some(Color(0, 128, 0, 255)),
            "darkgreen" => Some(Color(0, 100, 0, 255)),
            "forestgreen" => Some(Color(34, 139, 34, 255)),
            "lime" => Some(Color(0, 255, 0, 255)),
            "limegreen" => Some(Color(50, 205, 50, 255)),
            "lightgreen" => Some(Color(144, 238, 144, 255)),
            "olive" => Some(Color(128, 128, 0, 255)),
            "palegreen" => Some(Color(152, 251, 152, 255)),
            "springgreen" => Some(Color(0, 255, 127, 255)),
            "seagreen" => Some(Color(46, 139, 87, 255)),
            "mediumseagreen" => Some(Color(60, 179, 113, 255)),
            "yellowgreen" => Some(Color(154, 205, 50, 255)),

            // ===== Cyan / Aqua =====
            "aqua" | "cyan" => Some(Color(0, 255, 255, 255)),
            "lightcyan" => Some(Color(224, 255, 255, 255)),
            "paleturquoise" => Some(Color(175, 238, 238, 255)),
            "turquoise" => Some(Color(64, 224, 208, 255)),
            "mediumturquoise" => Some(Color(72, 209, 204, 255)),

            // ===== Blue =====
            "blue" => Some(Color(0, 0, 255, 255)),
            "mediumblue" => Some(Color(0, 0, 205, 255)),
            "darkblue" => Some(Color(0, 0, 139, 255)),
            "navy" => Some(Color(0, 0, 128, 255)),
            "royalblue" => Some(Color(65, 105, 225, 255)),
            "teal" => Some(Color(0, 128, 128, 255)),
            "cornflowerblue" => Some(Color(100, 149, 237, 255)),
            "skyblue" => Some(Color(135, 206, 235, 255)),
            "lightblue" => Some(Color(173, 216, 230, 255)),
            "deepskyblue" => Some(Color(0, 191, 255, 255)),

            // ===== Purple =====
            "purple" => Some(Color(128, 0, 128, 255)),
            "indigo" => Some(Color(75, 0, 130, 255)),
            "violet" => Some(Color(238, 130, 238, 255)),
            "plum" => Some(Color(221, 160, 221, 255)),
            "orchid" => Some(Color(218, 112, 214, 255)),
            "mediumpurple" => Some(Color(147, 112, 219, 255)),
            "thistle" => Some(Color(216, 191, 216, 255)),
            "rebeccapurple" => Some(Color(102, 51, 153, 255)),

            // ===== Brown =====
            "bisque" => Some(Color(255, 228, 196, 255)),
            "brown" => Some(Color(165, 42, 42, 255)),
            "saddlebrown" => Some(Color(139, 69, 19, 255)),
            "sienna" => Some(Color(160, 82, 45, 255)),
            "tan" => Some(Color(210, 180, 140, 255)),
            "chocolate" => Some(Color(210, 105, 30, 255)),
            "peru" => Some(Color(205, 133, 63, 255)),
            "burlywood" => Some(Color(222, 184, 135, 255)),

            // ===== White variations =====
            "snow" => Some(Color(255, 250, 250, 255)),
            "honeydew" => Some(Color(240, 255, 240, 255)),
            "mintcream" => Some(Color(245, 255, 250, 255)),
            "ivory" => Some(Color(255, 255, 240, 255)),
            "azure" => Some(Color(240, 255, 255, 255)),
            "aliceblue" => Some(Color(240, 248, 255, 255)),
            "ghostwhite" => Some(Color(248, 248, 255, 255)),
            "linen" => Some(Color(250, 240, 230, 255)),
            "oldlace" => Some(Color(253, 245, 230, 255)),

            // ===== Gray scale =====
            "gainsboro" => Some(Color(220, 220, 220, 255)),
            "lightgray" | "lightgrey" => Some(Color(211, 211, 211, 255)),
            "darkgray" | "darkgrey" => Some(Color(169, 169, 169, 255)),
            "dimgray" | "dimgrey" => Some(Color(105, 105, 105, 255)),
            "lightslategray" | "lightslategrey" => Some(Color(119, 136, 153, 255)),
            "slategray" | "slategrey" => Some(Color(112, 128, 144, 255)),

            // ===== CSS System Colors =====
            "buttonface" => Some(Color(240, 240, 240, 255)),
            "buttontext" => Some(Color(0, 0, 0, 255)),

            "linktext" => Some(Color(0, 0, 238, 255)),
            "visitedtext" => Some(Color(85, 26, 139, 255)),
            "activetext" => Some(Color(255, 0, 0, 255)),

            "canvas" => Some(Color(255, 255, 255, 255)),
            "canvastext" => Some(Color(0, 0, 0, 255)),

            "field" => Some(Color(255, 255, 255, 255)),
            "fieldtext" => Some(Color(0, 0, 0, 255)),

            "highlight" => Some(Color(0, 120, 215, 255)),
            "highlighttext" => Some(Color(255, 255, 255, 255)),

            "graytext" => Some(Color(128, 128, 128, 255)),

            // ===== Special =====
            "transparent" => Some(Color(0, 0, 0, 0)),
            "none" => Some(Color(0, 0, 0, 0)),

            _ => {
                // log::error!(target: "Layouter", "Unknown CSS color keyword `{}` for `{}`", keyword, _name);
                None
            }
        }
    }

    /// Convert HSL to RGB (0..255)
    /// Convert hue degrees (0..360) to fully-saturated RGB (each 0.0..1.0).
    fn hue_to_rgb(h: f32) -> (f32, f32, f32) {
        let h = h.rem_euclid(360.0);
        let sector = h / 60.0;
        let x = 1.0 - (sector % 2.0 - 1.0).abs();
        match sector as u32 {
            0 => (1.0, x, 0.0),
            1 => (x, 1.0, 0.0),
            2 => (0.0, 1.0, x),
            3 => (0.0, x, 1.0),
            4 => (x, 0.0, 1.0),
            _ => (1.0, 0.0, x),
        }
    }

    /// HWB to RGBA conversion per CSS Color Level 4.
    fn hwb_to_rgba(h: f32, w: f32, b: f32, a: f32) -> (u8, u8, u8, u8) {
        let w = w.clamp(0.0, 1.0);
        let b = b.clamp(0.0, 1.0);
        let a = a.clamp(0.0, 1.0);

        let (r, g, bl) = if w + b >= 1.0 {
            let gray = w / (w + b);
            (gray, gray, gray)
        } else {
            let (pr, pg, pb) = hue_to_rgb(h);
            let factor = 1.0 - w - b;
            (pr * factor + w, pg * factor + w, pb * factor + w)
        };

        let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        (to_u8(r), to_u8(g), to_u8(bl), (a * 255.0).round() as u8)
    }

    fn hsla_to_rgba(h: f32, s: f32, l: f32, a: f32) -> (u8, u8, u8, u8) {
        // 1. Compute Chroma
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let h_prime = h / 60.0;
        let x = c * (1.0 - ((h_prime % 2.0) - 1.0).abs());

        // 2. Determine preliminary RGB values based on hue sector
        let (r1, g1, b1) = match h_prime as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            5 | 6 => (c, 0.0, x),
            _ => (0.0, 0.0, 0.0),
        };

        // 3. Add m to match the lightness
        let m = l - c / 2.0;
        let r = ((r1 + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        let g = ((g1 + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        let b = ((b1 + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        let a = (a * 255.0).round().clamp(0.0, 255.0) as u8;

        (r, g, b, a)
    }

    match css_color {
        // Already parsed as an absolute color (rgb/rgba/hex, etc.)
        CssValue::Color(_) => {
            let (r, g, b, a) = css_color.to_rgba_tuple()?;
            Some(Color(r, g, b, a))
        }

        // Named color keyword
        CssValue::Keyword(value) => keyword_color_to_color(name, value),

        // rgb() / rgba() unified
        CssValue::Function(func, args) if func == "rgb" || func == "rgba" => {
            let mut values = Vec::new();
            let mut has_pct = false;
            let mut alpha: Option<f32> = None;
            let mut after_slash = false;

            for arg in args.iter().flatten() {
                match arg {
                    CssValue::Keyword(k) if k == "/" => {
                        after_slash = true;
                    }
                    CssValue::Keyword(k) if k == "%" => {
                        // A bare % after a resolved value means that value is a
                        // percentage. This handles `calc(100/2)%` from the parser.
                        has_pct = true;
                        if after_slash && !values.is_empty() {
                            // Convert the last pushed number to a percentage alpha.
                            let v = values.pop().unwrap();
                            alpha = Some(v / 100.0);
                        }
                    }
                    CssValue::Length(_p, Unit::Percent) => {
                        has_pct = true;
                        let v = resolve_channel(arg)?;
                        if after_slash {
                            alpha = Some(v / 100.0);
                        } else {
                            values.push(v);
                        }
                    }
                    _ => {
                        // Number, calc(), min(), max(), clamp(), var(), etc.
                        let v = resolve_channel(arg)?;
                        if after_slash {
                            alpha = Some(v);
                        } else {
                            values.push(v);
                        }
                    }
                }
            }

            // rgb(r, g, b) -> 3 values
            // rgba(r, g, b, a) -> 4 values
            // rgb(r g b / a) -> 3 values + after_slash
            let (a, values) = if values.len() == 4 && alpha.is_none() {
                (values[3], vec![values[0], values[1], values[2]])
            } else if values.len() == 3 {
                (alpha.unwrap_or(1.0), values)
            } else {
                return None;
            };

            // CSS stores rgb values as 0-255 integers or 0.0-1.0 floats
            // or 0%-100% (already handled above).
            let map_channel = |v: f32| -> f32 {
                if has_pct {
                    v / 100.0 * 255.0
                } else if v > 1.0 {
                    v.clamp(0.0, 255.0)
                } else {
                    v * 255.0
                }
            };

            Some(Color(
                map_channel(values[0]).round() as u8,
                map_channel(values[1]).round() as u8,
                map_channel(values[2]).round() as u8,
                (a * 255.0).round() as u8,
            ))
        }

        // hsl() / hsla() unified
        CssValue::Function(func, args) if func == "hsl" || func == "hsla" => {
            let mut hue_val: Option<f32> = None;
            let mut sat_val: Option<f32> = None;
            let mut light_val: Option<f32> = None;
            let mut alpha: Option<f32> = None;
            let mut after_slash = false;
            let mut channel_index = 0u8;

            for arg in args.iter().flatten() {
                match arg {
                    CssValue::Keyword(k) if k == "/" => {
                        after_slash = true;
                    }
                    CssValue::Keyword(k) if k == "%" => {
                        // Bare % means the *previous* channel is a percentage.
                        // channel_index was already incremented past it.
                        if after_slash {
                            if let Some(ref mut a) = alpha {
                                *a /= 100.0;
                            }
                        } else if channel_index > 0 {
                            match channel_index - 1 {
                                1 => {
                                    if let Some(ref mut s) = sat_val {
                                        *s /= 100.0;
                                    }
                                }
                                2 => {
                                    if let Some(ref mut l) = light_val {
                                        *l /= 100.0;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    CssValue::Length(percent, Unit::Percent) if after_slash => {
                        alpha = Some(percent / 100.0);
                    }
                    CssValue::Length(value, Unit::Deg) if !after_slash => {
                        hue_val = Some(value.rem_euclid(360.0));
                        channel_index += 1;
                    }
                    CssValue::Length(value, Unit::Percent) if !after_slash => {
                        let v = value / 100.0;
                        match channel_index {
                            0 => {
                                hue_val = Some(v.rem_euclid(360.0));
                            }
                            1 => {
                                sat_val = Some(v);
                            }
                            2 => {
                                light_val = Some(v);
                            }
                            _ => {
                                alpha = Some(v);
                            }
                        }
                        channel_index += 1;
                    }
                    _ => {
                        // Number, calc(), min(), max(), clamp(), etc.
                        let v = resolve_channel(arg)?;
                        if after_slash {
                            alpha = Some(v);
                        } else {
                            match channel_index {
                                0 => {
                                    hue_val = Some(v.rem_euclid(360.0));
                                }
                                1 => {
                                    sat_val = Some(normalize_hsl_percent(v));
                                }
                                2 => {
                                    light_val = Some(normalize_hsl_percent(v));
                                }
                                3 => {
                                    alpha = Some(v);
                                }
                                _ => {}
                            }
                            channel_index += 1;
                        }
                    }
                }
            }

            let hue = hue_val?;
            let saturation = sat_val?.clamp(0.0, 1.0);
            let lightness = light_val?.clamp(0.0, 1.0);
            let alpha = alpha.unwrap_or(1.0).clamp(0.0, 1.0);
            let (r, g, b, a) = hsla_to_rgba(hue, saturation, lightness, alpha);

            Some(Color(r, g, b, a))
        }

        // hwb() — hue, whiteness, blackness
        CssValue::Function(func, args) if func == "hwb" => {
            let mut hue_val: Option<f32> = None;
            let mut white_val: Option<f32> = None;
            let mut black_val: Option<f32> = None;
            let mut alpha: Option<f32> = None;
            let mut after_slash = false;
            let mut channel_index = 0u8;

            for arg in args.iter().flatten() {
                match arg {
                    CssValue::Keyword(k) if k == "/" => {
                        after_slash = true;
                    }
                    CssValue::Keyword(k) if k == "%" => {
                        if after_slash {
                            if let Some(ref mut a) = alpha {
                                *a /= 100.0;
                            }
                        } else if channel_index > 0 {
                            match channel_index - 1 {
                                1 => {
                                    if let Some(ref mut w) = white_val {
                                        *w /= 100.0;
                                    }
                                }
                                2 => {
                                    if let Some(ref mut b) = black_val {
                                        *b /= 100.0;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    CssValue::Length(percent, Unit::Percent) if after_slash => {
                        alpha = Some(percent / 100.0);
                    }
                    CssValue::Length(value, Unit::Deg) if !after_slash => {
                        hue_val = Some(value.rem_euclid(360.0));
                        channel_index += 1;
                    }
                    CssValue::Length(value, Unit::Percent) if !after_slash => {
                        let v = value / 100.0;
                        match channel_index {
                            0 => {
                                hue_val = Some(v.rem_euclid(360.0));
                            }
                            1 => {
                                white_val = Some(v);
                            }
                            2 => {
                                black_val = Some(v);
                            }
                            3 => {
                                alpha = Some(v);
                            }
                            _ => {}
                        }
                        channel_index += 1;
                    }
                    _ => {
                        let v = resolve_channel(arg)?;
                        if after_slash {
                            alpha = Some(v);
                        } else {
                            match channel_index {
                                0 => {
                                    hue_val = Some(v.rem_euclid(360.0));
                                }
                                1 => {
                                    white_val = Some(normalize_hsl_percent(v));
                                }
                                2 => {
                                    black_val = Some(normalize_hsl_percent(v));
                                }
                                3 => {
                                    alpha = Some(v);
                                }
                                _ => {}
                            }
                            channel_index += 1;
                        }
                    }
                }
            }

            let hue = hue_val?;
            let w = white_val?.clamp(0.0, 1.0);
            let b = black_val?.clamp(0.0, 1.0);
            let alpha = alpha.unwrap_or(1.0).clamp(0.0, 1.0);

            let (r, g, bl, a) = hwb_to_rgba(hue, w, b, alpha);
            Some(Color(r, g, bl, a))
        }

        // light-dark(<light-color>, <dark-color>)
        CssValue::Function(func, args) if func == "light-dark" && args.len() == 2 => {
            let chosen = match color_scheme {
                ColorScheme::Light => args[0].first()?,
                ColorScheme::Dark => args[1].first()?,
            };
            resolve_css_color(name, chosen, color_scheme)
        }

        // color-mix(in <space>, <color> [<percentage>], <color> [<percentage>])
        CssValue::Function(func, args) if func == "color-mix" => {
            let flat_args: Vec<CssValue> = args.iter().flatten().cloned().collect();
            parse_color_mix(&flat_args, name, color_scheme)
        }

        // Any other value reaching here is a pipeline error
        _ => {
            log::error!(
                target: "Layouter",
                "Unexpected CSS color value for `{}` at layout stage: {:?}",
                name,
                css_color
            );
            None
        }
    }
}
