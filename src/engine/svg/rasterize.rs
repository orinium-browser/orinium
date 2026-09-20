//! Scanline rasterizer: turns parsed `<svg>` XML into straight-alpha RGBA
//! pixels, replacing the previous `resvg`/`tiny-skia` dependency.
//!
//! Coverage is intentionally minimal and matches the SVGs the engine embeds
//! (audio icons, decoded `<img src="*.svg">`): `<svg>`, container `<g>`,
//! `<path d>`, `<rect>`, `<circle>`, `<ellipse>`, `transform` attributes, solid
//! `fill` colors from either a `fill` attribute or a `style=` declaration,
//! `fill-rule`, and `opacity`/`fill-opacity`/`stroke-opacity` (approximated as
//! a per-shape alpha multiplier), plus strokes (via outline expansion). Edges
//! are anti-aliased with 4× vertical supersampling. `stroke` colors are
//! resolved from a `stroke` attribute or a `style=` declaration, with the
//! `stroke-width`/cap/join/miter-limit/dash parameters inherited down
//! containers. Gradients, masks, `<defs>`/`<use>`, and text are out of scope.

use std::cmp::Ordering;

use crate::engine::html::xml::XmlElement;
use crate::engine::layouter::types::{Color, ColorScheme};
use crate::engine::renderer_model::{AffineTransform, FillRule, Path};
use crate::engine::svg::{
    PaintState, SvgStroke, ViewBox, element_shape, numeric_presentation_value, parse_len,
    parse_transform, parse_view_box, viewbox_meet_transform, with_alpha,
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
        process_element(
            child,
            base,
            1.0,
            PaintState::new(),
            width,
            height,
            &mut rgba,
        )?;
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
/// and its ancestors. Shapes apply it (plus their own `fill-opacity` /
/// `stroke-opacity`) to the alpha. Group opacity is approximated by
/// multiplying the alpha of every descendant shape rather than compositing the
/// group as a single layer. `paint` carries the inherited `fill`/`stroke`
/// state that containers set for their descendants, mirroring the inline
/// `collect_svg` walk.
fn process_element(
    node: &NodeRef<XmlElement>,
    inherited: AffineTransform,
    opacity: f32,
    mut paint: PaintState,
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
    paint.apply(&el.value, ColorScheme::Light, Color::default());

    match name.as_str() {
        "g" | "a" | "svg" | "symbol" => {
            for child in el.children() {
                process_element(child, eff, opacity, paint.clone(), width, height, rgba)?;
            }
        }
        "path" | "rect" | "circle" | "ellipse" | "polygon" | "polyline" | "line" => {
            let Some(path) = element_shape(&el.value) else {
                return Ok(());
            };
            if path.commands().is_empty() {
                return Ok(());
            }
            let fill = with_alpha(
                paint.fill.unwrap_or(Color(0, 0, 0, 255)),
                opacity * paint.fill_opacity,
            );
            let stroke = paint.stroke.clone().map(|mut stroke| {
                stroke.color = with_alpha(stroke.color, opacity * paint.stroke_opacity);
                stroke
            });
            render_shape(
                &path,
                eff,
                fill,
                paint.rule,
                stroke.as_ref(),
                width,
                height,
                rgba,
            );
        }
        _ => {}
    }
    Ok(())
}

/// Fills `path` (expressed in viewBox coordinates by `eff`) into `rgba`,
/// honoring `rule`, then paints the resolved stroke outline on top the same
/// way the inline pipeline does: expand the outline in the already-transformed
/// space and fill it with the nonzero rule.
#[allow(clippy::too_many_arguments)]
fn render_shape(
    path: &Path,
    eff: AffineTransform,
    fill: Color,
    rule: FillRule,
    stroke: Option<&SvgStroke>,
    width: u32,
    height: u32,
    rgba: &mut [u8],
) {
    let transformed = crate::engine::renderer_model::transform_path(path, &eff);
    fill_path_into(rgba, width, height, &transformed, fill, rule);
    if let Some(stroke) = stroke {
        let outline = crate::engine::renderer_model::stroke_path(&transformed, &stroke.to_stroke());
        fill_path_into(
            rgba,
            width,
            height,
            &outline,
            stroke.color,
            FillRule::NonZero,
        );
    }
}

/// Number of sub-scanlines sampled per output row for anti-aliasing.
const AA_SUBSAMPLES: u32 = 4;

/// Fills `path` (already in viewport pixel coordinates) into `rgba`, honoring
/// `rule`.
///
/// Each output row is sampled at [`AA_SUBSAMPLES`] sub-scanlines and the
/// covered pixel columns are tallied, so boundary pixels blend into fractional
/// alpha instead of a hard on/off edge. Fully interior pixels keep their exact
/// fill alpha.
pub(crate) fn fill_path_into(
    rgba: &mut [u8],
    width: u32,
    height: u32,
    path: &Path,
    fill: Color,
    rule: FillRule,
) {
    if fill.3 == 0 || width == 0 || height == 0 {
        return;
    }
    let height_i = height as i64;

    let rings: Vec<Vec<(f32, f32)>> = path
        .subpaths()
        .into_iter()
        .filter(|ring| ring.len() >= 3)
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
    let row_end = (max_y.min(height as f32).ceil() as i64).clamp(0, height_i);

    for row in row_start..row_end {
        let row_offset = (row as usize) * (width as usize);
        let mut coverage = vec![0u16; width as usize];
        for sub in 0..AA_SUBSAMPLES {
            let yc = (row as f32) + (sub as f32 + 0.5) / AA_SUBSAMPLES as f32;
            for (prev_x, x) in inside_spans(&rings, yc, rule) {
                mark_columns(&mut coverage, prev_x, x);
            }
        }
        for (column, hits) in coverage.iter().enumerate() {
            if *hits == 0 {
                continue;
            }
            let covers = *hits as f32 / AA_SUBSAMPLES as f32;
            let alpha = (fill.3 as f32 * covers).round() as u8;
            blend_pixel(
                rgba,
                row_offset + column,
                Color(fill.0, fill.1, fill.2, alpha),
            );
        }
    }
}

/// Returns the scanline spans of `rings` at `yc` that are inside the shape per
/// `rule`, as `(prev_x, x)` intervals (half-open along y and x).
fn inside_spans(
    rings: &[Vec<(f32, f32)>],
    yc: f32,
    rule: FillRule,
) -> impl Iterator<Item = (f32, f32)> + '_ {
    let mut edges: Vec<(f32, f32)> = Vec::new();
    for ring in rings {
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

    Spans {
        edges,
        rule,
        prev_x: f32::NEG_INFINITY,
        winding: 0.0,
        parity: false,
    }
}

/// Lazily yields the inside intervals of the sorted edge list, carrying the
/// winding count (non-zero rule) or parity (even-odd rule) across the whole
/// edge list so overlapping rings accumulate correctly.
struct Spans {
    edges: Vec<(f32, f32)>,
    rule: FillRule,
    prev_x: f32,
    winding: f32,
    parity: bool,
}

impl Iterator for Spans {
    type Item = (f32, f32);

    fn next(&mut self) -> Option<(f32, f32)> {
        while let Some((x, direction)) = self.edges.first().copied() {
            self.edges.remove(0);
            let span = match self.rule {
                FillRule::NonZero => {
                    let span = if self.winding != 0.0 && x > self.prev_x {
                        Some((self.prev_x, x))
                    } else {
                        None
                    };
                    self.winding += direction;
                    span
                }
                FillRule::EvenOdd => {
                    let span = if self.parity && x > self.prev_x {
                        Some((self.prev_x, x))
                    } else {
                        None
                    };
                    self.parity = !self.parity;
                    span
                }
            };
            self.prev_x = x;
            if let Some(span) = span {
                return Some(span);
            }
        }
        None
    }
}

/// Ticks the pixel columns whose centers land inside the scanline span
/// `(prev_x, x)` in the supersample coverage tally.
fn mark_columns(coverage: &mut [u16], prev_x: f32, x: f32) {
    let width = coverage.len() as i64;
    let c0 = (((prev_x - 0.5).ceil().max(0.0)) as i64).min(width);
    let c1 = ((x - 0.5).ceil().max(0.0) as i64).min(width);
    for c in c0.max(0)..c1 {
        coverage[c as usize] += 1;
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
        // The triangle is offset by translate(2,2): points land at
        // (4,10), (20,7), (4,4). Pixel (10,5) sits on the slanted right edge,
        // so anti-aliasing gives it partial coverage (191/255) rather than a
        // hard step; (5,5) is fully interior.
        let edge = result.rgba[(5 * 20 + 10) * 4..(5 * 20 + 10) * 4 + 4].to_vec();
        assert_eq!(edge, [0, 255, 0, 191]);
        let interior = result.rgba[(5 * 20 + 5) * 4..(5 * 20 + 5) * 4 + 4].to_vec();
        assert_eq!(interior, [0, 255, 0, 255]);
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
    fn anti_aliases_pixels_straddling_a_horizontal_edge() {
        // A rectangle from y=0.5 to y=2.5 in a 3-row canvas: rows 0 and 2 are
        // half covered (2 of 4 sub-scanlines), row 1 is fully covered.
        let result = rasterize(
            r##"<svg width="2" height="3"><rect x="0" y="0.5" width="2" height="2" fill="#ff0000"/></svg>"##,
            None,
        )
        .expect("rasterizes");
        let px = |row: usize| result.rgba[row * 8..row * 8 + 4].to_vec();
        assert_eq!(px(0), [255, 0, 0, 128]);
        assert_eq!(px(1), [255, 0, 0, 255]);
        assert_eq!(px(2), [255, 0, 0, 128]);
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
    fn stroke_width_zero_and_stroke_none_suppress_the_line() {
        // `stroke-width:0` and `stroke:none` (the audio icon cases) must leave
        // the canvas untouched even when a fill paint exists.
        let width_zero = rasterize(
            r##"<svg width="3" height="3"><path d="M0 1.5 L3 1.5" stroke="#ff0000" stroke-width="0"/></svg>"##,
            None,
        )
        .expect("rasterizes");
        assert!(width_zero.rgba.chunks(4).all(|p| p[3] == 0));
        let none = rasterize(
            r##"<svg width="3" height="3"><path d="M0 1.5 L3 1.5" stroke="none" stroke-width="2"/></svg>"##,
            None,
        )
        .expect("rasterizes");
        assert!(none.rgba.chunks(4).all(|p| p[3] == 0));
        let no_declaration = rasterize(
            r##"<svg width="3" height="3"><path d="M0 1.5 L3 1.5"/></svg>"##,
            None,
        )
        .expect("rasterizes");
        assert!(no_declaration.rgba.chunks(4).all(|p| p[3] == 0));
    }

    #[test]
    fn stroke_rasterizes_as_an_antialiased_band_around_the_line() {
        let result = rasterize(
            r##"<svg width="6" height="4"><path d="M0 1.5 L6 1.5" stroke="#ff0000" stroke-width="2" fill="none"/></svg>"##,
            None,
        )
        .expect("rasterizes");
        let px = |x: usize, y: usize| &result.rgba[(y * 6 + x) * 4..(y * 6 + x) * 4 + 4];
        // Band spans y in [0.5, 2.5]: center row full, neighbors halves, the
        // adjacent row empty (same AA profile as a filled rectangle).
        assert_eq!(px(2, 1), &[255, 0, 0, 255]);
        assert_eq!(px(2, 0), &[255, 0, 0, 128]);
        assert_eq!(px(2, 2), &[255, 0, 0, 128]);
        assert_eq!(px(2, 3), &[0, 0, 0, 0]);
        // Butt caps stop at the endpoint; nothing fills the corner cells.
        assert_eq!(px(0, 3), &[0, 0, 0, 0]);
    }

    #[test]
    fn round_cap_bulges_past_the_endpoint_where_butt_does_not() {
        let round = rasterize(
            r##"<svg width="8" height="5"><path d="M0.5 2.5 L3.5 2.5" stroke="#ff0000" stroke-width="4" stroke-linecap="round" fill="none"/></svg>"##,
            None,
        )
        .expect("rasterizes round");
        let butt = rasterize(
            r##"<svg width="8" height="5"><path d="M0.5 2.5 L3.5 2.5" stroke="#ff0000" stroke-width="4" fill="none"/></svg>"##,
            None,
        )
        .expect("rasterizes butt");
        fn px_at<'a>(rgba: &'a [u8], width: usize, x: usize, y: usize) -> &'a [u8] {
            &rgba[(y * width + x) * 4..(y * width + x) * 4 + 4]
        }

        // Right of the end at x=3.5, the radius-2 cap circle reaches column 4
        // only for the round cap.
        assert_eq!(px_at(&round.rgba, 8, 4, 2), &[255, 0, 0, 255]);
        assert_eq!(px_at(&butt.rgba, 8, 4, 2), &[0, 0, 0, 0]);
        // Two widths further out no cap reaches.
        assert_eq!(px_at(&round.rgba, 8, 6, 2), &[0, 0, 0, 0]);
    }

    #[test]
    fn dasharray_splits_the_stroke_into_on_and_off_phases() {
        let result = rasterize(
            r##"<svg width="8" height="3"><path d="M0 1.5 L8 1.5" stroke="#ff0000" stroke-width="2" stroke-dasharray="3 3" fill="none"/></svg>"##,
            None,
        )
        .expect("rasterizes");
        let px = |x: usize, y: usize| &result.rgba[(y * 8 + x) * 4..(y * 8 + x) * 4 + 4];
        assert_eq!(px(1, 1), &[255, 0, 0, 255]);
        assert_eq!(px(4, 1), &[0, 0, 0, 0]);
        assert_eq!(px(6, 1), &[255, 0, 0, 255]);
    }

    #[test]
    fn stroke_parameters_inherit_through_a_group() {
        let result = rasterize(
            r##"<svg width="4" height="3"><g stroke="#00ff00" stroke-width="2"><path d="M0 1.5 L4 1.5" fill="none"/></g></svg>"##,
            None,
        )
        .expect("rasterizes");
        let px = |x: usize, y: usize| &result.rgba[(y * 4 + x) * 4..(y * 4 + x) * 4 + 4];
        assert_eq!(px(1, 1), &[0, 255, 0, 255]);
        assert_eq!(px(1, 0), &[0, 255, 0, 128]);
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
