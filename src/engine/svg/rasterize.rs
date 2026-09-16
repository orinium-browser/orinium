//! Scanline rasterizer: turns parsed `<svg>` XML into straight-alpha RGBA
//! pixels, replacing the previous `resvg`/`tiny-skia` dependency.
//!
//! Coverage is intentionally minimal and matches the SVGs the engine embeds
//! (audio icons, decoded `<img src="*.svg">`): `<svg>`, container `<g>`,
//! `<path d>`, `<rect>`, `<circle>`, `<ellipse>`, `transform` attributes, solid
//! `fill` colors from either a `fill` attribute or a `style=` declaration,
//! `fill-rule`, and `opacity`/`fill-opacity` (approximated as a per-shape alpha
//! multiplier). Strokes, gradients, masks, `<defs>`/`<use>`, and text are out
//! of scope.

use std::cmp::Ordering;

use crate::engine::html::xml::XmlElement;
use crate::engine::layouter::types::{Color, ColorScheme};
use crate::engine::renderer_model::{AffineTransform, FillRule, Path, ellipse_path, rect_path};
use crate::engine::svg::{
    ViewBox, numeric_presentation_value, parse_view_box, path_from_d, resolve_fill_rule,
    resolve_shape_fill, viewbox_meet_transform, with_alpha,
};
use crate::engine::tree::NodeRef;

