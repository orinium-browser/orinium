//! SVG support shared by the inline `<svg>` painter and the standalone
//! rasterizer.
//!
//! # Inline SVG
//!
//! The layout builder calls [`collect_svg`] while building the node for an
//! `<svg>` element. The renderer then paints the collected shapes into the
//! element's content box via [`emit_commands`], mapping the `viewBox`
//! rectangle onto the box with [`viewbox_meet_transform`] (`xMidYMid meet`, the
//! SVG default).
//!
//! # Standalone SVG documents
//!
//! Audio icons and `<img src="*.svg">` are rasterized to RGBA pixels by
//! [`rasterize_from_bytes`], which replaces the removed `resvg` dependency.
//!
//! Coverage stays focused on solid-color fills: `<path d>` (including arc
//! segments, expanded into cubic Béziers), `<rect>`, `<circle>`, `<ellipse>`,
//! container `<g>`/`<a>`/`<symbol>` with composed `transform`/`opacity`
//! attributes, `fill-rule`, and `opacity`/`fill-opacity` (approximated as a
//! per-shape alpha multiplier). `<use>`/`<clipPath>`/gradients, strokes, and
//! text are out of scope.

mod rasterize;

pub use rasterize::{RasterResult, rasterize_from_bytes, rasterize_many, svg_to_draw_commands};

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::{Mutex, OnceLock};

use lru::LruCache;

use crate::engine::css::values::CssValue;
use crate::engine::layouter::css_resolver::resolve_inline_value;
use crate::engine::layouter::dom_snapshot::{DomSnapshot, NodeId};
use crate::engine::layouter::resolve_css_color;
use crate::engine::layouter::types::{
    Color, ColorScheme, ColorStop, Gradient, GradientKind, GradientSpread,
};
use crate::engine::renderer_model::Path;
use crate::engine::renderer_model::{
    AffineTransform, Brush, DrawCommand, FillRule, Paint, StrokeCap, StrokeJoin, ellipse_path,
    line_path, polygon_path, polyline_path, rect_path, rounded_rect_path, stroke_path,
    transform_path,
};

/// An `SVG viewBox` rectangle: `(min-x, min-y, width, height)`.
pub type ViewBox = (f32, f32, f32, f32);

/// A single shape inside an inline SVG: a viewBox-space path, its paint
/// (solid color or gradient), and the fill rule (default nonzero). `stroke`
/// is `None` when the shape has no stroke. `clip_path` optionally references a
/// clip path that masks this shape.
#[derive(Debug, Clone)]
pub struct SvgShape {
    pub path: Path,
    pub fill: Brush,
    pub rule: FillRule,
    pub stroke: Option<SvgStroke>,
    pub clip_path: Option<ClipPath>,
}

/// A clipping path: a viewBox-space path plus its fill rule, used to mask
/// shapes that reference it via `clip-path="url(#id)"`.
#[derive(Debug, Clone)]
pub struct ClipPath {
    pub path: Path,
    pub rule: FillRule,
}

/// A resolved SVG stroke: paint color plus the geometry parameters that
/// [`stroke_path`] expands the outline with. When `gradient` is set it paints
/// the outline instead of `color`.
#[derive(Debug, Clone, PartialEq)]
pub struct SvgStroke {
    pub color: Color,
    pub gradient: Option<Gradient>,
    pub width: f32,
    pub cap: StrokeCap,
    pub join: StrokeJoin,
    pub miter_limit: f32,
    pub dash: Option<Vec<f32>>,
    pub dash_offset: f32,
}

impl SvgStroke {
    /// The SVG defaults for an inherited stroke (color is provisional until a
    /// paint value resolves).
    pub(crate) fn inherited_from(inherited: Option<&SvgStroke>) -> SvgStroke {
        inherited.cloned().unwrap_or(SvgStroke {
            color: Color(0, 0, 0, 255),
            gradient: None,
            width: 1.0,
            cap: StrokeCap::Butt,
            join: StrokeJoin::Miter,
            miter_limit: 4.0,
            dash: None,
            dash_offset: 0.0,
        })
    }

    /// The geometry parameters as [`Stroke`] for [`stroke_path`].
    pub(crate) fn to_stroke(&self) -> crate::engine::renderer_model::Stroke {
        crate::engine::renderer_model::Stroke {
            width: self.width,
            cap: self.cap,
            join: self.join,
            miter_limit: self.miter_limit,
            dash: self.dash.clone(),
            dash_offset: self.dash_offset,
        }
    }
}

/// The vector content of an [`crate::engine::layouter::types::NodeKind::Svg`]
/// layout node: the element's `viewBox` and its subpaths in document order.
#[derive(Debug, Clone)]
pub struct SvgContent {
    pub view_box: ViewBox,
    pub shapes: Vec<SvgShape>,
}

/// Translates an SVG path `d` string into a [`Path`].
///
/// Supports the moveto/lineto/curveto/closepath commands in both absolute and
/// relative forms: `M/m`, `L/l`, `H/h`, `V/v`, `C/c`, `S/s`, `Q/q`, `T/t`,
/// `Z/z`. Arc segments (`A/a`) are approximated with cubic Bézier segments via
/// the SVG endpoint→center parametrization (radii are scaled up when they are
/// too small for the chord, and arcs larger than a quarter turn are split).
/// Repeated coordinate sets after a command repeat that command (per the SVG
/// grammar `M m x y ...` implicit lineto included). Malformed data is skipped
/// defensively.
pub fn path_from_d(d: &str) -> Path {
    let mut path = Path::new();
    let mut cur: Option<(f32, f32)> = None;
    let mut start: Option<(f32, f32)> = None;
    // Reflection anchor of the previous curve command (C/S/Q/T), used by the
    // smooth variants. `None` when the previous command was not a curve.
    let mut prev_control: Option<(f32, f32)> = None;
    let mut prev_was_curve = false;

    let mut nums = SvgNum::new(d);

    while let Some(cmd) = nums.next_command() {
        match cmd {
            'M' | 'm' => {
                let rel = cmd == 'm';
                if let Some((x, y)) = nums.next_point(rel, cur) {
                    path.move_to(x, y);
                    cur = Some((x, y));
                    start = Some((x, y));
                    // Extra coordinate pairs are implicit relative lineto.
                    while let Some((x, y)) = nums.next_point(rel, cur) {
                        path.line_to(x, y);
                        cur = Some((x, y));
                    }
                    prev_was_curve = false;
                }
            }
            'L' | 'l' => {
                let rel = cmd == 'l';
                while let Some((x, y)) = nums.next_point(rel, cur) {
                    path.line_to(x, y);
                    cur = Some((x, y));
                }
                prev_was_curve = false;
            }
            'H' | 'h' => {
                let rel = cmd == 'h';
                while let Some(x) = nums.next_number() {
                    let nx = if rel {
                        cur.unwrap_or((0.0, 0.0)).0 + x
                    } else {
                        x
                    };
                    path.line_to(nx, cur.map_or(0.0, |p| p.1));
                    cur = Some((nx, cur.map_or(0.0, |p| p.1)));
                }
                prev_was_curve = false;
            }
            'V' | 'v' => {
                let rel = cmd == 'v';
                while let Some(y) = nums.next_number() {
                    let ny = if rel {
                        cur.unwrap_or((0.0, 0.0)).1 + y
                    } else {
                        y
                    };
                    path.line_to(cur.map_or(0.0, |p| p.0), ny);
                    cur = Some((cur.map_or(0.0, |p| p.0), ny));
                }
                prev_was_curve = false;
            }
            'C' | 'c' => {
                let rel = cmd == 'c';
                let rel_ref = cur.unwrap_or((0.0, 0.0));
                while let (Some(c1), Some(c2), Some(p)) = (
                    nums.next_point(rel, Some(rel_ref)),
                    nums.next_point(rel, Some(rel_ref)),
                    nums.next_point(rel, cur),
                ) {
                    path.cubic_to(c1, c2, p);
                    cur = Some(p);
                    prev_control = Some(c2);
                    prev_was_curve = true;
                }
            }
            'S' | 's' => {
                let rel = cmd == 's';
                let rel_ref = cur.unwrap_or((0.0, 0.0));
                while let (Some(c2), Some(p)) = (
                    nums.next_point(rel, Some(rel_ref)),
                    nums.next_point(rel, cur),
                ) {
                    let c1 = if prev_was_curve {
                        let c2_ref = prev_control.unwrap_or(rel_ref);
                        (2.0 * rel_ref.0 - c2_ref.0, 2.0 * rel_ref.1 - c2_ref.1)
                    } else {
                        rel_ref
                    };
                    path.cubic_to(c1, c2, p);
                    cur = Some(p);
                    prev_control = Some(c2);
                    prev_was_curve = true;
                }
            }
            'Q' | 'q' => {
                let rel = cmd == 'q';
                let rel_ref = cur.unwrap_or((0.0, 0.0));
                while let (Some(c), Some(p)) = (
                    nums.next_point(rel, Some(rel_ref)),
                    nums.next_point(rel, cur),
                ) {
                    path.quad_to(c, p);
                    cur = Some(p);
                    prev_control = Some(c);
                    prev_was_curve = true;
                }
            }
            'T' | 't' => {
                let rel = cmd == 't';
                let rel_ref = cur.unwrap_or((0.0, 0.0));
                while let Some(p) = nums.next_point(rel, cur) {
                    let c = if prev_was_curve {
                        let c_ref = prev_control.unwrap_or(rel_ref);
                        (2.0 * rel_ref.0 - c_ref.0, 2.0 * rel_ref.1 - c_ref.1)
                    } else {
                        rel_ref
                    };
                    path.quad_to(c, p);
                    cur = Some(p);
                    prev_control = Some(c);
                    prev_was_curve = true;
                }
            }
            'A' | 'a' => {
                let rel = cmd == 'a';
                while let (Some(rx), Some(ry), Some(rot), Some(large), Some(sweep), Some(p)) = (
                    nums.next_number(),
                    nums.next_number(),
                    nums.next_number(),
                    nums.next_number(),
                    nums.next_number(),
                    nums.next_point(rel, cur),
                ) {
                    match cur {
                        Some(start) => append_arc(
                            &mut path,
                            start,
                            rx.abs(),
                            ry.abs(),
                            rot,
                            large != 0.0,
                            sweep != 0.0,
                            p,
                        ),
                        // An arc with no current point is a fresh subpath.
                        None => path.move_to(p.0, p.1),
                    }
                    cur = Some(p);
                }
                prev_was_curve = false;
            }
            'Z' | 'z' => {
                path.close();
                cur = start;
                prev_was_curve = false;
            }
            _ => break,
        }
    }

    path
}

