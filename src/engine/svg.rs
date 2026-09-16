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
//! Coverage is intentionally minimal: single-color `<path d>` subpaths in one
//! `viewBox`. `<g>`/`<use>`/`<clipPath>`/gradients and `transform` attributes
//! on inline SVGs are out of scope. The rasterizer additionally handles
//! `<rect>`/`<circle>`/`<ellipse>` and group `transform` attributes.

mod rasterize;

pub use rasterize::{RasterResult, rasterize_from_bytes};

use crate::engine::css::values::CssValue;
use crate::engine::layouter::css_resolver::resolve_inline_value;
use crate::engine::layouter::dom_snapshot::{DomSnapshot, NodeId};
use crate::engine::layouter::resolve_css_color;
use crate::engine::layouter::types::{Color, ColorScheme};
use crate::engine::renderer_model::Path;
use crate::engine::renderer_model::{AffineTransform, Brush, DrawCommand, FillRule, Paint};

/// An `SVG viewBox` rectangle: `(min-x, min-y, width, height)`.
pub type ViewBox = (f32, f32, f32, f32);

/// A single solid-color fill inside an inline SVG: a viewBox-space path, its
/// paint color, and the fill rule (default nonzero).
#[derive(Debug, Clone)]
pub struct SvgShape {
    pub path: Path,
    pub fill: Color,
    pub rule: FillRule,
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
/// `Z/z`. Arc segments (`A/a`) are out of scope and are approximated by a
/// straight line to the arc endpoint. Repeated coordinate sets after a
/// command repeat that command (per the SVG grammar `M m x y ...` implicit
/// lineto included). Malformed data is skipped defensively.
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
                // Arc segments are out of scope: consume the parameter set and
                // draw a straight line to the arc endpoint so the silhouette
                // remains roughly intact.
                let rel = cmd == 'a';
                loop {
                    let (Some(_rx), Some(_ry), Some(_rot), Some(_large), Some(_sweep), Some(p)) = (
                        nums.next_number(),
                        nums.next_number(),
                        nums.next_number(),
                        nums.next_number(),
                        nums.next_number(),
                        nums.next_point(rel, cur),
                    ) else {
                        break;
                    };
                    path.line_to(p.0, p.1);
                    cur = Some(p);
                }
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
        _ => resolve_css_color("fill", &css, color_scheme),
    }
}

fn is_transparent_keyword(keyword: &str) -> bool {
    keyword.eq_ignore_ascii_case("none")
        || keyword.eq_ignore_ascii_case("transparent")
        || keyword.eq_ignore_ascii_case("initial")
        || keyword.eq_ignore_ascii_case("unset")
}

/// Uniform attribute access over the DOM snapshot's [`HtmlNodeType`] and the
/// rasterizer's [`XmlElement`], so fill/rule/opacity resolution is shared.
pub(crate) trait SvgElement {
    fn svg_attr(&self, name: &str) -> Option<&str>;
}

impl SvgElement for crate::engine::html::HtmlNodeType {
    fn svg_attr(&self, name: &str) -> Option<&str> {
        self.get_attr(name)
    }
}

impl SvgElement for crate::engine::html::xml::XmlElement {
    fn svg_attr(&self, name: &str) -> Option<&str> {
        self.attr(name)
    }
}

/// Resolves an SVG `fill` into a concrete color: a `fill` attribute wins over
/// a `style` attribute's `fill:` declaration, then the SVG default (black).
/// `none`/`transparent` produce an invisible fill; `currentColor` resolves to
/// `current_color`.
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

/// Collects the inline SVG content of an `<svg>` element: its `viewBox` and
/// the solid-color `<path>` subpaths of its direct children.
///
/// `text_color` is the `<svg>` element's computed CSS `color`, used to resolve
/// `fill="currentColor"`. Returns `None` when the element has no usable
/// `viewBox` or no paintable subpaths.
pub fn collect_svg(
    snapshot: &DomSnapshot,
    svg_id: NodeId,
    color_scheme: ColorScheme,
    text_color: Color,
) -> Option<SvgContent> {
    let view_box = parse_view_box(snapshot.node(svg_id).kind.get_attr("viewBox")?);
    if view_box.2 <= 0.0 || view_box.3 <= 0.0 {
        return None;
    }
    let mut shapes = Vec::new();
    for &child in snapshot.children(svg_id) {
        let child = &snapshot.node(child).kind;
        if child.tag_name() != Some("path") {
            continue;
        }
        let Some(d) = child.get_attr("d") else {
            continue;
        };
        let fill = resolve_shape_fill(child, color_scheme, text_color);
        let fill_opacity = numeric_presentation_value(child, "fill-opacity").unwrap_or(1.0);
        let rule = resolve_fill_rule(child);
        let path = path_from_d(d);
        if path.commands().is_empty() {
            continue;
        }
        shapes.push(SvgShape {
            path,
            fill: with_alpha(fill, fill_opacity),
            rule,
        });
    }
    if shapes.is_empty() {
        return None;
    }
    Some(SvgContent { view_box, shapes })
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
        cmd_buf.push(DrawCommand::Fill {
            path: shape.path.clone(),
            paint: Paint {
                brush: Brush::Solid(shape.fill),
                opacity,
            },
            rule: shape.rule,
        });
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
    use crate::engine::html::tokenizer::Attribute;
    use crate::engine::layouter::dom_snapshot::DomSnapshot;
    use crate::engine::renderer_model::PathCommand;
    use crate::engine::tree::TreeNode;

    fn element(tag: &str, attrs: &[(&str, &str)]) -> HtmlNodeType {
        HtmlNodeType::Element {
            tag_name: tag.to_string(),
            attributes: attrs
                .iter()
                .map(|(name, value)| Attribute {
                    name: name.to_string(),
                    value: value.to_string(),
                })
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
    fn path_arc_approximated_by_straight_line_to_endpoint() {
        let path = path_from_d("M 0 0 A 5 5 0 0 1 10 10");
        assert_eq!(path.commands().len(), 2); // MoveTo + LineTo
        assert_eq!(path.subpaths(), vec![vec![(0.0, 0.0), (10.0, 10.0)]]);
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
            fill: Color(255, 0, 0, 255),
            rule: FillRule::EvenOdd,
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
            fill: Color(255, 0, 0, 255),
            rule: FillRule::NonZero,
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
            fill: Color(255, 0, 0, 255),
            rule: FillRule::NonZero,
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
        assert_eq!(content.shapes[0].fill, Color(1, 2, 3, 255));
        assert!(matches!(content.shapes[0].rule, FillRule::NonZero));
        assert_eq!(content.shapes[1].fill, Color(0, 255, 0, 128));
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
}