/// Decoded RGBA8 pixels plus the rasterized viewport dimensions.
#[derive(Debug, Clone, PartialEq)]
pub struct RasterResult {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Parses SVG bytes and rasterizes them to straight-alpha RGBA pixels.
///
/// `target_size` fixes the output viewport. When `None`, the SVG's intrinsic
/// `width`/`height` (falling back to `viewBox`, then 300×150) determine the
/// output size, matching `resvg`'s sizing behavior.
pub fn rasterize_from_bytes(
    svg: &[u8],
    target_size: Option<(u32, u32)>,
) -> Result<RasterResult, String> {
    let markup =
        std::str::from_utf8(svg).map_err(|error| format!("SVG is not valid UTF-8: {error}"))?;
    rasterize(markup, target_size)
}

/// Rasterizes an SVG document string into a viewport-sized RGBA buffer.
pub fn rasterize(markup: &str, target: Option<(u32, u32)>) -> Result<RasterResult, String> {
    let tree = crate::engine::html::xml::parse(markup)?;
    let root = tree.root.borrow();
    let root_el = &root.value;
    if root_el.name != "svg" {
        return Err(format!(
            "not an SVG document (root element is <{}>)",
            root_el.name
        ));
    }

    let view_box = root_el.attr("viewBox").map(parse_view_box);
    let (vw, vh) = match target {
        Some((w, h)) => (w as f32, h as f32),
        None => (
            viewport_len(root_el, "width", view_box, 300.0),
            viewport_len(root_el, "height", view_box, 150.0),
        ),
    };
    let width = vw.max(1.0) as u32;
    let height = vh.max(1.0) as u32;

    let base = viewbox_meet_transform(view_box.unwrap_or((0.0, 0.0, 0.0, 0.0)), vw, vh);
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    for child in root.children() {
        process_element(child, base, 1.0, width, height, &mut rgba)?;
    }
    Ok(RasterResult {
        width,
        height,
        rgba,
    })
}

/// Resolves the intrinsic viewport size of an `<svg>`: the `width`/`height`
/// attributes when present (clamped to at least 1px), otherwise the matching
/// `viewBox` edge, otherwise `fallback`.
fn viewport_len(el: &XmlElement, attr: &str, view_box: Option<ViewBox>, fallback: f32) -> f32 {
    if let Some(value) = el.attr(attr).and_then(parse_len) {
        return value.max(1.0).ceil();
    }
    match view_box {
        Some((_, _, w, _)) if attr == "width" => w.max(1.0).ceil(),
        Some((_, _, _, h)) if attr == "height" => h.max(1.0).ceil(),
        _ => fallback.max(1.0).ceil(),
    }
}

/// Walks one element, composing its `transform` into the inherited transform
/// and rasterizing any shape elements it describes.
///
/// `opacity` is the product of the `opacity` presentation values of this hook
/// and its ancestors. Shapes apply it (plus their own `fill-opacity`) to the
/// fill alpha. Group opacity is approximated by multiplying the alpha of every
/// descendant shape rather than compositing the group as a single layer.
fn process_element(
    node: &NodeRef<XmlElement>,
    inherited: AffineTransform,
    opacity: f32,
    width: u32,
    height: u32,
    rgba: &mut [u8],
) -> Result<(), String> {
    let el = node.borrow();
    let name = el.value.name.clone();
    let eff = match el.value.attr("transform") {
        Some(value) => inherited.then(&parse_transform(value).map_err(|e| format!("{name}: {e}"))?),
        None => inherited,
    };
    let element_opacity = numeric_presentation_value(&el.value, "opacity").unwrap_or(1.0);
    let opacity = opacity * element_opacity.clamp(0.0, 1.0);

    match name.as_str() {
        "g" | "a" | "svg" | "symbol" => {
            for child in el.children() {
                process_element(child, eff, opacity, width, height, rgba)?;
            }
        }
        "path" => {
            let Some(d) = el.value.attr("d") else {
                return Ok(());
            };
            let path = path_from_d(d);
            let fill = resolve_fill(&el.value, opacity);
            let rule = resolve_fill_rule(&el.value);
            render_shape(&path, eff, fill, rule, width, height, rgba);
        }
        "rect" => {
            let x = el.value.attr("x").and_then(parse_len).unwrap_or(0.0);
            let y = el.value.attr("y").and_then(parse_len).unwrap_or(0.0);
            let w = el.value.attr("width").and_then(parse_len).unwrap_or(0.0);
            let h = el.value.attr("height").and_then(parse_len).unwrap_or(0.0);
            if w > 0.0 && h > 0.0 {
                let fill = resolve_fill(&el.value, opacity);
                let rule = resolve_fill_rule(&el.value);
                render_shape(&rect_path(x, y, w, h), eff, fill, rule, width, height, rgba);
            }
        }
        "circle" => {
            let cx = el.value.attr("cx").and_then(parse_len).unwrap_or(0.0);
            let cy = el.value.attr("cy").and_then(parse_len).unwrap_or(0.0);
            let r = el.value.attr("r").and_then(parse_len).unwrap_or(0.0);
            if r > 0.0 {
                let fill = resolve_fill(&el.value, opacity);
                let rule = resolve_fill_rule(&el.value);
                render_shape(
                    &ellipse_path(cx, cy, r, r),
                    eff,
                    fill,
                    rule,
                    width,
                    height,
                    rgba,
                );
            }
        }
        "ellipse" => {
            let cx = el.value.attr("cx").and_then(parse_len).unwrap_or(0.0);
            let cy = el.value.attr("cy").and_then(parse_len).unwrap_or(0.0);
            let rx = el.value.attr("rx").and_then(parse_len).unwrap_or(0.0);
            let ry = el.value.attr("ry").and_then(parse_len).unwrap_or(0.0);
            if rx > 0.0 && ry > 0.0 {
                let fill = resolve_fill(&el.value, opacity);
                let rule = resolve_fill_rule(&el.value);
                render_shape(
                    &ellipse_path(cx, cy, rx, ry),
                    eff,
                    fill,
                    rule,
                    width,
                    height,
                    rgba,
                );
            }
        }
        _ => {}
    }
    Ok(())
}

/// Resolves the effective fill color of a shape: `fill` attribute wins over a
/// `style`-attribute `fill:` declaration; the SVG default is black.
/// `opacity` (inherited product) and `fill-opacity` multiply the alpha.
fn resolve_fill(el: &XmlElement, opacity: f32) -> Color {
    let fill = resolve_shape_fill(el, ColorScheme::Light, Color::default());
    let fill_opacity = numeric_presentation_value(el, "fill-opacity").unwrap_or(1.0);
    with_alpha(fill, opacity * fill_opacity)
}

/// Parses an SVG length, tolerating trailing unit suffixes (`px`, `%`, …) and
/// scientific-notation exponents (`1.5e2`).
fn parse_len(value: &str) -> Option<f32> {
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
fn parse_transform(value: &str) -> Result<AffineTransform, String> {
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

/// Fills the part of `path` (already expressed in viewport coordinates by
/// `eff`) between its subpaths into `rgba`, honoring `rule` (nonzero winding
/// or even-odd).
fn render_shape(
    path: &Path,
    eff: AffineTransform,
    fill: Color,
    rule: FillRule,
    width: u32,
    height: u32,
    rgba: &mut [u8],
) {
    if fill.3 == 0 || width == 0 || height == 0 {
        return;
    }
    let height_i = height as i64;

    let rings: Vec<Vec<(f32, f32)>> = path
        .subpaths()
        .into_iter()
        .map(|ring| ring.into_iter().map(|(x, y)| eff.apply(x, y)).collect())
        .filter(|ring: &Vec<(f32, f32)>| ring.len() >= 3)
        .collect();
    if rings.is_empty() {
        return;
    }

    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for ring in &rings {
        for &(_, y) in ring {
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
    }

    let row_start = min_y.max(0.0).floor() as i64;
    let row_end = max_y.min(height as f32).ceil() as i64;
    let row_end = row_end.clamp(0, height_i);

    for row in row_start..row_end {
        let yc = row as f32 + 0.5;
        let mut edges: Vec<(f32, f32)> = Vec::new();
        for ring in &rings {
            let last = ring.len() - 1;
            for i in 0..ring.len() {
                let (x0, y0) = ring[i];
                let (x1, y1) = ring[if i == last { 0 } else { i + 1 }];
                let (lo, hi) = if y0 < y1 { (y0, y1) } else { (y1, y0) };
                // Half-open along y so each edge crosses exactly once per
                // scanline, and horizontal edges never cross.
                if yc < lo || yc >= hi {
                    continue;
                }
                let t = (yc - y0) / (y1 - y0);
                let x = x0 + t * (x1 - x0);
                let direction = if y1 > y0 { 1.0 } else { -1.0 };
                edges.push((x, direction));
            }
        }
        edges.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));

        match rule {
            FillRule::NonZero => {
                let mut winding = 0.0f32;
                let mut prev_x = f32::NEG_INFINITY;
                for (x, direction) in edges {
                    if winding != 0.0 && x > prev_x {
                        fill_columns(rgba, width, row, prev_x, x, fill);
                    }
                    winding += direction;
                    prev_x = x;
                }
            }
            FillRule::EvenOdd => {
                let mut parity = false;
                let mut prev_x = f32::NEG_INFINITY;
                for (x, _) in edges {
                    if parity && x > prev_x {
                        fill_columns(rgba, width, row, prev_x, x, fill);
                    }
                    parity = !parity;
                    prev_x = x;
                }
            }
        }
    }
}

/// Fills the pixel columns whose centers land inside the scanline span
/// `(prev_x, x)` with `fill`.
fn fill_columns(rgba: &mut [u8], width: u32, row: i64, prev_x: f32, x: f32, fill: Color) {
    let width_i = width as i64;
    let c0 = ((prev_x - 0.5).ceil().max(0.0)) as i64;
    let c1 = ((x - 0.5).ceil()) as i64;
    let c0 = c0.max(0);
    let c1 = c1.min(width_i);
    for c in c0..c1 {
        blend_pixel(rgba, (row as usize) * (width as usize) + c as usize, fill);
    }
}

/// Sources a straight-alpha color over `rgba[idx]` (the destination) using the
/// standard "over" compositing operator.
fn blend_pixel(rgba: &mut [u8], index: usize, src: Color) {
    let idx = index * 4;
    let sa = src.3 as f32 / 255.0;
    let da = rgba[idx + 3] as f32 / 255.0;
    let out_alpha = sa + da * (1.0 - sa);
    if out_alpha <= 0.0 {
        return;
    }
    let rgba_f32 = [
        src.0 as f32 / 255.0,
        src.1 as f32 / 255.0,
        src.2 as f32 / 255.0,
    ];
    for channel in 0..3 {
        let s = rgba_f32[channel];
        let d = rgba[idx + channel] as f32 / 255.0;
        let composited = (s * sa + d * da * (1.0 - sa)) / out_alpha;
        rgba[idx + channel] = (composited * 255.0).round() as u8;
    }
    rgba[idx + 3] = (out_alpha * 255.0).round() as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasterizes_fill_attribute_rectangle() {
        let result = rasterize(
            r##"<svg width="4" height="3"><rect x="0" y="0" width="4" height="3" fill="#ff0000"/></svg>"##,
            None,
        )
        .expect("rasterizes");
        assert_eq!((result.width, result.height), (4, 3));
        // Every pixel is solid red.
        assert!(
            result
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [255, 0, 0, 255])
        );
    }

    #[test]
    fn rasterizes_style_fill_with_transform_and_viewbox() {
        // A triangle in viewBox space, offset by the group's translate(2,2):
        // points land at (4,10), (20,7), (4,4).
        let result = rasterize(
            r##"<svg width="20" height="10" viewBox="0 0 20 10">
                <g transform="translate(2, 2)">
                  <path d="M 2 8 L 18 5 L 2 2 Z" style="fill:#00ff00"/>
                </g>
               </svg>"##,
            None,
        )
        .expect("rasterizes");
        assert_eq!((result.width, result.height), (20, 10));
        // Row 5 (center y=5.5) crosses the triangle at x=10; the top-left
        // corner is outside it.
        let center = result.rgba[(5 * 20 + 10) * 4..(5 * 20 + 10) * 4 + 4].to_vec();
        assert_eq!(center, [0, 255, 0, 255]);
        assert_eq!(&result.rgba[0..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn viewbox_meet_centers_letterboxed_content() {
        // viewBox 10x10 in a 20x10 viewport scales to fit and centers,
        // leaving the left and right 5px empty.
        let result = rasterize(
            r##"<svg width="20" height="10" viewBox="0 0 10 10"><rect x="0" y="0" width="10" height="10" fill="#ff0000"/></svg>"##,
            None,
        )
        .expect("rasterizes");
        assert_eq!(&result.rgba[0..4], &[0, 0, 0, 0]);
        assert_eq!(&result.rgba[40..44], &[255, 0, 0, 255]);
    }

    #[test]
    fn preserves_transparent_regions_outside_shapes() {
        let result = rasterize(
            r##"<svg width="3" height="3"><rect x="1" y="1" width="1" height="1" fill="#ffffff"/></svg>"##,
            None,
        )
        .expect("rasterizes");
        assert_eq!(&result.rgba[0..4], &[0, 0, 0, 0]);
        assert_eq!(&result.rgba[16..20], &[255, 255, 255, 255]);
    }

    #[test]
    fn non_svg_root_is_rejected() {
        assert!(rasterize("<html></html>", None).is_err());
        assert!(rasterize("<svg", None).is_err());
    }

    #[test]
    fn rectangle_bounds_are_axis_aligned_after_scale() {
        let result = rasterize(
            r##"<svg width="10" height="10"><rect x="2" y="1" width="3" height="2" fill="#0000ff"/></svg>"##,
            None,
        )
        .expect("rasterizes");
        let px = |x: usize, y: usize| &result.rgba[(y * 10 + x) * 4..(y * 10 + x) * 4 + 4];
        // Inside the rect: blue.
        assert_eq!(px(2, 1), &[0, 0, 255, 255]);
        // Outside: transparent.
        assert_eq!(px(1, 1), &[0, 0, 0, 0]);
        assert_eq!(px(5, 1), &[0, 0, 0, 0]);
        assert_eq!(px(2, 0), &[0, 0, 0, 0]);
    }

    #[test]
    fn even_odd_rule_punches_a_hole_between_overlapping_rings() {
        // One path, two overlapping square subpaths. Nonzero winding keeps the
        // overlap (winding 2) filled; even-odd leaves it empty.
        let rings = r##"<svg width="10" height="10">
            <path d="M1 1 L7 1 L7 7 L1 7 Z M3 3 L9 3 L9 9 L3 9 Z" fill="#ff0000"/>
        </svg>"##;
        let even_odd = rasterize(
            r##"<svg width="10" height="10">
                <path d="M1 1 L7 1 L7 7 L1 7 Z M3 3 L9 3 L9 9 L3 9 Z" fill="#ff0000" fill-rule="evenodd"/>
            </svg>"##,
            None,
        )
        .expect("rasterizes even-odd");
        let nonzero = rasterize(rings, None).expect("rasterizes nonzero");
        fn px(r: &RasterResult, x: usize, y: usize) -> &[u8] {
            &r.rgba[(y * 10 + x) * 4..(y * 10 + x) * 4 + 4]
        }
        // Overlap region (5,5): hole in even-odd, filled in nonzero.
        assert_eq!(px(&even_odd, 5, 5), &[0, 0, 0, 0]);
        assert_eq!(px(&nonzero, 5, 5), &[255, 0, 0, 255]);
        // Region inside exactly one ring (2,2): filled in both modes.
        assert_eq!(px(&even_odd, 2, 2), &[255, 0, 0, 255]);
        assert_eq!(px(&nonzero, 2, 2), &[255, 0, 0, 255]);
    }

    #[test]
    fn opacity_attribute_multiplies_fill_alpha() {
        let result = rasterize(
            r##"<svg width="2" height="2"><rect x="0" y="0" width="2" height="2" fill="#ff0000" opacity="0.5"/></svg>"##,
            None,
        )
        .expect("rasterizes");
        assert_eq!(&result.rgba[0..4], &[255, 0, 0, 128]);
    }

    #[test]
    fn group_opacity_inherits_into_children() {
        let result = rasterize(
            r##"<svg width="2" height="2"><g opacity="0.5"><rect x="0" y="0" width="2" height="2" fill="#ff0000"/></g></svg>"##,
            None,
        )
        .expect("rasterizes");
        assert_eq!(&result.rgba[0..4], &[255, 0, 0, 128]);
    }

    #[test]
    fn fill_opacity_compounds_with_element_opacity() {
        let result = rasterize(
            r##"<svg width="2" height="2"><rect x="0" y="0" width="2" height="2" fill="#ff0000" opacity="0.5" fill-opacity="0.5"/></svg>"##,
            None,
        )
        .expect("rasterizes");
        // 0.5 * 0.5 * 255 = 63.75 → 64.
        assert_eq!(&result.rgba[0..4], &[255, 0, 0, 64]);
    }

    #[test]
    fn current_color_falls_back_to_black_for_standalone_documents() {
        let result = rasterize(
            r##"<svg width="2" height="2"><rect x="0" y="0" width="2" height="2" fill="currentColor"/></svg>"##,
            None,
        )
        .expect("rasterizes");
        assert_eq!(&result.rgba[0..4], &[0, 0, 0, 255]);
    }

    #[test]
    fn parse_len_handles_units_and_exponents() {
        assert_eq!(parse_len("10px"), Some(10.0));
        assert_eq!(parse_len(" 1.5e1em "), Some(15.0));
        assert_eq!(parse_len("-2e1"), Some(-20.0));
        assert_eq!(parse_len("100%"), Some(100.0));
        assert_eq!(parse_len("nope"), None);
    }
}