/// Approximates an SVG elliptical arc from `from` to `to` with cubic Bézier
/// segments using the SVG 1.1 endpoint→center parametrization (F.6).
///
/// Radii are scaled up when they are too small for the chord, and the sweep is
/// split so no single cubic spans more than a quarter turn, keeping the
/// 4/3·tan(Δ/4) control-point approximation accurate.
#[allow(clippy::too_many_arguments)]
fn append_arc(
    path: &mut Path,
    from: (f32, f32),
    rx: f32,
    ry: f32,
    phi_deg: f32,
    large_arc: bool,
    sweep: bool,
    to: (f32, f32),
) {
    let (x1, y1) = from;
    let (x2, y2) = to;
    if (rx == 0.0 || ry == 0.0)
        || ((x1 - x2).abs() < f32::EPSILON && (y1 - y2).abs() < f32::EPSILON)
    {
        path.line_to(x2, y2);
        return;
    }
    let phi = phi_deg.to_radians();
    let (cos_phi, sin_phi) = (phi.cos(), phi.sin());

    // Endpoints in the rotated (axis-aligned ellipse) frame F.6.5.1.
    let (dx2, dy2) = ((x1 - x2) * 0.5, (y1 - y2) * 0.5);
    let x1p = cos_phi * dx2 + sin_phi * dy2;
    let y1p = -sin_phi * dx2 + cos_phi * dy2;

    // Scale the radii up when they can't span the chord F.6.6.
    let (mut rx, mut ry) = (rx, ry);
    let lambda = (x1p / rx).powi(2) + (y1p / ry).powi(2);
    if lambda > 1.0 {
        let scale = lambda.sqrt();
        rx *= scale;
        ry *= scale;
    }

    // Center in the rotated frame F.6.5.2.
    let rx2 = rx * rx;
    let ry2 = ry * ry;
    let numerator = rx2 * ry2 - rx2 * y1p * y1p - ry2 * x1p * x1p;
    let denominator = rx2 * y1p * y1p + ry2 * x1p * x1p;
    let coef = if denominator <= 0.0 {
        0.0
    } else {
        (numerator / denominator).max(0.0).sqrt()
    };
    let sign = if large_arc == sweep { -1.0 } else { 1.0 };
    let (cxp, cyp) = (
        sign * coef * (rx * y1p / ry),
        sign * coef * (-ry * x1p / rx),
    );

    // World-space center F.6.5.3 and parameter angles F.6.5.5-6. The endpoint
    // is at ellipse-frame angle θ where (x1p, y1p) = (cxp, cyp) + (rx cosθ, ry
    // sinθ), so the angles are those of the radii-scaled vectors.
    let cx = cos_phi * cxp - sin_phi * cyp + (x1 + x2) * 0.5;
    let cy = sin_phi * cxp + cos_phi * cyp + (y1 + y2) * 0.5;
    let theta1 = ((y1p - cyp) / ry).atan2((x1p - cxp) / rx);
    let mut delta = ((-y1p - cyp) / ry).atan2((-x1p - cxp) / rx) - theta1;
    if sweep && delta < 0.0 {
        delta += std::f32::consts::TAU;
    } else if !sweep && delta > 0.0 {
        delta -= std::f32::consts::TAU;
    }

    // Split the sweep into ≤ quarter-turn cubics.
    let segments = (delta.abs() / std::f32::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    for i in 0..segments {
        let a = theta1 + delta * (i as f32 / segments as f32);
        let b = theta1 + delta * ((i + 1) as f32 / segments as f32);
        let k = (4.0 / 3.0) * ((b - a) * 0.25).tan();
        let (sin_a, cos_a) = (a.sin(), a.cos());
        let (sin_b, cos_b) = (b.sin(), b.cos());
        let world = |(px, py): (f32, f32)| {
            (
                cos_phi * px - sin_phi * py + cx,
                sin_phi * px + cos_phi * py + cy,
            )
        };
        // The last segment ends exactly at `to` to avoid a gap from floating
        // point drift in the trigonometric end-point computation.
        let (p3x, p3y) = if i + 1 == segments {
            to
        } else {
            world((rx * cos_b, ry * sin_b))
        };
        let (p1x, p1y) = world((rx * (cos_a - k * sin_a), ry * (sin_a + k * cos_a)));
        let (p2x, p2y) = world((rx * (cos_b + k * sin_b), ry * (sin_b - k * cos_b)));
        path.cubic_to((p1x, p1y), (p2x, p2y), (p3x, p3y));
    }
}

/// Parses an SVG `viewBox` attribute into `(min-x, min-y, width, height)`,
/// defaulting missing components to zero.
pub fn parse_view_box(value: &str) -> ViewBox {
    let mut nums = value
        .split(|c: char| c.is_ascii_whitespace() || c == ',')
        .filter_map(|token| token.parse::<f32>().ok());
    (
        nums.next().unwrap_or(0.0),
        nums.next().unwrap_or(0.0),
        nums.next().unwrap_or(0.0),
        nums.next().unwrap_or(0.0),
    )
}

/// Parses a `points` attribute into a vertex list, tolerating commas,
/// whitespace, and sign-separated coordinates.
pub(crate) fn parse_points(value: &str) -> Vec<(f32, f32)> {
    let mut nums = SvgNum::new(value);
    let mut points = Vec::new();
    while let (Some(x), Some(y)) = (nums.next_number(), nums.next_number()) {
        points.push((x, y));
    }
    points
}

/// Parses an SVG length, tolerating trailing unit suffixes (`px`, `%`, …) and
/// scientific-notation exponents (`1.5e2`).
pub(crate) fn parse_len(value: &str) -> Option<f32> {
    let value = value.trim().trim_start_matches('+');
    let bytes = value.as_bytes();
    let mut i = 0;
    if i < bytes.len() && bytes[i] == b'-' {
        i += 1;
    }
    let digits_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i == digits_start {
        return None;
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        let mut j = i + 1;
        if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
            j += 1;
        }
        if j < bytes.len() && bytes[j].is_ascii_digit() {
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    value[..i].parse::<f32>().ok()
}

/// Parses an SVG `transform` attribute into a single affine transform.
///
/// Supported functions: `matrix`, `translate`, `scale`, `rotate`, `skewX`,
/// `skewY`. Multiple functions compose left-to-right per the SVG spec.
pub(crate) fn parse_transform(value: &str) -> Result<AffineTransform, String> {
    let mut out = AffineTransform::identity();
    let mut rest = value;
    loop {
        let trimmed = rest.trim_start();
        if trimmed.is_empty() {
            return Ok(out);
        }
        let open = trimmed
            .find('(')
            .ok_or_else(|| format!("transform: missing '(' in {value:?}"))?;
        let function = trimmed[..open].trim();
        let inner = &trimmed[open + 1..];
        let close = inner
            .find(')')
            .ok_or_else(|| format!("transform: missing ')' after {function:?} in {value:?}"))?;
        let args = &inner[..close];
        let next = parse_function(function, args).ok_or_else(|| {
            format!("transform: unsupported or malformed {function:?} in {value:?}")
        })?;
        out = out.then(&next);
        rest = &inner[close + 1..];
    }
}

fn parse_function(name: &str, args: &str) -> Option<AffineTransform> {
    let nums = args
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|token| !token.trim().is_empty())
        .filter_map(|token| token.trim().parse::<f32>().ok())
        .collect::<Vec<f32>>();
    match name {
        "matrix" if nums.len() >= 6 => Some(AffineTransform {
            m11: nums[0],
            m12: nums[2],
            m21: nums[1],
            m22: nums[3],
            dx: nums[4],
            dy: nums[5],
        }),
        "translate" if !nums.is_empty() => Some(AffineTransform::translate(
            nums[0],
            nums.get(1).copied().unwrap_or(0.0),
        )),
        "scale" if !nums.is_empty() => Some(AffineTransform::scale(
            nums[0],
            nums.get(1).copied().unwrap_or(nums[0]),
        )),
        "rotate" if !nums.is_empty() => {
            let angle = nums[0].to_radians();
            if nums.len() >= 3 {
                let (cx, cy) = (nums[1], nums[2]);
                Some(
                    AffineTransform::identity()
                        .then(&AffineTransform::translate(-cx, -cy))
                        .then(&AffineTransform::rotate(angle))
                        .then(&AffineTransform::translate(cx, cy)),
                )
            } else {
                Some(AffineTransform::rotate(angle))
            }
        }
        "skewX" if nums.len() == 1 => Some(AffineTransform {
            m12: nums[0].to_radians().tan(),
            ..AffineTransform::identity()
        }),
        "skewY" if nums.len() == 1 => Some(AffineTransform {
            m21: nums[0].to_radians().tan(),
            ..AffineTransform::identity()
        }),
        _ => None,
    }
}

/// Resolves an SVG presentation value such as `fill="#006AFF"` to a color,
/// without round-tripping through a full CSS declaration block.
///
/// `none`/`transparent`/`initial`/`unset` resolve to a transparent color;
/// `currentColor` resolves to the element's computed text color (`inherit`
/// falls back to the SVG default, black, because the tree walker already
/// propagates inherited fills); every other value goes through the standard
/// CSS color pipeline. Unknown values return `None` so callers can apply the
/// SVG default (black).
pub(crate) fn parse_svg_fill(
    value: &str,
    color_scheme: ColorScheme,
    current_color: Color,
) -> Option<Color> {
    let css = resolve_inline_value(value.trim())?;
    match &css {
        CssValue::Keyword(kw) if is_transparent_keyword(kw) => Some(Color(0, 0, 0, 0)),
        CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("currentColor") => Some(current_color),
        CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("inherit") => None,
        // `url(...)` names a gradient paint server, never a solid color. Falling
        // through to the CSS pipeline would log a spurious error and return
        // `None` anyway, so short-circuit to keep the log clean.
        CssValue::Function(name, _) if name.eq_ignore_ascii_case("url") => None,
        _ => resolve_css_color("fill", &css, color_scheme),
    }
}

/// Extracts the fragment `id` from a `url(#id)` paint-server reference like
/// `fill="url(#blinearGradient971)"`. Returns `None` for any other value.
pub(crate) fn parse_svg_url(value: &str) -> Option<String> {
    let css = resolve_inline_value(value.trim())?;
    let CssValue::Function(name, args) = &css else {
        return None;
    };
    if !name.eq_ignore_ascii_case("url") {
        return None;
    }
    // The fragment is either an unquoted identifier or a quoted string.
    let fragment = args.first()?.iter().find_map(|arg| match arg {
        CssValue::Keyword(kw) => Some(kw.to_string()),
        CssValue::String(s) => Some(s.clone()),
        _ => None,
    })?;
    let fragment = fragment.strip_prefix('#').unwrap_or(&fragment);
    (!fragment.is_empty()).then(|| fragment.to_owned())
}

/// Parses a `<use href="...">` reference, accepting both `#id` (bare hash) and
/// `url(#id)` forms. Returns the referenced fragment id.
pub(crate) fn parse_use_ref(value: &str) -> Option<String> {
    let value = value.trim();
    if let Some(id) = value.strip_prefix('#') {
        return (!id.is_empty()).then_some(id.to_owned());
    }
    parse_svg_url(value)
}

fn is_transparent_keyword(keyword: &str) -> bool {
    keyword.eq_ignore_ascii_case("none")
        || keyword.eq_ignore_ascii_case("transparent")
        || keyword.eq_ignore_ascii_case("initial")
        || keyword.eq_ignore_ascii_case("unset")
}

/// Uniform attribute access over the DOM snapshot's [`HtmlNodeType`] and the
/// rasterizer's [`XmlElement`], so fill/rule/opacity resolution (and shape
/// parsing) is shared.
pub(crate) trait SvgElement {
    fn svg_attr(&self, name: &str) -> Option<&str>;
    fn svg_tag_name(&self) -> Option<&str>;
}

impl SvgElement for crate::engine::html::HtmlNodeType {
    fn svg_attr(&self, name: &str) -> Option<&str> {
        self.get_attr(name)
    }
    fn svg_tag_name(&self) -> Option<&str> {
        self.tag_name()
    }
}

impl SvgElement for crate::engine::html::xml::XmlElement {
    fn svg_attr(&self, name: &str) -> Option<&str> {
        self.attr(name)
    }
    fn svg_tag_name(&self) -> Option<&str> {
        Some(&self.name)
    }
}

/// Resolves an SVG `fill` into a concrete color: a `fill` attribute wins over
/// a `style` attribute's `fill:` declaration, then the SVG default (black).
/// `none`/`transparent` produce an invisible fill; `currentColor` resolves to
/// `current_color`.
#[cfg(test)]
pub(crate) fn resolve_shape_fill(
    el: &impl SvgElement,
    color_scheme: ColorScheme,
    current_color: Color,
) -> Color {
    match presentation_value(el, "fill") {
        Some(value) => {
            parse_svg_fill(value, color_scheme, current_color).unwrap_or(Color(0, 0, 0, 255))
        }
        None => Color(0, 0, 0, 255),
    }
}

/// Resolves the `fill-rule` presentation value (`attribute` first, then the
/// `style` declaration), defaulting to nonzero winding.
pub(crate) fn resolve_fill_rule(el: &impl SvgElement) -> FillRule {
    match presentation_value(el, "fill-rule") {
        Some(value) if value.eq_ignore_ascii_case("evenodd") => FillRule::EvenOdd,
        _ => FillRule::NonZero,
    }
}

/// Resolves the `stroke` presentation value into a concrete stroke, threading
/// the current stroke down the tree: a `stroke: none` (or transparent) stops
/// stroking, `currentColor` resolves to `current_color`, and the remaining
/// parameters keep whatever the ancestors declared when this element does not
/// override them. A non-positive `stroke-width` suppresses the stroke. Returns
/// `None` when nothing paints.
pub(crate) fn resolve_stroke(
    el: &impl SvgElement,
    color_scheme: ColorScheme,
    current_color: Color,
    gradients: &HashMap<String, Gradient>,
    inherited: Option<&SvgStroke>,
) -> Option<SvgStroke> {
    let mut stroke = SvgStroke::inherited_from(inherited);
    // Only produce a stroke when the element (or an ancestor) actually
    // declared one; the SVG default is `stroke: none`.
    let mut declared = inherited.is_some();
    if let Some(value) = presentation_value(el, "stroke") {
        if is_transparent_keyword(value.trim()) {
            return None;
        }
        declared = true;
        if let Some(id) = parse_svg_url(value) {
            // A gradient stroke paints even when the reference resolves; an
            // unresolved reference also paints nothing.
            stroke.gradient = Some(gradients.get(&id).cloned()?);
        } else {
            stroke.gradient = None;
            match parse_svg_fill(value, color_scheme, current_color) {
                Some(color) => stroke.color = color,
                None => stroke.color = Color(0, 0, 0, 255),
            }
        }
    }
    if let Some(width) = numeric_presentation_value(el, "stroke-width") {
        stroke.width = width;
    }
    if let Some(value) = presentation_value(el, "stroke-linecap") {
        stroke.cap = match value.trim() {
            v if v.eq_ignore_ascii_case("round") => StrokeCap::Round,
            v if v.eq_ignore_ascii_case("square") => StrokeCap::Square,
            _ => StrokeCap::Butt,
        };
    }
    if let Some(value) = presentation_value(el, "stroke-linejoin") {
        stroke.join = match value.trim() {
            v if v.eq_ignore_ascii_case("round") => StrokeJoin::Round,
            v if v.eq_ignore_ascii_case("bevel") => StrokeJoin::Bevel,
            _ => StrokeJoin::Miter,
        };
    }
    if let Some(limit) = numeric_presentation_value(el, "stroke-miterlimit") {
        stroke.miter_limit = limit.max(1.0);
    }
    if let Some(value) = presentation_value(el, "stroke-dasharray") {
        stroke.dash = parse_dasharray(value);
    }
    if let Some(offset) = numeric_presentation_value(el, "stroke-dashoffset") {
        stroke.dash_offset = offset;
    }
    if !declared || stroke.width <= 0.0 {
        return None;
    }
    let paints = stroke.gradient.is_some() || stroke.color.3 != 0;
    if !paints {
        return None;
    }
    Some(stroke)
}

/// Parses a `stroke-dasharray` value: `none` and empty lists yield `None`;
/// otherwise the whitespace/comma separated non-negative lengths. A pattern
/// that is entirely zero-length paints nothing.
pub(crate) fn parse_dasharray(value: &str) -> Option<Vec<f32>> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("none") {
        return None;
    }
    let lengths: Vec<f32> = value
        .split(',')
        .flat_map(|part| part.split_ascii_whitespace())
        .filter_map(|n| n.parse::<f32>().ok())
        .map(|n| n.max(0.0))
        .collect();
    if lengths.is_empty() || lengths.iter().all(|n| *n <= 0.0) {
        None
    } else {
        Some(lengths)
    }
}

/// The inherited paint properties SVG carries down the tree, so a container's
/// `fill`/`stroke` (and companions) are picked up by its descendants.
#[derive(Debug, Clone)]
pub(crate) struct PaintState {
    pub fill: Option<Color>,
    pub fill_gradient: Option<Gradient>,
    pub fill_opacity: f32,
    pub rule: FillRule,
    pub stroke: Option<SvgStroke>,
    pub stroke_opacity: f32,
}

impl PaintState {
    pub(crate) fn new() -> Self {
        PaintState {
            fill: None,
            fill_gradient: None,
            fill_opacity: 1.0,
            rule: FillRule::NonZero,
            stroke: None,
            stroke_opacity: 1.0,
        }
    }

    /// Overlays `el`'s declarations on the incoming state. `gradients` resolves
    /// `url(#id)` paint-server references to their gradient definitions.
    pub(crate) fn apply(
        &mut self,
        el: &impl SvgElement,
        color_scheme: ColorScheme,
        current_color: Color,
        gradients: &HashMap<String, Gradient>,
    ) {
        if let Some(value) = presentation_value(el, "fill") {
            match parse_svg_url(value) {
                Some(id) => {
                    // A gradient fill overrides any inherited solid fill; an
                    // unresolved reference paints nothing (`none`).
                    if let Some(gradient) = gradients.get(&id) {
                        self.fill_gradient = Some(gradient.clone());
                        self.fill = None;
                    } else {
                        self.fill_gradient = None;
                        self.fill = Some(Color(0, 0, 0, 0));
                    }
                }
                None => {
                    self.fill_gradient = None;
                    self.fill = parse_svg_fill(value, color_scheme, current_color)
                        .or(Some(Color(0, 0, 0, 255)));
                }
            }
        }
        self.fill_opacity *= numeric_presentation_value(el, "fill-opacity")
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        if let Some(value) = presentation_value(el, "fill-rule") {
            self.rule = if value.eq_ignore_ascii_case("evenodd") {
                FillRule::EvenOdd
            } else {
                FillRule::NonZero
            };
        }
        self.stroke_opacity *= numeric_presentation_value(el, "stroke-opacity")
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        self.stroke = resolve_stroke(
            el,
            color_scheme,
            current_color,
            gradients,
            self.stroke.as_ref(),
        );
    }
}

/// Reads a numeric presentation value such as `opacity` or `fill-opacity`
/// (attribute first, then the matching `style` declaration). Malformed or
/// non-finite values return `None`.
pub(crate) fn numeric_presentation_value(el: &impl SvgElement, name: &str) -> Option<f32> {
    presentation_value(el, name)?
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
}

/// Returns the `name` presentation value: the `name` attribute wins, then the
/// matching `<name>:` declaration inside the `style` attribute.
fn presentation_value<'a>(el: &'a impl SvgElement, name: &str) -> Option<&'a str> {
    el.svg_attr(name)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            el.svg_attr("style")
                .and_then(|style| style_property(style, name))
                .map(str::trim)
        })
}

/// Returns the `<name>:` declaration within a `style` attribute.
pub(crate) fn style_property<'a>(style: &'a str, name: &str) -> Option<&'a str> {
    style.split(';').find_map(|pair| {
        let (key, value) = pair.split_once(':')?;
        key.trim().eq_ignore_ascii_case(name).then_some(value)
    })
}

/// Multiplies the alpha channel of `color` by `alpha` (clamped to 0..=1).
pub(crate) fn with_alpha(color: Color, alpha: f32) -> Color {
    let a = (color.3 as f32 * alpha.clamp(0.0, 1.0)).round() as u8;
    Color(color.0, color.1, color.2, a)
}

/// Memoized inline SVG collections, keyed by SVG node id plus a content
/// signature. Re-collecting an unchanged SVG returns the cached [`SvgContent`]
/// without re-walking or re-parsing its subtree (differential update).
static SVG_CONTENT_CACHE: OnceLock<Mutex<LruCache<NodeId, (u64, SvgContent)>>> = OnceLock::new();

fn svg_content_cache() -> &'static Mutex<LruCache<NodeId, (u64, SvgContent)>> {
    SVG_CONTENT_CACHE.get_or_init(|| Mutex::new(LruCache::new(NonZeroUsize::new(64).unwrap())))
}

// Number of times the real SVG collection work ran (cache misses). Test-only
// instrumentation used to assert that unchanged SVGs are not re-collected.
// Kept thread-local so parallel tests do not disturb each other's deltas.
#[cfg(test)]
thread_local! {
    static COLLECT_SVG_MISSES: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
}

/// Hashes everything about the SVG subtree that affects its collected content:
/// the `viewBox` and every descendant's tag name and attributes.
fn svg_subtree_hash(snapshot: &DomSnapshot, svg_id: NodeId) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    fn hash_node(h: &mut u64, snapshot: &DomSnapshot, id: NodeId) {
        let mut mix_byte = |b: u8| {
            *h ^= b as u64;
            *h = (*h).wrapping_mul(0x100000001b3);
        };
        let node = &snapshot.node(id).kind;
        if let crate::engine::html::HtmlNodeType::Element {
            tag_name,
            attributes,
            ..
        } = node
        {
            for b in tag_name.bytes() {
                mix_byte(b);
            }
            let len = attributes.len() as u32;
            for b in len.to_le_bytes() {
                mix_byte(b);
            }
            for attr in attributes {
                for b in attr.name.bytes() {
                    mix_byte(b);
                }
                for b in attr.value.bytes() {
                    mix_byte(b);
                }
            }
        }
        for &child in snapshot.children(id) {
            hash_node(h, snapshot, child);
        }
    }
    hash_node(&mut h, snapshot, svg_id);
    h
}

/// Collects the inline SVG content of an `<svg>` element: its `viewBox` and
/// the solid-color shapes of its content, recursively.
///
/// Container elements (`g`, `a`, `symbol`) are walked with their `transform`
/// and `opacity` attributes composed down the tree. `path`/`rect`/`circle`/
/// `ellipse` become [`SvgShape`]s whose paths are pre-transformed into
/// `viewBox` space, so [`emit_commands`] only maps the `viewBox` onto the
/// content box. `text_color` is the `<svg>` element's computed CSS `color`,
/// used to resolve `fill="currentColor"`. Returns `None` when the element has
/// no usable `viewBox` or no paintable subpaths.
pub fn collect_svg(
    snapshot: &DomSnapshot,
    svg_id: NodeId,
    color_scheme: ColorScheme,
    text_color: Color,
) -> Option<SvgContent> {
    let mut signature = svg_subtree_hash(snapshot, svg_id);
    signature = signature
        .wrapping_add(color_scheme as u64)
        .wrapping_add(text_color.0 as u64)
        .wrapping_add((text_color.1 as u64) << 8)
        .wrapping_add((text_color.2 as u64) << 16)
        .wrapping_add((text_color.3 as u64) << 24);
    {
        let mut cache = svg_content_cache().lock().unwrap();
        if let Some((cached_sig, content)) = cache.get(&svg_id)
            && *cached_sig == signature
        {
            return Some(content.clone());
        }
    }
    #[cfg(test)]
    COLLECT_SVG_MISSES.with(|c| c.fetch_add(1, std::sync::atomic::Ordering::Relaxed));

    let content = {
        let view_box = parse_view_box(snapshot.node(svg_id).kind.get_attr("viewBox")?);
        if view_box.2 <= 0.0 || view_box.3 <= 0.0 {
            return None;
        }
        let mut gradients = HashMap::new();
        for &child in snapshot.children(svg_id) {
            collect_gradient_defs(snapshot, child, color_scheme, text_color, &mut gradients);
        }
        let mut id_map = HashMap::new();
        for &child in snapshot.children(svg_id) {
            collect_id_map(snapshot, child, &mut id_map);
        }
        let mut clip_paths = HashMap::new();
        for &child in snapshot.children(svg_id) {
            collect_clip_path_defs(snapshot, child, &mut clip_paths);
        }
        let mut shapes = Vec::new();
        let mut use_stack: Vec<NodeId> = Vec::new();
        for &child in snapshot.children(svg_id) {
            collect_content(
                snapshot,
                child,
                &AffineTransform::identity(),
                1.0,
                color_scheme,
                text_color,
                PaintState::new(),
                &gradients,
                &id_map,
                &clip_paths,
                &mut use_stack,
                &mut shapes,
            );
        }
        if shapes.is_empty() {
            return None;
        }
        SvgContent { view_box, shapes }
    };
    svg_content_cache()
        .lock()
        .unwrap()
        .put(svg_id, (signature, content.clone()));
    Some(content)
}

/// Collects `<clipPath>` definitions under `root` into `out`, keyed by their
/// `id` attribute. All shape descendants of a `<clipPath>` are combined into a
/// single path (transformed by the clipPath element's `transform` attribute).
fn collect_clip_path_defs(snapshot: &DomSnapshot, id: NodeId, out: &mut HashMap<String, ClipPath>) {
    let el = &snapshot.node(id).kind;
    if el.tag_name() == Some("clipPath")
        && let Some(clip_id) = el.get_attr("id").filter(|id| !id.is_empty())
    {
        let mut path = Path::new();
        let transform = match el.get_attr("transform") {
            Some(value) => parse_transform(value).unwrap_or(AffineTransform::identity()),
            None => AffineTransform::identity(),
        };
        let rule = resolve_fill_rule(el);
        collect_clip_path_shapes(snapshot, id, &transform, &mut path);
        if !path.commands().is_empty() {
            out.entry(clip_id.to_owned())
                .or_insert(ClipPath { path, rule });
        }
    }
    for &child in snapshot.children(id) {
        collect_clip_path_defs(snapshot, child, out);
    }
}

/// Recursively collects all shape descendants under `id` into `path`, applying
/// `transform` to each shape's geometry.
fn collect_clip_path_shapes(
    snapshot: &DomSnapshot,
    id: NodeId,
    transform: &AffineTransform,
    path: &mut Path,
) {
    let el = &snapshot.node(id).kind;
    if let Some(shape_path) = element_shape(el) {
        let transformed = transform_path(&shape_path, transform);
        // Append the transformed subpaths into the combined path.
        for subpath in transformed.subpaths() {
            if let Some(&first) = subpath.first() {
                path.move_to(first.0, first.1);
                for &(x, y) in &subpath[1..] {
                    path.line_to(x, y);
                }
                path.close();
            }
        }
    }
    for &child in snapshot.children(id) {
        collect_clip_path_shapes(snapshot, child, transform, path);
    }
}

/// Builds a map of `id` attribute values to node ids for all elements under
/// `root`, so `<use href="#id">` can resolve its target.
pub(crate) fn collect_id_map(
    snapshot: &DomSnapshot,
    id: NodeId,
    out: &mut HashMap<String, NodeId>,
) {
    let el = &snapshot.node(id).kind;
    if let Some(id_value) = el.get_attr("id")
        && !id_value.is_empty()
    {
        out.entry(id_value.to_owned()).or_insert(id);
    }
    for &child in snapshot.children(id) {
        collect_id_map(snapshot, child, out);
    }
}

/// Collects the `<linearGradient>`/`<radialGradient>` definitions reachable
/// under `id` (typically inside a `<defs>` block) into `out`, keyed by their
/// `id` attribute.
fn collect_gradient_defs(
    snapshot: &DomSnapshot,
    id: NodeId,
    color_scheme: ColorScheme,
    text_color: Color,
    out: &mut HashMap<String, Gradient>,
) {
    let el = &snapshot.node(id).kind;
    match el.tag_name() {
        Some("linearGradient") => {
            let stops = collect_stops(snapshot, id, color_scheme, text_color);
            if let Some(gradient) = parse_linear_gradient(el, &stops, color_scheme, text_color)
                && let Some(gid) = el.get_attr("id").filter(|id| !id.is_empty())
            {
                out.entry(gid.to_owned()).or_insert(gradient);
            }
        }
        Some("radialGradient") => {
            let stops = collect_stops(snapshot, id, color_scheme, text_color);
            if let Some(gradient) = parse_radial_gradient(el, &stops, color_scheme, text_color)
                && let Some(gid) = el.get_attr("id").filter(|id| !id.is_empty())
            {
                out.entry(gid.to_owned()).or_insert(gradient);
            }
        }
        _ => {}
    }
    for &child in snapshot.children(id) {
        collect_gradient_defs(snapshot, child, color_scheme, text_color, out);
    }
}

/// Resolves a gradient's geometry from its attributes. Both gradient kinds share
/// the `gradientUnits`/`spreadMethod` handling.
fn gradient_units_and_spread(el: &impl SvgElement) -> (bool, GradientSpread) {
    let user_space = el
        .svg_attr("gradientUnits")
        .map(|v| v.trim().eq_ignore_ascii_case("userSpaceOnUse"))
        .unwrap_or(false);
    let spread = match el
        .svg_attr("spreadMethod")
        .map(|v| v.trim())
        .unwrap_or("pad")
    {
        v if v.eq_ignore_ascii_case("repeat") => GradientSpread::Repeat,
        v if v.eq_ignore_ascii_case("reflect") => GradientSpread::Reflect,
        _ => GradientSpread::Pad,
    };
    (user_space, spread)
}

/// Parses an `<linearGradient>` element's geometry around already-resolved
/// color stops. The defs live in `viewBox`/bbox space, so no transform is
/// applied here.
fn parse_linear_gradient(
    el: &impl SvgElement,
    stops: &[ColorStop],
    _color_scheme: ColorScheme,
    _text_color: Color,
) -> Option<Gradient> {
    let (user_space, spread) = gradient_units_and_spread(el);
    if stops.is_empty() {
        return None;
    }
    Some(Gradient {
        kind: GradientKind::SvgLinear {
            x1: el.svg_attr("x1").and_then(parse_len).unwrap_or(0.0),
            y1: el.svg_attr("y1").and_then(parse_len).unwrap_or(0.0),
            x2: el.svg_attr("x2").and_then(parse_len).unwrap_or(1.0),
            y2: el.svg_attr("y2").and_then(parse_len).unwrap_or(0.0),
            user_space,
        },
        stops: stops.to_vec(),
        spread,
    })
}

/// Parses a `<radialGradient>` element's geometry. A missing focal point
/// degrades to the center (`fx/fy` inherit `cx/cy` per the SVG spec).
fn parse_radial_gradient(
    el: &impl SvgElement,
    stops: &[ColorStop],
    _color_scheme: ColorScheme,
    _text_color: Color,
) -> Option<Gradient> {
    let (user_space, spread) = gradient_units_and_spread(el);
    let cx = el.svg_attr("cx").and_then(parse_len).unwrap_or(0.5);
    let cy = el.svg_attr("cy").and_then(parse_len).unwrap_or(0.5);
    if stops.is_empty() {
        return None;
    }
    Some(Gradient {
        kind: GradientKind::SvgRadial {
            cx,
            cy,
            r: el.svg_attr("r").and_then(parse_len).unwrap_or(0.5),
            fx: el.svg_attr("fx").and_then(parse_len).unwrap_or(cx),
            fy: el.svg_attr("fy").and_then(parse_len).unwrap_or(cy),
            fr: el.svg_attr("fr").and_then(parse_len).unwrap_or(0.0),
            user_space,
        },
        stops: stops.to_vec(),
        spread,
    })
}

/// Resolves the direct `<stop>` children of the gradient element `id`,
/// resolving each stop's color, opacity, and offset. Unparsable stops are
/// skipped.
fn collect_stops(
    snapshot: &DomSnapshot,
    id: NodeId,
    color_scheme: ColorScheme,
    text_color: Color,
) -> Vec<ColorStop> {
    let mut out = Vec::new();
    for &child in snapshot.children(id) {
        let el = &snapshot.node(child).kind;
        if el.tag_name() != Some("stop") {
            continue;
        }
        let offset = el.get_attr("offset").and_then(parse_stop_offset);
        let color = presentation_value(el, "stop-color")
            .and_then(|value| parse_svg_fill(value, color_scheme, text_color))
            .unwrap_or(Color(0, 0, 0, 255));
        let opacity = numeric_presentation_value(el, "stop-opacity")
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        out.push(ColorStop {
            color: with_alpha(color, opacity),
            position: offset,
        });
    }
    out
}

/// Parses an SVG gradient stop `offset` (`0.5`, `50%`, …) into a 0..1 fraction.
fn parse_stop_offset(value: &str) -> Option<f32> {
    let value = value.trim();
    if let Some(percent) = value.strip_suffix('%') {
        return parse_len(percent).map(|v| (v / 100.0).clamp(0.0, 1.0));
    }
    parse_len(value).map(|v| v.clamp(0.0, 1.0))
}

/// Recursively gathers the paintable shapes under `id`, composing `transform`
/// and `opacity` into the inherited values as it walks containers. `paint`
/// carries the inherited `fill`/`stroke` state that containers set for their
/// descendants.
#[allow(clippy::too_many_arguments)]
fn collect_content(
    snapshot: &DomSnapshot,
    id: NodeId,
    inherited: &AffineTransform,
    opacity: f32,
    color_scheme: ColorScheme,
    text_color: Color,
    mut paint: PaintState,
    gradients: &HashMap<String, Gradient>,
    id_map: &HashMap<String, NodeId>,
    clip_paths: &HashMap<String, ClipPath>,
    use_stack: &mut Vec<NodeId>,
    shapes: &mut Vec<SvgShape>,
) {
    let el = &snapshot.node(id).kind;
    let eff = match el.get_attr("transform") {
        Some(value) => match parse_transform(value) {
            Ok(transform) => inherited.then(&transform),
            Err(_) => *inherited,
        },
        None => *inherited,
    };
    let element_opacity = numeric_presentation_value(el, "opacity").unwrap_or(1.0);
    let opacity = opacity * element_opacity.clamp(0.0, 1.0);
    paint.apply(el, color_scheme, text_color, gradients);

    match el.tag_name() {
        Some("g" | "a" | "symbol") => {
            for &child in snapshot.children(id) {
                collect_content(
                    snapshot,
                    child,
                    &eff,
                    opacity,
                    color_scheme,
                    text_color,
                    paint.clone(),
                    gradients,
                    id_map,
                    clip_paths,
                    use_stack,
                    shapes,
                );
            }
        }
        Some("use") => {
            // `<use href="#id">` clones the referenced element's content at the
            // use element's position, with x/y translating the content.
            let href = el.svg_attr("href").or_else(|| el.svg_attr("xlink:href"));
            let target_id = href
                .and_then(parse_use_ref)
                .and_then(|frag| id_map.get(&frag).copied());
            if let Some(target) = target_id {
                // Circular reference detection: bail out if the target is
                // already being expanded by an ancestor `<use>`.
                if !use_stack.contains(&target) {
                    let x = el.svg_attr("x").and_then(parse_len).unwrap_or(0.0);
                    let y = el.svg_attr("y").and_then(parse_len).unwrap_or(0.0);
                    let use_transform = eff.then(&AffineTransform::translate(x, y));
                    use_stack.push(target);
                    collect_content(
                        snapshot,
                        target,
                        &use_transform,
                        opacity,
                        color_scheme,
                        text_color,
                        paint.clone(),
                        gradients,
                        id_map,
                        clip_paths,
                        use_stack,
                        shapes,
                    );
                    use_stack.pop();
                }
            }
        }
        Some("path" | "rect" | "circle" | "ellipse" | "polygon" | "polyline" | "line") => {
            let Some(path) = element_shape(el) else {
                return;
            };
            let path = transform_path(&path, &eff);
            if path.commands().is_empty() {
                return;
            }
            let fill_alpha = opacity * paint.fill_opacity;
            let fill = match &paint.fill_gradient {
                Some(gradient) => Brush::Gradient(with_stop_alpha(gradient.clone(), fill_alpha)),
                None => Brush::Solid(with_alpha(
                    paint.fill.unwrap_or(Color(0, 0, 0, 255)),
                    fill_alpha,
                )),
            };
            let stroke = paint.stroke.clone().map(|mut stroke| {
                if let Some(gradient) = &mut stroke.gradient {
                    *gradient = with_stop_alpha(gradient.clone(), opacity * paint.stroke_opacity);
                } else {
                    stroke.color = with_alpha(stroke.color, opacity * paint.stroke_opacity);
                }
                stroke
            });
            let clip_path = el
                .svg_attr("clip-path")
                .and_then(parse_use_ref)
                .and_then(|frag| clip_paths.get(&frag).cloned());
            shapes.push(SvgShape {
                path,
                fill,
                rule: paint.rule,
                stroke,
                clip_path,
            });
        }
        _ => {}
    }
}

/// Scales every gradient stop's alpha by `alpha` (clamped). Used to fold an
/// element's `opacity` and the fill/stroke opacity into the painted gradient,
/// mirroring how solid paints bake those into their alpha channel.
fn with_stop_alpha(gradient: Gradient, alpha: f32) -> Gradient {
    let mut gradient = gradient;
    for stop in &mut gradient.stops {
        stop.color = with_alpha(stop.color, alpha);
    }
    gradient
}

/// Builds the shape-space path an element describes, or `None` when the element
/// is malformed or has no area.
fn element_shape(el: &impl SvgElement) -> Option<Path> {
    match el.svg_tag_name()? {
        "path" => {
            let d = el.svg_attr("d")?;
            let path = path_from_d(d);
            (!path.commands().is_empty()).then_some(path)
        }
        "rect" => {
            let x = el.svg_attr("x").and_then(parse_len).unwrap_or(0.0);
            let y = el.svg_attr("y").and_then(parse_len).unwrap_or(0.0);
            let w = el.svg_attr("width").and_then(parse_len).unwrap_or(0.0);
            let h = el.svg_attr("height").and_then(parse_len).unwrap_or(0.0);
            let rx = el
                .svg_attr("rx")
                .and_then(parse_len)
                .unwrap_or(0.0)
                .max(0.0);
            let ry = el
                .svg_attr("ry")
                .and_then(parse_len)
                .unwrap_or(0.0)
                .max(0.0);
            if w <= 0.0 || h <= 0.0 {
                return None;
            }
            if rx > 0.0 || ry > 0.0 {
                // A single rx/ry applies to every corner, per the SVG grammar.
                let radius = (rx, ry);
                Some(rounded_rect_path(
                    x, y, w, h, radius, radius, radius, radius,
                ))
            } else {
                Some(rect_path(x, y, w, h))
            }
        }
        "circle" | "ellipse" => {
            let cx = el.svg_attr("cx").and_then(parse_len).unwrap_or(0.0);
            let cy = el.svg_attr("cy").and_then(parse_len).unwrap_or(0.0);
            let (rx, ry) = if el.svg_tag_name() == Some("circle") {
                let r = el.svg_attr("r").and_then(parse_len).unwrap_or(0.0);
                (r, r)
            } else {
                (
                    el.svg_attr("rx").and_then(parse_len).unwrap_or(0.0),
                    el.svg_attr("ry").and_then(parse_len).unwrap_or(0.0),
                )
            };
            (rx > 0.0 && ry > 0.0).then_some(ellipse_path(cx, cy, rx, ry))
        }
        "polygon" => {
            let points = parse_points(el.svg_attr("points").unwrap_or_default());
            (!points.is_empty()).then(|| polygon_path(&points))
        }
        "polyline" => {
            let points = parse_points(el.svg_attr("points").unwrap_or_default());
            (!points.is_empty()).then(|| polyline_path(&points))
        }
        "line" => {
            let (x1, y1) = (
                el.svg_attr("x1").and_then(parse_len).unwrap_or(0.0),
                el.svg_attr("y1").and_then(parse_len).unwrap_or(0.0),
            );
            let (x2, y2) = (
                el.svg_attr("x2").and_then(parse_len).unwrap_or(0.0),
                el.svg_attr("y2").and_then(parse_len).unwrap_or(0.0),
            );
            Some(line_path(x1, y1, x2, y2))
        }
        _ => None,
    }
}

/// Builds the transform mapping `viewBox` coordinates into a viewport using
/// `xMidYMid meet` (the SVG default: fit the whole box inside the viewport,
/// centered). Without a usable `viewBox`, coordinates map 1:1.
pub fn viewbox_meet_transform(view_box: ViewBox, vw: f32, vh: f32) -> AffineTransform {
    let (vx, vy, vbw, vbh) = view_box;
    if vbw <= 0.0 || vbh <= 0.0 {
        return AffineTransform::identity();
    }
    let scale = (vw / vbw).min(vh / vbh);
    let tx = (vw - vbw * scale) * 0.5;
    let ty = (vh - vbh * scale) * 0.5;
    AffineTransform {
        m11: scale,
        m22: scale,
        dx: tx - vx * scale,
        dy: ty - vy * scale,
        ..AffineTransform::identity()
    }
}

/// Paints the shapes of an inline SVG into its content box.
///
/// Pushes a transform mapping the `viewBox` rectangle onto the content box
/// with [`viewbox_meet_transform`] (letterboxing when the aspect ratios
/// differ), emits one `Fill` per shape in document order, then pops the
/// transform. The caller must have already pushed the box model:
/// `push_box_model` translates block boxes to their content-box origin, and
/// inline boxes push no transform (so `is_inline` paints in absolute
/// coordinates). Paints nothing when the box or `viewBox` is degenerate.
pub fn emit_commands(
    cmd_buf: &mut Vec<DrawCommand>,
    content_box: (f32, f32, f32, f32),
    is_inline: bool,
    view_box: ViewBox,
    shapes: &[SvgShape],
    opacity: f32,
) {
    let (cx, cy, cw, ch) = content_box;
    if cw <= 0.0 || ch <= 0.0 {
        return;
    }
    let (ox, oy) = if is_inline { (cx, cy) } else { (0.0, 0.0) };
    let mut transform = viewbox_meet_transform(view_box, cw, ch);
    transform.dx += ox;
    transform.dy += oy;
    cmd_buf.push(DrawCommand::PushTransform { transform });
    for shape in shapes {
        if let Some(clip) = &shape.clip_path {
            cmd_buf.push(DrawCommand::PushClip {
                path: clip.path.clone(),
                rule: clip.rule,
            });
        }
        cmd_buf.push(DrawCommand::Fill {
            path: shape.path.clone(),
            paint: Paint {
                brush: shape.fill.clone(),
                opacity,
            },
            rule: shape.rule,
        });
        if let Some(stroke) = &shape.stroke {
            // The stroke outline is expanded in viewBox space (the same space
            // the shape's path lives in) and filled with the nonzero rule by
            // the same pipeline as the shape fill.
            let brush = match &stroke.gradient {
                Some(gradient) => Brush::Gradient(gradient.clone()),
                None => Brush::Solid(stroke.color),
            };
            cmd_buf.push(DrawCommand::Fill {
                path: stroke_path(&shape.path, &stroke.to_stroke()),
                paint: Paint { brush, opacity },
                rule: FillRule::NonZero,
            });
        }
        if shape.clip_path.is_some() {
            cmd_buf.push(DrawCommand::PopClip);
        }
    }
    cmd_buf.push(DrawCommand::PopTransform);
}

/// SVG path number lexer: reads sign-separated floats per the path `d`
/// grammar, tracking the command letter position.
struct SvgNum {
    chars: Vec<char>,
    pos: usize,
}

impl SvgNum {
    fn new(d: &str) -> Self {
        SvgNum {
            chars: d.chars().collect(),
            pos: 0,
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            if c.is_ascii_whitespace() || c == ',' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    /// Next command letter (consuming it), or `None` at end of data.
    fn next_command(&mut self) -> Option<char> {
        self.skip_ws();
        let c = *self.chars.get(self.pos)?;
        if c.is_ascii_alphabetic() {
            self.pos += 1;
            Some(c)
        } else {
            None
        }
    }

    fn next_number(&mut self) -> Option<f32> {
        self.skip_ws();
        if self.pos >= self.chars.len() {
            return None;
        }
        // A command letter terminates the current coordinate list.
        if self.chars[self.pos].is_ascii_alphabetic() {
            return None;
        }
        let start = self.pos;
        let mut i = self.pos;
        if i < self.chars.len() && (self.chars[i] == '+' || self.chars[i] == '-') {
            i += 1;
        }
        let mut digits = 0;
        while i < self.chars.len() && self.chars[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
        if i < self.chars.len() && self.chars[i] == '.' {
            i += 1;
            while i < self.chars.len() && self.chars[i].is_ascii_digit() {
                i += 1;
                digits += 1;
            }
        }
        // A lone '.' is a valid zero in SVG data.
        if digits == 0 && start == self.pos {
            return None;
        }
        // Exponent.
        if i < self.chars.len() && (self.chars[i] == 'e' || self.chars[i] == 'E') {
            let mut j = i + 1;
            if j < self.chars.len() && (self.chars[j] == '+' || self.chars[j] == '-') {
                j += 1;
            }
            if j < self.chars.len() && self.chars[j].is_ascii_digit() {
                i = j;
                while i < self.chars.len() && self.chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
        }
        // Reject trailing decimal point with no digits (e.g. "10.5M").
        let text: String = self.chars[start..i].iter().collect();
        match text.parse::<f32>() {
            Ok(v) => {
                self.pos = i;
                Some(v)
            }
            Err(_) => None,
        }
    }

    /// A relative coordinate is relative to `ref` (the current point).
    fn next_value(&mut self, rel: bool, ref_val: Option<f32>) -> Option<f32> {
        let v = self.next_number()?;
        if rel {
            Some(ref_val.unwrap_or(0.0) + v)
        } else {
            Some(v)
        }
    }

    fn next_point(&mut self, rel: bool, cur: Option<(f32, f32)>) -> Option<(f32, f32)> {
        let ref_pt = cur.unwrap_or((0.0, 0.0));
        let x = self.next_value(rel, Some(ref_pt.0))?;
        let y = self.next_value(rel, Some(ref_pt.1))?;
        Some((x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::html::HtmlNodeType;
    use crate::engine::layouter::dom_snapshot::DomSnapshot;
    use crate::engine::renderer_model::PathCommand;
    use crate::engine::tree::TreeNode;
    use std::rc::Rc;

    fn element(tag: &str, attrs: &[(&str, &str)]) -> HtmlNodeType {
        HtmlNodeType::Element {
            tag_name: tag.to_string(),
            attributes: attrs
                .iter()
                .map(
                    |(name, value)| crate::engine::html::parser::OwnedAttribute {
                        name: name.to_string(),
                        value: value.to_string(),
                    },
                )
                .collect(),
        }
    }

    #[test]
    fn smooth_curve_path_skips_previous_non_curve() {
        // A previous non-curve command forces S' first control to the current
        // point instead of a reflection.
        let path = path_from_d("M 10 10 L 20 10 S 30 0, 40 10");
        match &path.commands()[2] {
            PathCommand::CubicTo { c1x, c1y, .. } => {
                assert_eq!((*c1x, *c1y), (20.0, 10.0));
            }
            other => panic!("expected cubic, got {other:?}"),
        }
    }

    #[test]
    fn path_parses_absolute_moveto_lineto_and_close() {
        let path = path_from_d("M 0 0 L 10 0 L 10 10 L 0 10 Z");
        assert_eq!(
            path.subpaths(),
            vec![vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]]
        );
    }

    #[test]
    fn path_parses_relative_implicit_lineto_and_hv() {
        // `m 1 1 3 0` is a relative moveto plus an implicit relative lineto;
        // `h 2` / `v 2` are relative to the current point.
        let path = path_from_d("m 1 1 3 0 h 2 v 2 z");
        assert_eq!(
            path.subpaths(),
            vec![vec![(1.0, 1.0), (4.0, 1.0), (6.0, 1.0), (6.0, 3.0)]]
        );
    }

    #[test]
    fn path_cubic_s_reflects_previous_control_point() {
        // After `C 10 10, 20 10, 30 0`, S reflects (20,10) about (30,0) → (40,-10).
        let path = path_from_d("M 0 0 C 10 10, 20 10, 30 0 S 50 -10, 60 0");
        match &path.commands()[2] {
            PathCommand::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => {
                assert_eq!((*c1x, *c1y), (40.0, -10.0));
                assert_eq!((*c2x, *c2y), (50.0, -10.0));
                assert_eq!((*x, *y), (60.0, 0.0));
            }
            other => panic!("expected cubic, got {other:?}"),
        }
    }

    #[test]
    fn path_quad_t_reflects_previous_control_point() {
        // After `Q 10 20, 20 0`, T reflects (10,20) about (20,0) → (30,-20).
        let path = path_from_d("M 0 0 Q 10 20 20 0 T 40 0");
        match &path.commands()[2] {
            PathCommand::QuadTo { cx, cy, x, y } => {
                assert_eq!((*cx, *cy), (30.0, -20.0));
                assert_eq!((*x, *y), (40.0, 0.0));
            }
            other => panic!("expected quad, got {other:?}"),
        }
    }

    #[test]
    fn arc_is_approximated_by_cubics_staying_on_the_circle() {
        // Semicircle from (0,0) to (10,0) at radius 5, center (5,0).
        let path = path_from_d("M 0 0 A 5 5 0 0 1 10 0");
        assert!(matches!(&path.commands()[0], PathCommand::MoveTo { .. }));
        let cubics = path
            .commands()
            .iter()
            .filter(|c| matches!(c, PathCommand::CubicTo { .. }))
            .count();
        assert!(cubics >= 2, "expected ≥ 2 cubics, got {cubics}");
        let ring = path.subpaths().into_iter().next().expect("one ring");
        assert_close(ring.first().copied().unwrap(), (0.0, 0.0));
        assert_close(ring.last().copied().unwrap(), (10.0, 0.0));
        for &(x, y) in &ring {
            let v = ((x - 5.0) / 5.0).powi(2) + (y / 5.0).powi(2);
            assert!((v - 1.0).abs() < 0.05, "point ({x},{y}) off circle: {v}");
        }
        // The arc bows up to the circle top instead of cutting straight across.
        assert!(
            ring.iter().map(|&(_, y)| y.abs()).fold(0.0f32, f32::max) > 4.0,
            "arc did not bulge"
        );
    }

    #[test]
    fn arc_sweep_flag_selects_the_complementary_side() {
        // Same radii/endpoints: sweep=1 swings through negative y, sweep=0
        // through positive y (y-down screen coordinates).
        let sweep1 = path_from_d("M 0 0 A 5 5 0 0 1 10 0");
        let sweep0 = path_from_d("M 0 0 A 5 5 0 0 0 10 0");
        let extreme_y = |p: &Path, f: fn(f32, f32) -> f32| {
            p.subpaths()[0].iter().map(|&(_, y)| y).fold(f32::NAN, &f)
        };
        // sweep=1 reaches the top (most negative y); sweep=0 the bottom.
        assert!(
            extreme_y(&sweep1, f32::min) < -4.0,
            "sweep=1 should go over the top"
        );
        assert!(
            extreme_y(&sweep0, f32::max) > 4.0,
            "sweep=0 should go under the bottom"
        );
    }

    #[test]
    fn arc_large_flag_takes_the_long_way_around() {
        // Chord 6 < diameter 10: the small arc is < 180° (a single cubic),
        // the large arc is the remaining sweep (split into several).
        let small = path_from_d("M 0 0 A 5 5 0 0 1 6 0");
        let large = path_from_d("M 0 0 A 5 5 0 1 1 6 0");
        let cubics = |p: &Path| {
            p.commands()
                .iter()
                .filter(|c| matches!(c, PathCommand::CubicTo { .. }))
                .count()
        };
        assert!(
            cubics(&large) > cubics(&small),
            "large arc should split more"
        );
        for p in [&small, &large] {
            let ring = &p.subpaths()[0];
            assert_close(ring.first().copied().unwrap(), (0.0, 0.0));
            assert_close(ring.last().copied().unwrap(), (6.0, 0.0));
        }
    }

    #[test]
    fn arc_scales_radii_up_when_the_chord_exceeds_the_radius() {
        // rx=3 cannot span the (0,0)→(10,0) chord; radii must be scaled to 5.
        let path = path_from_d("M 0 0 A 3 3 0 0 1 10 0");
        let ring = &path.subpaths()[0];
        assert_close(ring.first().copied().unwrap(), (0.0, 0.0));
        assert_close(ring.last().copied().unwrap(), (10.0, 0.0));
        for &(x, y) in ring {
            let v = ((x - 5.0) / 5.0).powi(2) + (y / 5.0).powi(2);
            assert!((v - 1.0).abs() < 0.05, "point ({x},{y}) off circle: {v}");
        }
    }

    #[test]
    fn arc_rotates_ellipse_with_x_axis_rotation() {
        // An ellipse rotated 45°, radii (20,10), chord length 20 fits inside
        // (λ ≤ 1), so no radii scaling. Verify every flattened vertex sits on
        // the ellipse whose center follows the spec's endpoint→center
        // parametrization.
        let path = path_from_d("M 0 0 A 20 10 45 0 1 20 0");
        let ring = &path.subpaths()[0];
        let phi = 45f32.to_radians();
        let (cos_p, sin_p) = (phi.cos(), phi.sin());
        let (x1p, y1p) = (cos_p * -10.0, -sin_p * -10.0);
        let num = 400.0 * 100.0 - 400.0 * y1p * y1p - 100.0 * x1p * x1p;
        let den = 400.0 * y1p * y1p + 100.0 * x1p * x1p;
        let coef = (num / den).max(0.0).sqrt();
        // large(0) != sweep(1), so the center is offset by +coef.
        let (cxp, cyp) = (coef * (20.0 * y1p / 10.0), coef * (-10.0 * x1p / 20.0));
        let (cx, cy) = (cos_p * cxp - sin_p * cyp + 10.0, sin_p * cxp + cos_p * cyp);
        for &(x, y) in ring {
            let (dx, dy) = (x - cx, y - cy);
            let (rx_local, ry_local) = (dx * cos_p + dy * sin_p, -dx * sin_p + dy * cos_p);
            let v = (rx_local / 20.0).powi(2) + (ry_local / 10.0).powi(2);
            assert!((v - 1.0).abs() < 0.05, "point ({x},{y}) off ellipse: {v}");
        }
    }

    fn assert_close(a: (f32, f32), b: (f32, f32)) {
        assert!(
            (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3,
            "expected {b:?}, got {a:?}"
        );
    }

    #[test]
    fn relative_arc_and_radii_of_zero_fall_back_to_lines() {
        // Relative `a` moves the endpoint by the current point.
        let path = path_from_d("M 0 0 a 5 5 0 0 1 6 0");
        assert_eq!(path.subpaths()[0].last().copied(), Some((6.0, 0.0)));
        // A zero radius degenerates to a straight line.
        let degenerate = path_from_d("M 0 0 A 0 0 0 0 1 10 10");
        assert_eq!(degenerate.subpaths(), vec![vec![(0.0, 0.0), (10.0, 10.0)]]);
    }

    #[test]
    fn path_number_lexer_handles_exponents_commas_and_fractions() {
        let path = path_from_d("M1e2,1.5 L2E1,-.25 z");
        assert_eq!(path.subpaths(), vec![vec![(100.0, 1.5), (20.0, -0.25)]]);
    }

    #[test]
    fn parse_view_box_defaults_missing_components_to_zero() {
        assert_eq!(parse_view_box("0 0 100 50"), (0.0, 0.0, 100.0, 50.0));
        assert_eq!(parse_view_box("10,20,30,40"), (10.0, 20.0, 30.0, 40.0));
        assert_eq!(parse_view_box("5 6"), (5.0, 6.0, 0.0, 0.0));
        assert_eq!(parse_view_box(""), (0.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn parse_svg_fill_resolves_hex_named_and_function_colors() {
        let black = Color::default();
        assert_eq!(
            parse_svg_fill("#ff0000", ColorScheme::Light, black),
            Some(Color(255, 0, 0, 255))
        );
        assert_eq!(
            parse_svg_fill("red", ColorScheme::Light, black),
            Some(Color(255, 0, 0, 255))
        );
        assert_eq!(
            parse_svg_fill("rgb(0, 255, 0)", ColorScheme::Light, black),
            Some(Color(0, 255, 0, 255))
        );
    }

    #[test]
    fn parse_svg_fill_resolves_current_color_and_transparency() {
        let text_color = Color(1, 2, 3, 255);
        assert_eq!(
            parse_svg_fill("currentColor", ColorScheme::Light, text_color),
            Some(text_color)
        );
        assert_eq!(
            parse_svg_fill("none", ColorScheme::Light, text_color),
            Some(Color(0, 0, 0, 0))
        );
        assert_eq!(
            parse_svg_fill("transparent", ColorScheme::Light, text_color),
            Some(Color(0, 0, 0, 0))
        );
        assert_eq!(
            parse_svg_fill("gibberish", ColorScheme::Light, text_color),
            None
        );
    }

    #[test]
    fn resolve_shape_fill_prefers_attribute_over_style() {
        let el = element("path", &[("fill", "#ff0000"), ("style", "fill:#00ff00")]);
        assert_eq!(
            resolve_shape_fill(&el, ColorScheme::Light, Color::default()),
            Color(255, 0, 0, 255)
        );
        let style_only = element("path", &[("style", "fill: rgb(0, 0, 255)")]);
        assert_eq!(
            resolve_shape_fill(&style_only, ColorScheme::Light, Color::default()),
            Color(0, 0, 255, 255)
        );
        let inheriting = element("path", &[("fill", "currentColor")]);
        assert_eq!(
            resolve_shape_fill(&inheriting, ColorScheme::Light, Color(9, 8, 7, 255)),
            Color(9, 8, 7, 255)
        );
    }

    #[test]
    fn resolve_fill_rule_supports_attribute_and_style() {
        assert!(matches!(
            resolve_fill_rule(&element("path", &[])),
            FillRule::NonZero
        ));
        assert!(matches!(
            resolve_fill_rule(&element("path", &[("fill-rule", "evenodd")])),
            FillRule::EvenOdd
        ));
        assert!(matches!(
            resolve_fill_rule(&element("path", &[("style", "fill-rule:evenodd")])),
            FillRule::EvenOdd
        ));
    }

    #[test]
    fn resolve_stroke_defaults_to_none_without_any_declared_stroke() {
        assert_eq!(
            resolve_stroke(
                &element("path", &[]),
                ColorScheme::Light,
                Color::default(),
                &HashMap::new(),
                None
            ),
            None
        );
        // stroke-width on its own does not invent a stroke.
        assert_eq!(
            resolve_stroke(
                &element("path", &[("stroke-width", "2")]),
                ColorScheme::Light,
                Color::default(),
                &HashMap::new(),
                None
            ),
            None
        );
    }

    #[test]
    fn resolve_stroke_honors_declared_parameters_including_style() {
        let el = element(
            "line",
            &[
                ("stroke", "#ff0000"),
                ("stroke-width", "3"),
                ("stroke-linecap", "round"),
                ("stroke-linejoin", "bevel"),
                ("stroke-miterlimit", "8"),
                ("stroke-dasharray", "4 2"),
                ("stroke-dashoffset", "1"),
            ],
        );
        let stroke = resolve_stroke(
            &el,
            ColorScheme::Light,
            Color::default(),
            &HashMap::new(),
            None,
        )
        .expect("stroke");
        assert_eq!(stroke.color, Color(255, 0, 0, 255));
        assert_eq!(stroke.width, 3.0);
        assert!(matches!(stroke.cap, StrokeCap::Round));
        assert!(matches!(stroke.join, StrokeJoin::Bevel));
        assert_eq!(stroke.miter_limit, 8.0);
        assert_eq!(stroke.dash, Some(vec![4.0, 2.0]));
        assert_eq!(stroke.dash_offset, 1.0);

        // A style declaration drives every stroke parameter too.
        let style = element(
            "path",
            &[(
                "style",
                "stroke:#00ff00;stroke-width:5;stroke-linecap:square;\
                 stroke-linejoin:round;stroke-dasharray:none",
            )],
        );
        let stroke = resolve_stroke(
            &style,
            ColorScheme::Light,
            Color::default(),
            &HashMap::new(),
            None,
        )
        .expect("stroke");
        assert_eq!(stroke.color, Color(0, 255, 0, 255));
        assert!(matches!(stroke.cap, StrokeCap::Square));
        assert!(matches!(stroke.join, StrokeJoin::Round));
        assert_eq!(stroke.dash, None);
    }

    #[test]
    fn resolve_stroke_none_and_namewidth_zero_suppress() {
        assert_eq!(
            resolve_stroke(
                &element("path", &[("stroke", "none")]),
                ColorScheme::Light,
                Color::default(),
                &HashMap::new(),
                None
            ),
            None
        );
        assert_eq!(
            resolve_stroke(
                &element("path", &[("stroke", "red"), ("stroke-width", "0")]),
                ColorScheme::Light,
                Color::default(),
                &HashMap::new(),
                None
            ),
            None
        );
        assert_eq!(
            resolve_stroke(
                &element("path", &[("stroke", "transparent")]),
                ColorScheme::Light,
                Color::default(),
                &HashMap::new(),
                None
            ),
            None
        );
    }

    #[test]
    fn resolve_stroke_inherits_through_containers_until_overridden() {
        let parent = element(
            "g",
            &[
                ("stroke", "#00ff00"),
                ("stroke-width", "2"),
                ("stroke-linecap", "round"),
            ],
        );
        let inherited = resolve_stroke(
            &parent,
            ColorScheme::Light,
            Color::default(),
            &HashMap::new(),
            None,
        )
        .expect("stroke");
        // A child that declares nothing keeps the inherited stroke...
        let child = resolve_stroke(
            &element("rect", &[]),
            ColorScheme::Light,
            Color::default(),
            &HashMap::new(),
            Some(&inherited),
        )
        .expect("inherits");
        assert_eq!(child.color, Color(0, 255, 0, 255));
        assert_eq!(child.width, 2.0);
        assert!(matches!(child.cap, StrokeCap::Round));
        // ...one that sets stroke:none drops it entirely.
        assert_eq!(
            resolve_stroke(
                &element("rect", &[("stroke", "none")]),
                ColorScheme::Light,
                Color::default(),
                &HashMap::new(),
                Some(&inherited)
            ),
            None
        );
        // currentColor resolves against the passed text color.
        let cc = resolve_stroke(
            &element("line", &[("stroke", "currentColor")]),
            ColorScheme::Light,
            Color(9, 8, 7, 255),
            &HashMap::new(),
            None,
        )
        .expect("currentColor");
        assert_eq!(cc.color, Color(9, 8, 7, 255));
    }

    #[test]
    fn parse_dasharray_handles_none_lists_and_odd_patterns() {
        assert_eq!(parse_dasharray("none"), None);
        assert_eq!(parse_dasharray(""), None);
        assert_eq!(parse_dasharray("0 0"), None);
        assert_eq!(parse_dasharray("4,2"), Some(vec![4.0, 2.0]));
        assert_eq!(parse_dasharray("4 2 1"), Some(vec![4.0, 2.0, 1.0]));
        assert_eq!(parse_dasharray("2,-1"), Some(vec![2.0, 0.0]));
        assert_eq!(parse_dasharray("bogus"), None);
    }

    #[test]
    fn viewbox_meet_centers_content_and_preserves_aspect() {
        // viewBox 10x10 in a 20x10 viewport: scale 1, shifted right by 5.
        let t = viewbox_meet_transform((0.0, 0.0, 10.0, 10.0), 20.0, 10.0);
        assert!((t.m11 - 1.0).abs() < 1e-6);
        assert!((t.dx - 5.0).abs() < 1e-6);
        assert!((t.dy - 0.0).abs() < 1e-6);
        // A non-zero viewBox origin is mapped through the centered layout.
        let t = viewbox_meet_transform((2.0, 3.0, 10.0, 10.0), 20.0, 10.0);
        assert!((t.dx - 3.0).abs() < 1e-6);
        assert!((t.dy - -3.0).abs() < 1e-6);
        assert!(viewbox_meet_transform((0.0, 0.0, 0.0, 10.0), 5.0, 5.0).m11 == 1.0);
    }

    #[test]
    fn emit_commands_letterboxes_and_honors_per_shape_fill_rule() {
        let shape = SvgShape {
            path: path_from_d("M0 0 L10 0 L10 10 Z"),
            fill: Brush::Solid(Color(255, 0, 0, 255)),
            rule: FillRule::EvenOdd,
            stroke: None,
            clip_path: None,
        };
        let mut buf = Vec::new();
        emit_commands(
            &mut buf,
            (0.0, 0.0, 20.0, 10.0),
            false,
            (0.0, 0.0, 10.0, 10.0),
            std::slice::from_ref(&shape),
            0.5,
        );
        assert_eq!(buf.len(), 3);
        let DrawCommand::PushTransform { transform } = &buf[0] else {
            panic!("expected transform");
        };
        assert!((transform.m11 - 1.0).abs() < 1e-6);
        assert!((transform.dx - 5.0).abs() < 1e-6);
        let DrawCommand::Fill { paint, rule, .. } = &buf[1] else {
            panic!("expected fill");
        };
        assert!(matches!(*rule, FillRule::EvenOdd));
        assert!((paint.opacity - 0.5).abs() < 1e-6);
        assert!(matches!(&buf[2], DrawCommand::PopTransform));
    }

    #[test]
    fn emit_commands_paints_inline_at_absolute_position() {
        let shape = SvgShape {
            path: path_from_d("M0 0 L10 0 L10 10 Z"),
            fill: Brush::Solid(Color(255, 0, 0, 255)),
            rule: FillRule::NonZero,
            stroke: None,
            clip_path: None,
        };
        let mut buf = Vec::new();
        emit_commands(
            &mut buf,
            (30.0, 40.0, 10.0, 10.0),
            true,
            (0.0, 0.0, 10.0, 10.0),
            std::slice::from_ref(&shape),
            1.0,
        );
        let DrawCommand::PushTransform { transform } = &buf[0] else {
            panic!("expected transform");
        };
        assert!((transform.dx - 30.0).abs() < 1e-6);
        assert!((transform.dy - 40.0).abs() < 1e-6);
    }

    #[test]
    fn emit_commands_paints_nothing_for_degenerate_box() {
        let shape = SvgShape {
            path: path_from_d("M0 0 L10 0 L10 10 Z"),
            fill: Brush::Solid(Color(255, 0, 0, 255)),
            rule: FillRule::NonZero,
            stroke: None,
            clip_path: None,
        };
        let mut buf = Vec::new();
        emit_commands(
            &mut buf,
            (0.0, 0.0, 0.0, 10.0),
            false,
            (0.0, 0.0, 10.0, 10.0),
            std::slice::from_ref(&shape),
            1.0,
        );
        assert!(buf.is_empty());
    }

    #[test]
    fn collect_svg_gathers_paths_with_fill_rule_and_opacity() {
        let svg = TreeNode::new(element("svg", &[("viewBox", "0 0 10 10")]));
        TreeNode::add_child(
            &svg,
            TreeNode::new(element(
                "path",
                &[("d", "M0 0 L10 0 L10 10 Z"), ("fill", "currentColor")],
            )),
        );
        TreeNode::add_child(
            &svg,
            TreeNode::new(element(
                "path",
                &[
                    ("d", "M1 1 L5 1 L5 5 Z"),
                    ("fill", "#00ff00"),
                    ("fill-rule", "evenodd"),
                    ("fill-opacity", "0.5"),
                ],
            )),
        );
        TreeNode::add_child(&svg, TreeNode::new(element("rect", &[("x", "1")])));

        let (snapshot, _refs) = DomSnapshot::from_tree(&svg);
        let content = collect_svg(&snapshot, 0, ColorScheme::Light, Color(1, 2, 3, 255)).unwrap();

        assert_eq!(content.view_box, (0.0, 0.0, 10.0, 10.0));
        assert_eq!(content.shapes.len(), 2);
        assert_eq!(content.shapes[0].fill, Brush::Solid(Color(1, 2, 3, 255)));
        assert!(matches!(content.shapes[0].rule, FillRule::NonZero));
        assert_eq!(content.shapes[1].fill, Brush::Solid(Color(0, 255, 0, 128)));
        assert!(matches!(content.shapes[1].rule, FillRule::EvenOdd));
    }

    #[test]
    fn collect_svg_returns_none_without_usable_viewbox_or_paths() {
        let no_viewbox = TreeNode::new(element("svg", &[]));
        let (snapshot, _) = DomSnapshot::from_tree(&no_viewbox);
        assert!(collect_svg(&snapshot, 0, ColorScheme::Light, Color::default()).is_none());

        let empty = TreeNode::new(element("svg", &[("viewBox", "0 0 10 10")]));
        let (snapshot, _) = DomSnapshot::from_tree(&empty);
        assert!(collect_svg(&snapshot, 0, ColorScheme::Light, Color::default()).is_none());
    }

    #[test]
    fn collect_svg_walks_groups_applies_transforms_and_shapes() {
        let svg = TreeNode::new(element("svg", &[("viewBox", "0 0 12 8")]));
        let g = TreeNode::new(element(
            "g",
            &[("transform", "translate(2 1)"), ("opacity", "0.5")],
        ));
        TreeNode::add_child(&svg, Rc::clone(&g));
        TreeNode::add_child(
            &g,
            TreeNode::new(element(
                "path",
                &[("d", "M1 1 L5 1 L3 5 Z"), ("fill", "#ff0000")],
            )),
        );
        TreeNode::add_child(
            &g,
            TreeNode::new(element(
                "rect",
                &[
                    ("x", "0"),
                    ("y", "6"),
                    ("width", "3"),
                    ("height", "2"),
                    ("fill", "#00ff00"),
                ],
            )),
        );
        TreeNode::add_child(&svg, TreeNode::new(element("circle", &[("r", "2")])));

        let (snapshot, _refs) = DomSnapshot::from_tree(&svg);
        let content = collect_svg(&snapshot, 0, ColorScheme::Light, Color(9, 8, 7, 255)).unwrap();

        assert_eq!(content.shapes.len(), 3);
        // Path vertices shifted by translate(2 1).
        assert_eq!(
            content.shapes[0].path.subpaths()[0][..3],
            [(3.0, 2.0), (7.0, 2.0), (5.0, 6.0)]
        );
        // 0.5 group opacity folds into the shape alpha.
        assert_eq!(content.shapes[0].fill, Brush::Solid(Color(255, 0, 0, 128)));
        // Rect corners translated into viewBox space.
        assert_eq!(
            content.shapes[1].path.subpaths()[0][..3],
            [(2.0, 7.0), (5.0, 7.0), (5.0, 9.0)]
        );
        // circle with no cx/cy defaults to the origin.
        assert_eq!(
            content.shapes[2].path.bounding_box().unwrap(),
            crate::engine::renderer_model::Rect::new(-2.0, -2.0, 4.0, 4.0)
        );
    }

    #[test]
    fn collect_svg_supports_polygon_polyline_line_and_rounded_rect() {
        let svg = TreeNode::new(element("svg", &[("viewBox", "0 0 20 20")]));
        TreeNode::add_child(
            &svg,
            TreeNode::new(element(
                "rect",
                &[
                    ("x", "1"),
                    ("y", "2"),
                    ("width", "8"),
                    ("height", "6"),
                    ("rx", "2"),
                    ("ry", "2"),
                ],
            )),
        );
        TreeNode::add_child(
            &svg,
            TreeNode::new(element("polygon", &[("points", "0,0 10,0 10,10")])),
        );
        TreeNode::add_child(
            &svg,
            TreeNode::new(element("polyline", &[("points", "0,0 5,5 10,0")])),
        );
        TreeNode::add_child(
            &svg,
            TreeNode::new(element(
                "line",
                &[("x1", "0"), ("y1", "0"), ("x2", "10"), ("y2", "0")],
            )),
        );

        let (snapshot, _refs) = DomSnapshot::from_tree(&svg);
        let content = collect_svg(&snapshot, 0, ColorScheme::Light, Color::default()).unwrap();
        assert_eq!(content.shapes.len(), 4);
        // Rounded rect keeps its corners on the radius-2 arcs. The top-right arc is
        // centered at the corner inset by rx/ry: (7, 4) with radius 2.
        let arc_ok = content.shapes[0].path.subpaths()[0].iter().any(|&(x, y)| {
            ((x - 7.0).powi(2) + (y - 4.0).powi(2) - 4.0).abs() < 0.1 && x > 7.5 && y < 3.0
        });
        assert!(
            arc_ok,
            "rounded rect must keep arc points on the corner circle"
        );
        // Polygon closes implicitly, so its vertices are a closed ring.
        assert_eq!(
            content.shapes[1].path.subpaths()[0][..3],
            [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]
        );
        assert_eq!(
            content.shapes[2].path.subpaths()[0][..3],
            [(0.0, 0.0), (5.0, 5.0), (10.0, 0.0)]
        );
        assert_eq!(
            content.shapes[3].path.subpaths()[0][..2],
            [(0.0, 0.0), (10.0, 0.0)]
        );
    }

    #[test]
    fn collect_svg_inherits_stroke_and_fill_through_containers() {
        let svg = TreeNode::new(element("svg", &[("viewBox", "0 0 10 10")]));
        let g = TreeNode::new(element(
            "g",
            &[
                ("fill", "#0000ff"),
                ("stroke", "#00ff00"),
                ("stroke-width", "2"),
                ("stroke-linecap", "round"),
            ],
        ));
        TreeNode::add_child(&svg, Rc::clone(&g));
        let inherits = TreeNode::new(element(
            "line",
            &[("x1", "0"), ("y1", "0"), ("x2", "5"), ("y2", "0")],
        ));
        let overrides = TreeNode::new(element(
            "rect",
            &[
                ("x", "0"),
                ("y", "1"),
                ("width", "3"),
                ("height", "3"),
                ("stroke", "none"),
            ],
        ));
        TreeNode::add_child(&g, inherits);
        TreeNode::add_child(&g, overrides);

        let (snapshot, _refs) = DomSnapshot::from_tree(&svg);
        let content = collect_svg(&snapshot, 0, ColorScheme::Light, Color::default()).unwrap();
        assert_eq!(content.shapes.len(), 2);

        let inherited_stroke = content.shapes[0].stroke.as_ref().expect("stroke inherited");
        assert_eq!(inherited_stroke.color, Color(0, 255, 0, 255));
        assert_eq!(inherited_stroke.width, 2.0);
        assert!(matches!(inherited_stroke.cap, StrokeCap::Round));
        assert_eq!(content.shapes[0].fill, Brush::Solid(Color(0, 0, 255, 255)));

        assert_eq!(
            content.shapes[1].stroke, None,
            "stroke:none cancels inheritance"
        );
        assert_eq!(content.shapes[1].fill, Brush::Solid(Color(0, 0, 255, 255)));
    }

    #[test]
    fn emit_commands_paints_a_stroke_outline_after_the_fill() {
        let shape = SvgShape {
            path: path_from_d("M0 5 L10 5"),
            fill: Brush::Solid(Color(0, 0, 0, 0)),
            rule: FillRule::NonZero,
            stroke: Some(SvgStroke {
                color: Color(255, 0, 0, 255),
                gradient: None,
                width: 2.0,
                cap: StrokeCap::Butt,
                join: StrokeJoin::Miter,
                miter_limit: 4.0,
                dash: None,
                dash_offset: 0.0,
            }),
            clip_path: None,
        };
        let mut buf = Vec::new();
        emit_commands(
            &mut buf,
            (0.0, 0.0, 10.0, 10.0),
            false,
            (0.0, 0.0, 10.0, 10.0),
            std::slice::from_ref(&shape),
            1.0,
        );
        // Push + shape fill + stroke fill + pop.
        assert_eq!(buf.len(), 4);
        let DrawCommand::Fill {
            paint: fill_paint, ..
        } = &buf[1]
        else {
            panic!("expected shape fill");
        };
        let fill_alpha = match &fill_paint.brush {
            Brush::Solid(color) => color.3,
            _ => panic!("solid"),
        };
        assert_eq!(fill_alpha, 0);
        let DrawCommand::Fill {
            path: stroke_path,
            paint,
            rule,
        } = &buf[2]
        else {
            panic!("expected stroke fill");
        };
        assert!(matches!(rule, FillRule::NonZero));
        let Brush::Solid(color) = &paint.brush else {
            panic!("solid stroke");
        };
        assert_eq!(*color, Color(255, 0, 0, 255));
        // The expanded outline hugs the horizontal line: width-2 stroke spans
        // y in [-1, 1] about the line at y=5.
        let bb = stroke_path.bounding_box().unwrap();
        assert!((bb.y - 4.0).abs() < 1e-3);
        assert!((bb.height - 2.0).abs() < 1e-3);
        assert!((bb.x - 0.0).abs() < 1e-3);
        assert!((bb.width - 10.0).abs() < 1e-3);
    }

    #[test]
    fn parse_transform_composes_functions_left_to_right() {
        let t = parse_transform("translate(10, 20) scale(2)").expect("parses");
        assert_eq!(t.apply(1.0, 1.0), (12.0, 22.0));
        let rotated = parse_transform("rotate(90)").expect("parses");
        assert!((rotated.apply(1.0, 0.0).0).abs() < 1e-5);
        assert!((rotated.apply(1.0, 0.0).1 - 1.0).abs() < 1e-5);
        assert!(parse_transform("bogus(1)").is_err());
    }

    /// Rasterizes `DrawCommand`s with a transform stack, mirroring how the GPU
    /// renderer lays out `PushTransform`/`Fill`/`PopTransform`.
    fn rasterize_commands(commands: &[DrawCommand], width: u32, height: u32) -> Vec<u8> {
        let mut rgba = vec![0u8; width as usize * height as usize * 4];
        let mut stack = vec![AffineTransform::identity()];
        let mut clip_stack: Vec<(Path, FillRule)> = Vec::new();
        for command in commands {
            match command {
                DrawCommand::PushTransform { transform } => {
                    let next = stack.last().unwrap().then(transform);
                    stack.push(next);
                }
                DrawCommand::PopTransform => {
                    stack.pop();
                }
                DrawCommand::PushClip { path, rule } => {
                    clip_stack.push((path.clone(), *rule));
                }
                DrawCommand::PopClip => {
                    clip_stack.pop();
                }
                DrawCommand::Fill { path, paint, rule } => {
                    let Brush::Solid(color) = &paint.brush else {
                        continue;
                    };
                    let fill = with_alpha(*color, paint.opacity);
                    let transformed = transform_path(path, stack.last().unwrap());
                    let clip_mask = if !clip_stack.is_empty() {
                        let mut combined = vec![255u8; (width as usize) * (height as usize) * 4];
                        for (clip_path, clip_rule) in &clip_stack {
                            let mut tmp = vec![0u8; (width as usize) * (height as usize) * 4];
                            let clip_transformed = transform_path(clip_path, stack.last().unwrap());
                            super::rasterize::fill_path_into(
                                &mut tmp,
                                width,
                                height,
                                &clip_transformed,
                                Color(255, 255, 255, 255),
                                *clip_rule,
                                None,
                            );
                            // Intersect: combined alpha *= tmp alpha.
                            for i in (3..combined.len()).step_by(4) {
                                combined[i] = ((combined[i] as u16 * tmp[i] as u16) / 255) as u8;
                            }
                        }
                        Some(combined)
                    } else {
                        None
                    };
                    super::rasterize::fill_path_into(
                        &mut rgba,
                        width,
                        height,
                        &transformed,
                        fill,
                        *rule,
                        clip_mask.as_deref(),
                    );
                }
                _ => {}
            }
        }
        rgba
    }

    #[test]
    fn parse_svg_fill_url_reference_is_handled_without_color_pipeline() {
        // `url(#id)` names a gradient, not a color: it must not trip the CSS
        // color pipeline's error log and resolves to `None` here.
        assert_eq!(
            parse_svg_fill(
                "url(#blinearGradient971)",
                ColorScheme::Light,
                Color::default()
            ),
            None
        );
        assert_eq!(
            parse_svg_url("url(#gradient-1)"),
            Some("gradient-1".to_owned())
        );
        assert_eq!(
            parse_svg_url("url(\"quoted-id\")"),
            Some("quoted-id".to_owned())
        );
        assert_eq!(parse_svg_url("#ff0000"), None);
        assert_eq!(parse_svg_url("red"), None);
    }

    #[test]
    fn collect_svg_resolves_url_fill_to_gradient_brushes() {
        let svg = TreeNode::new(element("svg", &[("viewBox", "0 0 10 10")]));
        let defs = TreeNode::new(element("defs", &[]));
        TreeNode::add_child(&svg, Rc::clone(&defs));

        let linear = TreeNode::new(element(
            "linearGradient",
            &[
                ("id", "g1"),
                ("x1", "0"),
                ("y1", "0"),
                ("x2", "1"),
                ("y2", "0"),
                ("spreadMethod", "reflect"),
            ],
        ));
        TreeNode::add_child(
            &linear,
            TreeNode::new(element(
                "stop",
                &[("offset", "0"), ("stop-color", "#ff0000")],
            )),
        );
        TreeNode::add_child(
            &linear,
            TreeNode::new(element(
                "stop",
                &[
                    ("offset", "1"),
                    ("style", "stop-color:#0000ff;stop-opacity:0.5"),
                ],
            )),
        );
        TreeNode::add_child(&defs, linear);

        let radial = TreeNode::new(element(
            "radialGradient",
            &[
                ("id", "g2"),
                ("gradientUnits", "userSpaceOnUse"),
                ("cx", "5"),
                ("cy", "5"),
                ("r", "5"),
                ("fx", "4"),
                ("fy", "4"),
            ],
        ));
        TreeNode::add_child(
            &radial,
            TreeNode::new(element("stop", &[("offset", "0"), ("stop-color", "green")])),
        );
        TreeNode::add_child(
            &radial,
            TreeNode::new(element(
                "stop",
                &[("offset", "100%"), ("stop-color", "blue")],
            )),
        );
        TreeNode::add_child(&defs, radial);

        TreeNode::add_child(
            &svg,
            TreeNode::new(element(
                "rect",
                &[
                    ("x", "1"),
                    ("y", "1"),
                    ("width", "8"),
                    ("height", "8"),
                    ("fill", "url(#g1)"),
                ],
            )),
        );
        TreeNode::add_child(
            &svg,
            TreeNode::new(element(
                "circle",
                &[("cx", "5"), ("cy", "5"), ("r", "4"), ("fill", "url(#g2)")],
            )),
        );

        let (snapshot, _refs) = DomSnapshot::from_tree(&svg);
        let content = collect_svg(&snapshot, 0, ColorScheme::Light, Color::default()).unwrap();
        assert_eq!(content.shapes.len(), 2);

        let Brush::Gradient(g1) = &content.shapes[0].fill else {
            panic!("rect should be a gradient fill");
        };
        assert!(matches!(
            g1.kind,
            GradientKind::SvgLinear {
                x1: 0.0,
                y1: 0.0,
                x2: 1.0,
                y2: 0.0,
                user_space: false
            }
        ));
        assert_eq!(g1.spread, GradientSpread::Reflect);
        assert_eq!(g1.stops.len(), 2);
        assert_eq!(g1.stops[0].color, Color(255, 0, 0, 255));
        assert_eq!(g1.stops[0].position, Some(0.0));
        assert_eq!(g1.stops[1].color, Color(0, 0, 255, 128));
        assert_eq!(g1.stops[1].position, Some(1.0));

        let Brush::Gradient(g2) = &content.shapes[1].fill else {
            panic!("circle should be a gradient fill");
        };
        assert!(matches!(
            g2.kind,
            GradientKind::SvgRadial {
                cx: 5.0,
                cy: 5.0,
                r: 5.0,
                fx: 4.0,
                fy: 4.0,
                fr: 0.0,
                user_space: true
            }
        ));
        assert_eq!(g2.spread, GradientSpread::Pad);
        assert_eq!(g2.stops.len(), 2);
        assert_eq!(g2.stops[0].color, Color(0, 128, 0, 255));
        assert_eq!(g2.stops[1].position, Some(1.0));
    }

    #[test]
    fn collect_svg_unresolved_url_fill_paints_nothing() {
        let svg = TreeNode::new(element("svg", &[("viewBox", "0 0 10 10")]));
        TreeNode::add_child(
            &svg,
            TreeNode::new(element(
                "rect",
                &[
                    ("x", "1"),
                    ("y", "1"),
                    ("width", "8"),
                    ("height", "8"),
                    ("fill", "url(#missing)"),
                ],
            )),
        );
        let (snapshot, _refs) = DomSnapshot::from_tree(&svg);
        let content = collect_svg(&snapshot, 0, ColorScheme::Light, Color::default()).unwrap();
        assert_eq!(content.shapes[0].fill, Brush::Solid(Color(0, 0, 0, 0)));
    }

    #[test]
    fn resolve_stroke_resolves_url_paint_servers() {
        let mut gradients = HashMap::new();
        gradients.insert(
            "g".to_owned(),
            Gradient {
                kind: GradientKind::SvgLinear {
                    x1: 0.0,
                    y1: 0.0,
                    x2: 1.0,
                    y2: 0.0,
                    user_space: false,
                },
                stops: vec![ColorStop {
                    color: Color(255, 0, 0, 255),
                    position: Some(0.0),
                }],
                spread: GradientSpread::Pad,
            },
        );
        let stroke = resolve_stroke(
            &element("path", &[("stroke", "url(#g)"), ("stroke-width", "2")]),
            ColorScheme::Light,
            Color::default(),
            &gradients,
            None,
        )
        .expect("gradient stroke");
        assert!(stroke.gradient.is_some());
        assert_eq!(stroke.width, 2.0);
        // An unresolved reference paints nothing.
        assert_eq!(
            resolve_stroke(
                &element("path", &[("stroke", "url(#nope)")]),
                ColorScheme::Light,
                Color::default(),
                &HashMap::new(),
                None,
            ),
            None
        );
    }

    #[test]
    fn emit_commands_paints_gradient_brushes_for_fill_and_stroke() {
        let shape = SvgShape {
            path: path_from_d("M0 0 L10 0 L10 10 Z"),
            fill: Brush::Gradient(Gradient {
                kind: GradientKind::SvgLinear {
                    x1: 0.0,
                    y1: 0.0,
                    x2: 1.0,
                    y2: 0.0,
                    user_space: false,
                },
                stops: vec![
                    ColorStop {
                        color: Color(255, 0, 0, 255),
                        position: Some(0.0),
                    },
                    ColorStop {
                        color: Color(0, 0, 255, 255),
                        position: Some(1.0),
                    },
                ],
                spread: GradientSpread::Pad,
            }),
            rule: FillRule::NonZero,
            stroke: Some(SvgStroke {
                color: Color(0, 0, 0, 255),
                gradient: Some(Gradient {
                    kind: GradientKind::SvgLinear {
                        x1: 0.0,
                        y1: 0.0,
                        x2: 1.0,
                        y2: 0.0,
                        user_space: false,
                    },
                    stops: vec![ColorStop {
                        color: Color(0, 255, 0, 255),
                        position: Some(0.0),
                    }],
                    spread: GradientSpread::Pad,
                }),
                width: 2.0,
                cap: StrokeCap::Butt,
                join: StrokeJoin::Miter,
                miter_limit: 4.0,
                dash: None,
                dash_offset: 0.0,
            }),
            clip_path: None,
        };
        let mut buf = Vec::new();
        emit_commands(
            &mut buf,
            (0.0, 0.0, 10.0, 10.0),
            false,
            (0.0, 0.0, 10.0, 10.0),
            std::slice::from_ref(&shape),
            1.0,
        );
        let DrawCommand::Fill { paint, .. } = &buf[1] else {
            panic!("expected fill");
        };
        assert!(matches!(paint.brush, Brush::Gradient(_)));
        let DrawCommand::Fill {
            paint: stroke_paint,
            ..
        } = &buf[2]
        else {
            panic!("expected stroke fill");
        };
        assert!(matches!(stroke_paint.brush, Brush::Gradient(_)));
    }

    #[test]
    fn inline_content_rasterizes_identically_to_standalone_document() {
        // The same geometry as serialized SVG (rasterizer) and as inline SVG
        // content (`collect_svg` → `emit_commands` → software fill) must yield
        // byte-identical pixels: same transforms, opacity folding, and fill
        // rule at both layers.
        let markup = r##"<svg width="24" height="12" viewBox="0 0 24 12">
            <g transform="translate(2 2) skewX(10)" opacity="0.8">
              <path d="M2 2 L22 6 L2 10 Z M8 4 L16 4 L12 6 Z" fill="#ff0000" fill-rule="evenodd"/>
              <rect x="0" y="0" width="4" height="12" fill="#00ff00" fill-opacity="0.5"/>
              <circle cx="20" cy="3" r="2" fill="#0000ff"/>
            </g>
            <g transform="translate(4 6)">
              <line x1="0" y1="2" x2="18" y2="2" stroke="#ffffff" stroke-width="2" stroke-linecap="round" stroke-dasharray="4 2" fill="none"/>
              <rect x="0" y="0" width="6" height="4" fill="none" stroke="#00ffff" stroke-width="1"/>
            </g>
          </svg>"##;
        let standalone = super::rasterize::rasterize(markup, Some((24, 12))).expect("rasterizes");

        let svg = TreeNode::new(element(
            "svg",
            &[("width", "24"), ("height", "12"), ("viewBox", "0 0 24 12")],
        ));
        let g = TreeNode::new(element(
            "g",
            &[
                ("transform", "translate(2 2) skewX(10)"),
                ("opacity", "0.8"),
            ],
        ));
        TreeNode::add_child(&svg, Rc::clone(&g));
        TreeNode::add_child(
            &g,
            TreeNode::new(element(
                "path",
                &[
                    ("d", "M2 2 L22 6 L2 10 Z M8 4 L16 4 L12 6 Z"),
                    ("fill", "#ff0000"),
                    ("fill-rule", "evenodd"),
                ],
            )),
        );
        TreeNode::add_child(
            &g,
            TreeNode::new(element(
                "rect",
                &[
                    ("x", "0"),
                    ("y", "0"),
                    ("width", "4"),
                    ("height", "12"),
                    ("fill", "#00ff00"),
                    ("fill-opacity", "0.5"),
                ],
            )),
        );
        TreeNode::add_child(
            &g,
            TreeNode::new(element(
                "circle",
                &[("cx", "20"), ("cy", "3"), ("r", "2"), ("fill", "#0000ff")],
            )),
        );
        let g2 = TreeNode::new(element("g", &[("transform", "translate(4 6)")]));
        TreeNode::add_child(&svg, Rc::clone(&g2));
        TreeNode::add_child(
            &g2,
            TreeNode::new(element(
                "line",
                &[
                    ("x1", "0"),
                    ("y1", "2"),
                    ("x2", "18"),
                    ("y2", "2"),
                    ("stroke", "#ffffff"),
                    ("stroke-width", "2"),
                    ("stroke-linecap", "round"),
                    ("stroke-dasharray", "4 2"),
                    ("fill", "none"),
                ],
            )),
        );
        TreeNode::add_child(
            &g2,
            TreeNode::new(element(
                "rect",
                &[
                    ("x", "0"),
                    ("y", "0"),
                    ("width", "6"),
                    ("height", "4"),
                    ("fill", "none"),
                    ("stroke", "#00ffff"),
                    ("stroke-width", "1"),
                ],
            )),
        );

        let (snapshot, _refs) = DomSnapshot::from_tree(&svg);
        let content = collect_svg(&snapshot, 0, ColorScheme::Light, Color::default())
            .expect("collects inline SVG");
        let mut commands = Vec::new();
        emit_commands(
            &mut commands,
            (0.0, 0.0, 24.0, 12.0),
            false,
            content.view_box,
            &content.shapes,
            1.0,
        );
        let inline_pixels = rasterize_commands(&commands, 24, 12);

        assert_eq!(
            inline_pixels, standalone.rgba,
            "inline and standalone SVG rasterization differ"
        );
        // Sanity: some pixels are actually painted.
        assert!(
            inline_pixels.chunks(4).any(|p| p[3] != 0),
            "expected non-transparent pixels"
        );
    }

    /// Converts a parsed [`XmlElement`] tree into an [`HtmlNodeType`] tree so
    /// the inline SVG pipeline can process real SVG documents.
    fn xml_to_html_tree(
        node: &crate::engine::tree::NodeRef<crate::engine::html::xml::XmlElement>,
    ) -> crate::engine::tree::NodeRef<crate::engine::html::HtmlNodeType> {
        let el = node.borrow();
        let html_value = crate::engine::html::HtmlNodeType::Element {
            tag_name: el.value.name.clone(),
            attributes: el
                .value
                .attributes
                .iter()
                .map(
                    |(name, value)| crate::engine::html::parser::OwnedAttribute {
                        name: name.clone(),
                        value: value.clone(),
                    },
                )
                .collect(),
        };
        let html_node = TreeNode::new(html_value);
        for child in el.children() {
            TreeNode::add_child(&html_node, xml_to_html_tree(child));
        }
        html_node
    }

    /// Real-world icon SVGs must rasterize identically through the inline
    /// (`collect_svg` → `emit_commands`) and standalone (`rasterize_from_bytes`)
    /// pipelines.
    #[test]
    fn icon_svg_inline_matches_standalone() {
        fn run_icon_test(file_path: &str, target_size: (u32, u32)) {
            let svg_bytes = std::fs::read(file_path).expect("read icon svg");
            let markup = std::str::from_utf8(&svg_bytes).expect("valid UTF-8");

            // Standalone rasterization.
            let standalone = super::rasterize::rasterize(markup, Some(target_size))
                .expect("rasterizes standalone");

            // Inline pipeline: parse XML → HtmlNodeType tree → DomSnapshot →
            // collect_svg → emit_commands → software fill.
            let tree = crate::engine::html::xml::parse(markup).expect("parses SVG XML");
            let root = tree.root.borrow();
            assert_eq!(root.value.name, "svg");
            let html_root = xml_to_html_tree(&tree.root);
            let (snapshot, _refs) = DomSnapshot::from_tree(&html_root);

            let content = collect_svg(&snapshot, 0, ColorScheme::Light, Color::default())
                .expect("collects inline SVG");
            let mut commands = Vec::new();
            emit_commands(
                &mut commands,
                (0.0, 0.0, target_size.0 as f32, target_size.1 as f32),
                false,
                content.view_box,
                &content.shapes,
                1.0,
            );
            let inline_pixels = rasterize_commands(&commands, target_size.0, target_size.1);

            assert_eq!(
                inline_pixels, standalone.rgba,
                "inline and standalone SVG rasterization differ for {file_path}"
            );
            assert!(
                inline_pixels.chunks(4).any(|p| p[3] != 0),
                "expected non-transparent pixels for {file_path}"
            );
        }

        run_icon_test("resource/icons/audio_play.svg", (256, 256));
        run_icon_test("resource/icons/audio_stop.svg", (256, 256));
        run_icon_test("resource/icon.svg", (512, 512));
    }

    /// Truncated path data (a command letter with missing coordinates) must not
    /// panic; the incomplete command is skipped defensively.
    #[test]
    fn path_from_d_truncated_command_does_not_panic() {
        // `M` with no following coordinates: nothing to move to.
        let path = path_from_d("M");
        assert!(path.commands().is_empty());
        // `L` with no coordinates: nothing to draw.
        let path = path_from_d("M 0 0 L");
        assert_eq!(path.subpaths(), vec![vec![(0.0, 0.0)]]);
        // Truncated cubic: missing control/end points.
        let path = path_from_d("M 0 0 C 10 10");
        assert_eq!(path.subpaths(), vec![vec![(0.0, 0.0)]]);
    }

    /// An invalid `transform` attribute on a shape element must not crash; the
    /// element inherits the parent's transform unchanged.
    #[test]
    fn collect_svg_invalid_transform_is_ignored() {
        let svg = TreeNode::new(element("svg", &[("viewBox", "0 0 10 10")]));
        let g = TreeNode::new(element("g", &[("transform", "not-a-valid-transform")]));
        TreeNode::add_child(&svg, Rc::clone(&g));
        TreeNode::add_child(
            &g,
            TreeNode::new(element(
                "rect",
                &[
                    ("x", "0"),
                    ("y", "0"),
                    ("width", "5"),
                    ("height", "5"),
                    ("fill", "#ff0000"),
                ],
            )),
        );

        let (snapshot, _refs) = DomSnapshot::from_tree(&svg);
        let content = collect_svg(&snapshot, 0, ColorScheme::Light, Color::default()).unwrap();
        assert_eq!(content.shapes.len(), 1);
        // Invalid transform is ignored: rect stays at (0,0)×(5,5).
        let bb = content.shapes[0].path.bounding_box().unwrap();
        assert!((bb.x - 0.0).abs() < 1e-6);
        assert!((bb.y - 0.0).abs() < 1e-6);
        assert!((bb.width - 5.0).abs() < 1e-6);
        assert!((bb.height - 5.0).abs() < 1e-6);
    }

    /// A `<path>` without a `d` attribute produces no shape.
    #[test]
    fn element_shape_missing_d_attribute_returns_none() {
        let el = element("path", &[("fill", "#ff0000")]);
        assert!(crate::engine::svg::element_shape(&el).is_none());
    }

    /// Malformed SVG markup is rejected by the standalone rasterizer instead of
    /// crashing or producing garbage pixels.
    #[test]
    fn rasterize_malformed_svg_returns_error() {
        assert!(super::rasterize::rasterize("<svg><unclosed>", None).is_err());
        assert!(super::rasterize::rasterize("<notsvg></notsvg>", None).is_err());
        // Extremely large viewport is rejected, not allocated.
        assert!(
            super::rasterize::rasterize(r##"<svg width="1e300" height="10"></svg>"##, None)
                .is_err()
        );
    }

    /// `<use href="#id">` clones the referenced element's content at the use
    /// element's position.
    #[test]
    fn collect_svg_use_element_references_target() {
        let svg = TreeNode::new(element("svg", &[("viewBox", "0 0 10 10")]));
        let defs = TreeNode::new(element("defs", &[]));
        TreeNode::add_child(&svg, Rc::clone(&defs));
        let rect = TreeNode::new(element(
            "rect",
            &[
                ("id", "myRect"),
                ("x", "1"),
                ("y", "1"),
                ("width", "3"),
                ("height", "3"),
                ("fill", "#ff0000"),
            ],
        ));
        TreeNode::add_child(&defs, Rc::clone(&rect));
        TreeNode::add_child(
            &svg,
            TreeNode::new(element(
                "use",
                &[("href", "#myRect"), ("x", "5"), ("y", "5")],
            )),
        );

        let (snapshot, _refs) = DomSnapshot::from_tree(&svg);
        let content = collect_svg(&snapshot, 0, ColorScheme::Light, Color::default()).unwrap();
        assert_eq!(content.shapes.len(), 1);
        // The rect is translated by x=5,y=5: from (1,1)×(3,3) to (6,6)×(3,3).
        let bb = content.shapes[0].path.bounding_box().unwrap();
        assert!((bb.x - 6.0).abs() < 1e-6);
        assert!((bb.y - 6.0).abs() < 1e-6);
        assert!((bb.width - 3.0).abs() < 1e-6);
        assert!((bb.height - 3.0).abs() < 1e-6);
        assert_eq!(content.shapes[0].fill, Brush::Solid(Color(255, 0, 0, 255)));
    }

    /// `<use>` referencing a `<g>` clones all of the group's children.
    #[test]
    fn collect_svg_use_element_references_group() {
        let svg = TreeNode::new(element("svg", &[("viewBox", "0 0 20 10")]));
        let defs = TreeNode::new(element("defs", &[]));
        TreeNode::add_child(&svg, Rc::clone(&defs));
        let g = TreeNode::new(element("g", &[("id", "myGroup")]));
        TreeNode::add_child(
            &g,
            TreeNode::new(element(
                "rect",
                &[
                    ("x", "0"),
                    ("y", "0"),
                    ("width", "2"),
                    ("height", "2"),
                    ("fill", "#ff0000"),
                ],
            )),
        );
        TreeNode::add_child(
            &g,
            TreeNode::new(element(
                "circle",
                &[("cx", "5"), ("cy", "5"), ("r", "2"), ("fill", "#00ff00")],
            )),
        );
        TreeNode::add_child(&defs, Rc::clone(&g));
        TreeNode::add_child(&svg, TreeNode::new(element("use", &[("href", "#myGroup")])));

        let (snapshot, _refs) = DomSnapshot::from_tree(&svg);
        let content = collect_svg(&snapshot, 0, ColorScheme::Light, Color::default()).unwrap();
        assert_eq!(content.shapes.len(), 2);
        assert_eq!(content.shapes[0].fill, Brush::Solid(Color(255, 0, 0, 255)));
        assert_eq!(content.shapes[1].fill, Brush::Solid(Color(0, 255, 0, 255)));
    }

    /// Circular `<use>` references do not cause infinite recursion.
    #[test]
    fn collect_svg_use_circular_reference() {
        let svg = TreeNode::new(element("svg", &[("viewBox", "0 0 10 10")]));
        let use_a = TreeNode::new(element("use", &[("id", "a"), ("href", "#b")]));
        let use_b = TreeNode::new(element("use", &[("id", "b"), ("href", "#a")]));
        TreeNode::add_child(&svg, Rc::clone(&use_a));
        TreeNode::add_child(&use_a, Rc::clone(&use_b));

        let (snapshot, _refs) = DomSnapshot::from_tree(&svg);
        let content = collect_svg(&snapshot, 0, ColorScheme::Light, Color::default());
        // Circular reference is detected: no shapes are produced.
        assert!(content.is_none());
    }

    /// `<use>` SVG rasterizes identically through inline and standalone pipelines.
    #[test]
    fn use_svg_inline_matches_standalone() {
        let markup = r##"<svg width="20" height="10" viewBox="0 0 20 10">
            <defs>
                <rect id="r1" x="0" y="0" width="4" height="4" fill="#ff0000"/>
                <circle id="c1" cx="8" cy="5" r="2" fill="#00ff00"/>
            </defs>
            <use href="#r1" x="2" y="2"/>
            <use href="#c1" x="4" y="0"/>
        </svg>"##;

        // Standalone rasterization.
        let standalone =
            super::rasterize::rasterize(markup, Some((20, 10))).expect("rasterizes standalone");

        // Inline pipeline.
        let tree = crate::engine::html::xml::parse(markup).expect("parses SVG XML");
        let html_root = xml_to_html_tree(&tree.root);
        let (snapshot, _refs) = DomSnapshot::from_tree(&html_root);

        let content = collect_svg(&snapshot, 0, ColorScheme::Light, Color::default())
            .expect("collects inline SVG");
        let mut commands = Vec::new();
        emit_commands(
            &mut commands,
            (0.0, 0.0, 20.0, 10.0),
            false,
            content.view_box,
            &content.shapes,
            1.0,
        );
        let inline_pixels = rasterize_commands(&commands, 20, 10);

        assert_eq!(
            inline_pixels, standalone.rgba,
            "inline and standalone SVG rasterization differ for <use>"
        );
        assert!(
            inline_pixels.chunks(4).any(|p| p[3] != 0),
            "expected non-transparent pixels"
        );
    }

    /// `<clipPath id="...">` collects its shape descendants into a clip path
    /// that shapes can reference via `clip-path="url(#id)"`.
    #[test]
    fn collect_svg_clip_path() {
        let svg = TreeNode::new(element("svg", &[("viewBox", "0 0 10 10")]));
        let clip = TreeNode::new(element("clipPath", &[("id", "myClip")]));
        TreeNode::add_child(
            &clip,
            TreeNode::new(element(
                "rect",
                &[("x", "2"), ("y", "2"), ("width", "4"), ("height", "4")],
            )),
        );
        TreeNode::add_child(&svg, Rc::clone(&clip));
        TreeNode::add_child(
            &svg,
            TreeNode::new(element(
                "rect",
                &[
                    ("x", "0"),
                    ("y", "0"),
                    ("width", "10"),
                    ("height", "10"),
                    ("fill", "#ff0000"),
                    ("clip-path", "#myClip"),
                ],
            )),
        );

        let (snapshot, _refs) = DomSnapshot::from_tree(&svg);
        let content = collect_svg(&snapshot, 0, ColorScheme::Light, Color::default()).unwrap();
        assert_eq!(content.shapes.len(), 1);
        let shape = &content.shapes[0];
        assert!(shape.clip_path.is_some());
        let clip = shape.clip_path.as_ref().unwrap();
        // The clip rect is (2,2)×(4,4).
        let bb = clip.path.bounding_box().unwrap();
        assert!((bb.x - 2.0).abs() < 1e-6);
        assert!((bb.y - 2.0).abs() < 1e-6);
        assert!((bb.width - 4.0).abs() < 1e-6);
        assert!((bb.height - 4.0).abs() < 1e-6);
    }

    /// `<clipPath>` rasterizes identically through inline and standalone pipelines.
    #[test]
    fn clip_path_svg_inline_matches_standalone() {
        let markup = r##"<svg width="10" height="10" viewBox="0 0 10 10">
            <clipPath id="c1">
                <rect x="2" y="2" width="4" height="4"/>
            </clipPath>
            <rect x="0" y="0" width="10" height="10" fill="#ff0000" clip-path="#c1"/>
        </svg>"##;

        let standalone =
            super::rasterize::rasterize(markup, Some((10, 10))).expect("rasterizes standalone");

        let tree = crate::engine::html::xml::parse(markup).expect("parses SVG XML");
        let html_root = xml_to_html_tree(&tree.root);
        let (snapshot, _refs) = DomSnapshot::from_tree(&html_root);

        let content = collect_svg(&snapshot, 0, ColorScheme::Light, Color::default())
            .expect("collects inline SVG");
        let mut commands = Vec::new();
        emit_commands(
            &mut commands,
            (0.0, 0.0, 10.0, 10.0),
            false,
            content.view_box,
            &content.shapes,
            1.0,
        );
        let inline_pixels = rasterize_commands(&commands, 10, 10);

        assert_eq!(
            inline_pixels, standalone.rgba,
            "inline and standalone SVG rasterization differ for <clipPath>"
        );
    }

    #[test]
    fn collect_svg_memoizes_and_invalidates_on_content_change() {
        let svg = TreeNode::new(element("svg", &[("viewBox", "0 0 4 4")]));
        TreeNode::add_child(
            &svg,
            TreeNode::new(element(
                "rect",
                &[
                    ("x", "0"),
                    ("y", "0"),
                    ("width", "4"),
                    ("height", "4"),
                    ("fill", "#ff0000"),
                ],
            )),
        );
        let (snapshot, _refs) = DomSnapshot::from_tree(&svg);

        // Clear the global cache so parallel tests cannot pollute it.
        svg_content_cache().lock().unwrap().clear();
        // Reset the per-thread counter so parallel tests cannot pollute it.
        COLLECT_SVG_MISSES.with(|c| c.store(0, std::sync::atomic::Ordering::Relaxed));
        let first = collect_svg(&snapshot, 0, ColorScheme::Light, Color::default())
            .expect("collects red svg");
        let m1 = COLLECT_SVG_MISSES.with(|c| c.load(std::sync::atomic::Ordering::Relaxed));
        let second = collect_svg(&snapshot, 0, ColorScheme::Light, Color::default())
            .expect("collects red svg");
        let m2 = COLLECT_SVG_MISSES.with(|c| c.load(std::sync::atomic::Ordering::Relaxed));

        // Same content: second call hits the cache and is not re-collected.
        assert_eq!(first.view_box, second.view_box);
        assert_eq!(first.shapes.len(), second.shapes.len());
        assert_eq!(first.shapes[0].fill, second.shapes[0].fill);
        assert_eq!(m1, 1, "first call collects");
        assert_eq!(m2, 1, "repeat call is a cache hit (no re-collection)");

        // Different fill changes the content signature, so it re-collects.
        let svg2 = TreeNode::new(element("svg", &[("viewBox", "0 0 4 4")]));
        TreeNode::add_child(
            &svg2,
            TreeNode::new(element(
                "rect",
                &[
                    ("x", "0"),
                    ("y", "0"),
                    ("width", "4"),
                    ("height", "4"),
                    ("fill", "#00ff00"),
                ],
            )),
        );
        let (snapshot2, _refs) = DomSnapshot::from_tree(&svg2);
        svg_content_cache().lock().unwrap().clear();
        COLLECT_SVG_MISSES.with(|c| c.store(0, std::sync::atomic::Ordering::Relaxed));
        let m3 = COLLECT_SVG_MISSES.with(|c| c.load(std::sync::atomic::Ordering::Relaxed));
        let changed = collect_svg(&snapshot2, 0, ColorScheme::Light, Color::default())
            .expect("collects green svg");
        let m4 = COLLECT_SVG_MISSES.with(|c| c.load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(m3, 0, "counter reset before changed-content collect");
        assert_eq!(m4, 1, "changed content re-collects");
        assert_ne!(first.shapes[0].fill, changed.shapes[0].fill);
    }

    #[test]
    fn standalone_svg_draw_commands_match_software_rasterize() {
        let svg = r##"<svg width="4" height="4" viewBox="0 0 4 4"><rect width="4" height="4" fill="#ff0000"/></svg>"##;
        let commands = super::rasterize::svg_to_draw_commands(svg, Some((4, 4))).expect("converts");
        assert!(
            commands
                .iter()
                .all(|cmd| matches!(cmd, DrawCommand::Fill { .. }))
        );
        let command_pixels = rasterize_commands(&commands, 4, 4);
        let software = super::rasterize::rasterize(svg, Some((4, 4))).expect("rasterizes");
        assert_eq!((software.width, software.height), (4, 4));
        // GPU-path commands must produce identical pixels to software rasterization.
        assert_eq!(command_pixels, software.rgba);
    }
}
