//! Path model for the renderer: a sequence of move/line/curve commands with
//! helpers for bounding boxes and polygon conversion for GPU rasterization.

use crate::engine::renderer_model::geom::{AffineTransform, Rect};

/// A single path drawing command.
#[derive(Debug, Clone, PartialEq)]
pub enum PathCommand {
    /// Move the current point to `(x, y)` without drawing.
    MoveTo { x: f32, y: f32 },
    /// Draw a straight line to `(x, y)`.
    LineTo { x: f32, y: f32 },
    /// Draw a quadratic Bézier curve to `(x, y)` with control point `(cx, cy)`.
    QuadTo { cx: f32, cy: f32, x: f32, y: f32 },
    /// Draw a cubic Bézier curve to `(x, y)` with control points
    /// `(c1x, c1y)` and `(c2x, c2y)`.
    CubicTo {
        c1x: f32,
        c1y: f32,
        c2x: f32,
        c2y: f32,
        x: f32,
        y: f32,
    },
    /// Close the current subpath back to its starting point.
    Close,
}

/// A path made of [`PathCommand`]s.
///
/// Used by `DrawCommand::Fill` and `DrawCommand::PushClip`. The GPU rasterizer
/// converts the path into a polygon with [`Path::as_polygon_vertices`],
/// flattening curved segments into line segments.
#[derive(Debug, Clone)]
pub struct Path {
    pub commands: Vec<PathCommand>,
    current: Option<(f32, f32)>,
    start: Option<(f32, f32)>,
}

impl Path {
    /// Creates an empty path.
    pub fn new() -> Self {
        Path {
            commands: Vec::new(),
            current: None,
            start: None,
        }
    }

    /// Starts a new subpath at `(x, y)`.
    pub fn move_to(&mut self, x: f32, y: f32) {
        self.commands.push(PathCommand::MoveTo { x, y });
        self.current = Some((x, y));
        self.start = Some((x, y));
    }

    /// Pushes an implicit [`PathCommand::MoveTo`] when the path has no current
    /// point yet, so a following line segment starts at `(x, y)`.
    fn ensure_current(&mut self, x: f32, y: f32) {
        if self.current.is_none() {
            self.commands.push(PathCommand::MoveTo { x, y });
        }
        self.current = Some((x, y));
    }

    /// Draws a straight line to `(x, y)`.
    ///
    /// If the path has no current point yet, the line is treated as a
    /// starting point.
    pub fn line_to(&mut self, x: f32, y: f32) {
        self.ensure_current(x, y);
        self.commands.push(PathCommand::LineTo { x, y });
    }

    /// Draws a quadratic Bézier curve to `p` with control point `c`.
    ///
    /// If the path has no current point yet, the curve is dropped and the
    /// path is moved to `p` instead (a curve has no defined start point).
    pub fn quad_to(&mut self, c: (f32, f32), p: (f32, f32)) {
        if self.current.is_none() {
            self.move_to(p.0, p.1);
            return;
        }
        self.commands.push(PathCommand::QuadTo {
            cx: c.0,
            cy: c.1,
            x: p.0,
            y: p.1,
        });
        self.current = Some((p.0, p.1));
    }

    /// Draws a cubic Bézier curve to `p` with control points `c1` and `c2`.
    ///
    /// If the path has no current point yet, the curve is dropped and the
    /// path is moved to `p` instead (a curve has no defined start point).
    pub fn cubic_to(&mut self, c1: (f32, f32), c2: (f32, f32), p: (f32, f32)) {
        if self.current.is_none() {
            self.move_to(p.0, p.1);
            return;
        }
        self.commands.push(PathCommand::CubicTo {
            c1x: c1.0,
            c1y: c1.1,
            c2x: c2.0,
            c2y: c2.1,
            x: p.0,
            y: p.1,
        });
        self.current = Some((p.0, p.1));
    }

    /// Closes the current subpath, returning the current point to the
    /// subpath start.
    pub fn close(&mut self) {
        self.commands.push(PathCommand::Close);
        self.current = self.start;
    }

    /// Computes the axis-aligned bounding box of the path, or `None` for an
    /// empty path.
    ///
    /// Curve bounds include the control points, so they may overestimate the
    /// true extent.
    pub fn bounding_box(&self) -> Option<Rect> {
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for cmd in &self.commands {
            match cmd {
                PathCommand::MoveTo { x, y } | PathCommand::LineTo { x, y } => {
                    min_x = min_x.min(*x);
                    min_y = min_y.min(*y);
                    max_x = max_x.max(*x);
                    max_y = max_y.max(*y);
                }
                PathCommand::QuadTo { cx, cy, x, y } => {
                    min_x = min_x.min(*x).min(*cx);
                    min_y = min_y.min(*y).min(*cy);
                    max_x = max_x.max(*x).max(*cx);
                    max_y = max_y.max(*y).max(*cy);
                }
                PathCommand::CubicTo {
                    c1x,
                    c1y,
                    c2x,
                    c2y,
                    x,
                    y,
                } => {
                    min_x = min_x.min(*x).min(*c1x).min(*c2x);
                    min_y = min_y.min(*y).min(*c1y).min(*c2y);
                    max_x = max_x.max(*x).max(*c1x).max(*c2x);
                    max_y = max_y.max(*y).max(*c1y).max(*c2y);
                }
                PathCommand::Close => {}
            }
        }
        if min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite() {
            Some(Rect::new(min_x, min_y, max_x - min_x, max_y - min_y))
        } else {
            None
        }
    }
    /// Returns the path flattened into a list of vertices per subpath,
    /// curving segments into line segments.
    ///
    /// Each entry is one closed ring (its closing edge back to the first
    /// point is implicit). Multiple `MoveTo`s yield multiple rings.
    pub fn subpaths(&self) -> Vec<Vec<(f32, f32)>> {
        let mut rings: Vec<Vec<(f32, f32)>> = Vec::new();
        let mut current: Vec<(f32, f32)> = Vec::new();
        let mut cur_point: Option<(f32, f32)> = None;

        for cmd in &self.commands {
            match cmd {
                PathCommand::MoveTo { x, y } => {
                    if !current.is_empty() {
                        rings.push(std::mem::take(&mut current));
                    }
                    current.push((*x, *y));
                    cur_point = Some((*x, *y));
                }
                PathCommand::LineTo { x, y } => {
                    current.push((*x, *y));
                    cur_point = Some((*x, *y));
                }
                PathCommand::QuadTo { cx, cy, x, y } => {
                    if let Some(p0) = cur_point {
                        flatten_quad(p0, (*cx, *cy), (*x, *y), &mut current);
                    } else {
                        current.push((*x, *y));
                    }
                    cur_point = Some((*x, *y));
                }
                PathCommand::CubicTo {
                    c1x,
                    c1y,
                    c2x,
                    c2y,
                    x,
                    y,
                } => {
                    if let Some(p0) = cur_point {
                        flatten_cubic(p0, (*c1x, *c1y), (*c2x, *c2y), (*x, *y), &mut current);
                    } else {
                        current.push((*x, *y));
                    }
                    cur_point = Some((*x, *y));
                }
                PathCommand::Close => {
                    // The ring's closing edge is implicit; nothing to emit.
                }
            }
        }
        if !current.is_empty() {
            rings.push(current);
        }
        rings
    }

    /// Returns the path as a single flat list of vertices, or `None` if the
    /// path has fewer than three vertices total.
    ///
    /// Prefer [`Path::subpaths`] when subpaths must be triangulated
    /// independently.
    pub fn as_polygon_vertices(&self) -> Option<Vec<(f32, f32)>> {
        let rings = self.subpaths();
        let total: usize = rings.iter().map(Vec::len).sum();
        if total < 3 {
            return None;
        }
        Some(rings.into_iter().flatten().collect())
    }

    pub fn commands(&self) -> &[PathCommand] {
        &self.commands
    }
}

/// Maximum allowed deviation of flattened line segments from the original
/// Bézier curve, in logical pixels.
const FLATTEN_TOLERANCE: f32 = 0.25;

/// Recursively subdivide a cubic Bézier with de Casteljau's algorithm until
/// both control points lie within [`FLATTEN_TOLERANCE`] of the chord, then
/// append the segment endpoint to `out`.
fn flatten_cubic(
    p0: (f32, f32),
    c1: (f32, f32),
    c2: (f32, f32),
    p1: (f32, f32),
    out: &mut Vec<(f32, f32)>,
) {
    // Distance from a control point to the chord (p0..p1).
    let flatness = |p: (f32, f32)| -> f32 {
        let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
        let len_sq = dx * dx + dy * dy;
        if len_sq <= f32::EPSILON {
            ((p.0 - p0.0).powi(2) + (p.1 - p0.1).powi(2)).sqrt()
        } else {
            let t = (((p.0 - p0.0) * dx + (p.1 - p0.1) * dy) / len_sq).clamp(0.0, 1.0);
            let (qx, qy) = (p0.0 + t * dx, p0.1 + t * dy);
            ((p.0 - qx).powi(2) + (p.1 - qy).powi(2)).sqrt()
        }
    };

    if flatness(c1) <= FLATTEN_TOLERANCE && flatness(c2) <= FLATTEN_TOLERANCE {
        out.push(p1);
        return;
    }

    // Split at t = 0.5 and recurse on both halves.
    let mid = |a: (f32, f32), b: (f32, f32)| ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
    let m01 = mid(p0, c1);
    let m12 = mid(c1, c2);
    let m23 = mid(c2, p1);
    let m012 = mid(m01, m12);
    let m123 = mid(m12, m23);
    let m0123 = mid(m012, m123);

    flatten_cubic(p0, m01, m012, m0123, out);
    flatten_cubic(m0123, m123, m23, p1, out);
}

/// Flatten a quadratic Bézier into line segments by converting it to a cubic
/// and delegating to [`flatten_cubic`].
fn flatten_quad(p0: (f32, f32), c: (f32, f32), p1: (f32, f32), out: &mut Vec<(f32, f32)>) {
    let c1 = (
        p0.0 + (c.0 - p0.0) * 2.0 / 3.0,
        p0.1 + (c.1 - p0.1) * 2.0 / 3.0,
    );
    let c2 = (
        p1.0 + (c.0 - p1.0) * 2.0 / 3.0,
        p1.1 + (c.1 - p1.1) * 2.0 / 3.0,
    );
    flatten_cubic(p0, c1, c2, p1, out);
}

impl Default for Path {
    fn default() -> Self {
        Self::new()
    }
}

// Shape helpers
/// Builds a closed rectangle path with top-left corner at `(x, y)`.
pub fn rect_path(x: f32, y: f32, w: f32, h: f32) -> Path {
    let mut path = Path::new();
    path.move_to(x, y);
    path.line_to(x + w, y);
    path.line_to(x + w, y + h);
    path.line_to(x, y + h);
    path.close();
    path
}

/// Builds a closed ellipse path centered at `(cx, cy)` with radii `rx`/`ry`.
pub fn ellipse_path(cx: f32, cy: f32, rx: f32, ry: f32) -> Path {
    let k = 4.0 * (std::f32::consts::SQRT_2 - 1.0) / 3.0;
    let mut path = Path::new();
    path.move_to(cx + rx, cy);
    path.cubic_to(
        (cx + rx, cy - k * ry),
        (cx + k * rx, cy - ry),
        (cx, cy - ry),
    );
    path.cubic_to(
        (cx - k * rx, cy - ry),
        (cx - rx, cy - k * ry),
        (cx - rx, cy),
    );
    path.cubic_to(
        (cx - rx, cy + k * ry),
        (cx - k * rx, cy + ry),
        (cx, cy + ry),
    );
    path.cubic_to(
        (cx + k * rx, cy + ry),
        (cx + rx, cy + k * ry),
        (cx + rx, cy),
    );
    path.close();
    path
}

/// Builds a closed polygon path from the given vertex list.
pub fn polygon_path(points: &[(f32, f32)]) -> Path {
    if points.is_empty() {
        return Path::new();
    }
    let mut path = Path::new();
    path.move_to(points[0].0, points[0].1);
    for p in &points[1..] {
        path.line_to(p.0, p.1);
    }
    path.close();
    path
}

/// Builds an open polyline path through the given vertex list.
pub fn polyline_path(points: &[(f32, f32)]) -> Path {
    if points.is_empty() {
        return Path::new();
    }
    let mut path = Path::new();
    path.move_to(points[0].0, points[0].1);
    for p in &points[1..] {
        path.line_to(p.0, p.1);
    }
    path
}

/// Builds a single line segment from `(x1, y1)` to `(x2, y2)`.
pub fn line_path(x1: f32, y1: f32, x2: f32, y2: f32) -> Path {
    let mut path = Path::new();
    path.move_to(x1, y1);
    path.line_to(x2, y2);
    path
}

/// Translate every coordinate in `path` by `(ox, oy)`.
pub fn offset_path(path: &Path, ox: f32, oy: f32) -> Path {
    let mut out = Path::new();
    for cmd in path.commands() {
        match *cmd {
            PathCommand::MoveTo { x, y } => out.move_to(x + ox, y + oy),
            PathCommand::LineTo { x, y } => out.line_to(x + ox, y + oy),
            PathCommand::QuadTo { cx, cy, x, y } => {
                out.quad_to((cx + ox, cy + oy), (x + ox, y + oy))
            }
            PathCommand::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => out.cubic_to((c1x + ox, c1y + oy), (c2x + ox, c2y + oy), (x + ox, y + oy)),
            PathCommand::Close => out.close(),
        }
    }
    out
}

/// Apply an affine transform to every coordinate in `path`.
pub fn transform_path(path: &Path, transform: &AffineTransform) -> Path {
    let mut out = Path::new();
    for cmd in path.commands() {
        match *cmd {
            PathCommand::MoveTo { x, y } => {
                let (x, y) = transform.apply(x, y);
                out.move_to(x, y);
            }
            PathCommand::LineTo { x, y } => {
                let (x, y) = transform.apply(x, y);
                out.line_to(x, y);
            }
            PathCommand::QuadTo { cx, cy, x, y } => {
                let (cx, cy) = transform.apply(cx, cy);
                let (x, y) = transform.apply(x, y);
                out.quad_to((cx, cy), (x, y));
            }
            PathCommand::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => {
                let (c1x, c1y) = transform.apply(c1x, c1y);
                let (c2x, c2y) = transform.apply(c2x, c2y);
                let (x, y) = transform.apply(x, y);
                out.cubic_to((c1x, c1y), (c2x, c2y), (x, y));
            }
            PathCommand::Close => out.close(),
        }
    }
    out
}

/// Scale a set of corner radii `(rx, ry)` (CSS order TL, TR, BR, BL) down
/// proportionally so no opposing pair exceeds the box dimensions, following
/// the CSS `border-radius` clamping rule.
pub fn clamp_radii(radii: [(f32, f32); 4], w: f32, h: f32) -> [(f32, f32); 4] {
    let constraints = [
        if w > 0.0 && radii[0].0 + radii[1].0 > 0.0 {
            w / (radii[0].0 + radii[1].0)
        } else {
            1.0
        },
        if w > 0.0 && radii[2].0 + radii[3].0 > 0.0 {
            w / (radii[2].0 + radii[3].0)
        } else {
            1.0
        },
        if h > 0.0 && radii[0].1 + radii[3].1 > 0.0 {
            h / (radii[0].1 + radii[3].1)
        } else {
            1.0
        },
        if h > 0.0 && radii[1].1 + radii[2].1 > 0.0 {
            h / (radii[1].1 + radii[2].1)
        } else {
            1.0
        },
    ];
    let f = constraints.into_iter().fold(1.0f32, f32::min).max(0.0);
    if f >= 1.0 {
        return radii;
    }
    radii.map(|(rx, ry)| (rx * f, ry * f))
}

/// Append a single cubic Bézier approximating a quarter ellipse arc centered at
/// `(cx, cy)` with radii `(rx, ry)`, from point `from` to point `to`. The sweep
/// direction is derived from the relative position of the two endpoints.
pub(crate) fn append_quarter_ellipse(
    path: &mut Path,
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    from: (f32, f32),
    to: (f32, f32),
) {
    if rx <= 0.0 || ry <= 0.0 {
        path.line_to(to.0, to.1);
        return;
    }
    let k = 4.0 * (std::f32::consts::SQRT_2 - 1.0) / 3.0;
    let f = (from.0 - cx, from.1 - cy);
    let t = (to.0 - cx, to.1 - cy);
    // In y-down screen coordinates, the sign of the cross product tells us the
    // sweep direction between the two radial vectors.
    let sign = if f.0 * t.1 - f.1 * t.0 >= 0.0 {
        1.0
    } else {
        -1.0
    };
    let fu = (f.0 / rx, f.1 / ry);
    let tu = (t.0 / rx, t.1 / ry);
    let cp1 = (from.0 - sign * k * rx * fu.1, from.1 + sign * k * ry * fu.0);
    let cp2 = (to.0 + sign * k * rx * tu.1, to.1 - sign * k * ry * tu.0);
    path.cubic_to(cp1, cp2, to);
}

/// Builds a closed rounded rectangle path with top-left corner at `(x, y)`.
///
/// Corner radii are given as `(rx, ry)` pairs in CSS order (TL, TR, BR, BL) and
/// are clamped so opposing radii fit within the box.
#[allow(clippy::too_many_arguments)]
pub fn rounded_rect_path(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    tl: (f32, f32),
    tr: (f32, f32),
    br: (f32, f32),
    bl: (f32, f32),
) -> Path {
    let radii = clamp_radii([tl, tr, br, bl], w, h);
    let (tl, tr, br, bl) = (radii[0], radii[1], radii[2], radii[3]);
    let mut path = Path::new();
    path.move_to(x + w - tr.0, y);
    append_quarter_ellipse(
        &mut path,
        x + w - tr.0,
        y + tr.1,
        tr.0,
        tr.1,
        (x + w - tr.0, y),
        (x + w, y + tr.1),
    );
    path.line_to(x + w, y + h - br.1);
    append_quarter_ellipse(
        &mut path,
        x + w - br.0,
        y + h - br.1,
        br.0,
        br.1,
        (x + w, y + h - br.1),
        (x + w - br.0, y + h),
    );
    path.line_to(x + bl.0, y + h);
    append_quarter_ellipse(
        &mut path,
        x + bl.0,
        y + h - bl.1,
        bl.0,
        bl.1,
        (x + bl.0, y + h),
        (x, y + h - bl.1),
    );
    path.line_to(x, y + tl.1);
    append_quarter_ellipse(
        &mut path,
        x + tl.0,
        y + tl.1,
        tl.0,
        tl.1,
        (x, y + tl.1),
        (x + tl.0, y),
    );
    path.close();
    path
}

// ---------------------------------------------------------------------------
// Stroke geometry
// ---------------------------------------------------------------------------

/// How the open ends of a stroked subpath are finished.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StrokeCap {
    Butt,
    Round,
    Square,
}

/// How consecutive segments of a stroked subpath are joined.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StrokeJoin {
    /// Offset lines meet at their intersection, clamped to the miter limit.
    Miter,
    /// The join corners bulge into a round arc of the stroke half-width.
    Round,
    /// The join is a straight diagonal between the two offset edges.
    Bevel,
}

/// Defines the geometry of a stroked line: width, cap/join style, miter limit,
/// and optional dash pattern.
///
/// [`stroke_path`] turns this into a closed outline [`Path`] that the existing
/// fill pipeline (GPU and the software rasterizer) paints with the nonzero
/// rule, so stroking needs no dedicated tessellator or stencil pass.
#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub width: f32,
    pub cap: StrokeCap,
    pub join: StrokeJoin,
    /// Ratio of the maximum allowed miter length to the stroke width. When a
    /// miter join would exceed it, the join falls back to a bevel (SVG
    /// default 4).
    pub miter_limit: f32,
    /// Dash pattern `(dash, gap, dash, gap, …)`. `None` strokes a solid line.
    pub dash: Option<Vec<f32>>,
    /// How far into the dash pattern the start of each subpath is offset.
    pub dash_offset: f32,
}

impl Stroke {
    pub fn new(width: f32) -> Self {
        Stroke {
            width,
            cap: StrokeCap::Butt,
            join: StrokeJoin::Miter,
            miter_limit: 4.0,
            dash: None,
            dash_offset: 0.0,
        }
    }
}

/// Expands `path` into the closed outline of its stroke: the region swept by a
/// disk of radius `stroke.width / 2` traveling along the path.
///
/// The path is flattened first (curves become polylines) and each subpath is
/// offset on both sides along its segment normals, joined at the vertices. The
/// result is one or more closed rings that fill correctly under the nonzero
/// winding rule in both the GPU filler and the software rasterizer. Round
/// caps/joins are emitted as cubic arcs, keeping the outline smooth.
///
/// Known simplifications: a zero-length subpath draws nothing (the SVG
/// round-cap dot is omitted), non-uniform transforms are applied to the
/// expanded outline rather than to the width, and a dashed closed subpath gets
/// caps at the dash seam.
pub fn stroke_path(path: &Path, stroke: &Stroke) -> Path {
    let mut out = Path::new();
    let half = stroke.width * 0.5;
    if half <= 0.0 {
        return out;
    }
    for chain in stroke_chains(path) {
        let parts = match &stroke.dash {
            Some(pattern) => dash_chain(&chain, pattern, stroke.dash_offset),
            None => vec![chain.points.clone()],
        };
        for points in parts {
            if points.len() < 2 {
                continue;
            }
            if chain.closed {
                stroke_closed_ring(&mut out, &points, stroke, half);
            } else {
                stroke_open_ring(&mut out, &points, stroke, half);
            }
        }
    }
    out
}

/// A flattened subpath: vertex list plus whether it was explicitly closed.
struct StrokeChain {
    points: Vec<(f32, f32)>,
    closed: bool,
}

/// Walks the path commands, mirroring [`Path::subpaths`], but remembering
/// which subpaths end in an explicit `Close` (open ones need caps). Duplicate
/// consecutive vertices are dropped so segment normals stay well-defined.
fn stroke_chains(path: &Path) -> Vec<StrokeChain> {
    let mut chains: Vec<StrokeChain> = Vec::new();
    let mut cur: Option<(f32, f32)> = None;
    for cmd in path.commands() {
        match *cmd {
            PathCommand::MoveTo { x, y } => {
                chains.push(StrokeChain {
                    points: vec![(x, y)],
                    closed: false,
                });
                cur = Some((x, y));
            }
            PathCommand::LineTo { x, y } => {
                if let Some(chain) = chains.last_mut() {
                    push_point(&mut chain.points, (x, y));
                }
                cur = Some((x, y));
            }
            PathCommand::QuadTo { cx, cy, x, y } => {
                if let Some(chain) = chains.last_mut() {
                    if let Some(p0) = cur {
                        let mut flat = Vec::new();
                        flatten_quad(p0, (cx, cy), (x, y), &mut flat);
                        for p in flat {
                            push_point(&mut chain.points, p);
                        }
                    } else {
                        push_point(&mut chain.points, (x, y));
                    }
                }
                cur = Some((x, y));
            }
            PathCommand::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => {
                if let Some(chain) = chains.last_mut() {
                    if let Some(p0) = cur {
                        let mut flat = Vec::new();
                        flatten_cubic(p0, (c1x, c1y), (c2x, c2y), (x, y), &mut flat);
                        for p in flat {
                            push_point(&mut chain.points, p);
                        }
                    } else {
                        push_point(&mut chain.points, (x, y));
                    }
                }
                cur = Some((x, y));
            }
            PathCommand::Close => {
                if let Some(chain) = chains.last_mut() {
                    chain.closed = true;
                }
                cur = None;
            }
        }
    }
    chains.retain(|chain| chain.points.len() >= 2);
    chains
}

/// Appends `p` to `points` unless it coincides with the last point.
fn push_point(points: &mut Vec<(f32, f32)>, p: (f32, f32)) {
    if let Some(&last) = points.last()
        && (last.0 - p.0).abs() <= f32::EPSILON
        && (last.1 - p.1).abs() <= f32::EPSILON
    {
        return;
    }
    points.push(p);
}

/// Returns the unit left-hand normal of a direction vector.
fn normal(t: (f32, f32)) -> (f32, f32) {
    (-t.1, t.0)
}

/// Unit vector from `a` toward `b`, or `None` when they coincide.
fn tangent(a: (f32, f32), b: (f32, f32)) -> Option<(f32, f32)> {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len = (dx * dx + dy * dy).sqrt();
    (len > f32::EPSILON).then_some((dx / len, dy / len))
}

fn cross(u: (f32, f32), v: (f32, f32)) -> f32 {
    u.0 * v.1 - u.1 * v.0
}

/// Strokes an open polyline: one ring that starts with the start cap, follows
/// the +w side forward, crosses the end cap, and returns on the −w side.
fn stroke_open_ring(out: &mut Path, points: &[(f32, f32)], stroke: &Stroke, w: f32) {
    let last = points.len() - 1;
    let Some(t_first) = tangent(points[0], points[1]) else {
        return;
    };
    let Some(t_last) = tangent(points[last - 1], points[last]) else {
        return;
    };
    let n_first = normal(t_first);

    out.move_to(points[0].0 - w * n_first.0, points[0].1 - w * n_first.1);
    cap_at(out, stroke.cap, w, points[0], t_first, true);
    if last == 1 {
        // A single-segment chain has no interior joins to trace the forward
        // side with, so draw the +w edge explicitly.
        let n_last = normal(t_last);
        out.line_to(points[1].0 + w * n_last.0, points[1].1 + w * n_last.1);
    } else {
        for i in 1..last {
            let u = tangent(points[i - 1], points[i]).expect("non-degenerate chain");
            let v = tangent(points[i], points[i + 1]).expect("non-degenerate chain");
            join_at(out, stroke, w, points[i], u, v);
        }
    }
    cap_at(out, stroke.cap, w, points[last], t_last, false);
    for i in (1..last).rev() {
        let u = tangent(points[i], points[i + 1]).expect("non-degenerate chain");
        let v = tangent(points[i - 1], points[i]).expect("non-degenerate chain");
        join_at(out, stroke, -w, points[i], u, v);
    }
    out.close();
}

/// Strokes a closed polyline: the +w ring forward and the −w ring in reverse,
/// so the band between them carries winding +1 but the inner hole winds 0.
fn stroke_closed_ring(out: &mut Path, points: &[(f32, f32)], stroke: &Stroke, w: f32) {
    let n = points.len();
    if n < 3 {
        return;
    }
    let tangents: Vec<(f32, f32)> = (0..n)
        .map(|i| tangent(points[i], points[(i + 1) % n]).expect("non-degenerate chain"))
        .collect();

    // Forward ring on the +w side: stays on the same geometric loop as the
    // original, so its winding orientation matches the path order.
    out.move_to(
        points[0].0 + w * normal(tangents[0]).0,
        points[0].1 + w * normal(tangents[0]).1,
    );
    for i in 1..n {
        join_at(out, stroke, w, points[i], tangents[i - 1], tangents[i]);
    }
    join_at(out, stroke, w, points[0], tangents[n - 1], tangents[0]);
    out.close();
    // Reverse ring on the −w side: traversed in the opposite direction so the
    // two rings have opposing winding and only the band between them fills.
    out.move_to(
        points[0].0 - w * normal(tangents[n - 1]).0,
        points[0].1 - w * normal(tangents[n - 1]).1,
    );
    for i in (1..n).rev() {
        join_at(out, stroke, -w, points[i], tangents[i], tangents[i - 1]);
    }
    join_at(out, stroke, -w, points[0], tangents[0], tangents[n - 1]);
    out.close();
}

/// Draws the joint between the −w/+w offset edges of a vertex whose incoming
/// and outgoing tangents are `u` and `v`, transitioning from the offset point
/// on `u` to the offset point on `v`.
fn join_at(out: &mut Path, stroke: &Stroke, dir: f32, p: (f32, f32), u: (f32, f32), v: (f32, f32)) {
    let miter_limit = stroke.miter_limit.max(1.0) * dir.abs();
    let to = (p.0 + dir * normal(v).0, p.1 + dir * normal(v).1);
    match stroke.join {
        StrokeJoin::Bevel => out.line_to(to.0, to.1),
        StrokeJoin::Round => {
            let from = (p.0 + dir * normal(u).0, p.1 + dir * normal(u).1);
            append_arc(out, p, dir.abs(), from, to);
        }
        StrokeJoin::Miter => {
            // The current point is already the +-side offset of the incoming
            // segment, so the apex reaches the joined corner directly.
            match miter_point(dir, p, u, v) {
                Some(apex)
                    if (apex.0 - p.0).abs() <= miter_limit
                        && (apex.1 - p.1).abs() <= miter_limit =>
                {
                    out.line_to(apex.0, apex.1);
                    out.line_to(to.0, to.1);
                }
                _ => out.line_to(to.0, to.1),
            }
        }
    }
}

/// Intersection of the two offset lines at a vertex, or `None` for parallel
/// edges (a near-reversal where a miter would be infinitely long).
fn miter_point(dir: f32, p: (f32, f32), u: (f32, f32), v: (f32, f32)) -> Option<(f32, f32)> {
    let n_u = normal(u);
    let n_v = normal(v);
    let q = (p.0 + dir * n_u.0, p.1 + dir * n_u.1);
    let r = (p.0 + dir * n_v.0, p.1 + dir * n_v.1);
    let den = cross(u, v);
    if den.abs() < 1e-6 {
        return None;
    }
    // Solve q + λ·u = r + μ·v for λ.
    let b = (r.0 - q.0, r.1 - q.1);
    let lambda = cross(b, v) / den;
    Some((q.0 + lambda * u.0, q.1 + lambda * u.1))
}

/// Adds the cap of an open subpath end at `p` with tangent `t`, connecting the
/// offset on one side to the start of the contour on the other.
fn cap_at(out: &mut Path, cap: StrokeCap, w: f32, p: (f32, f32), t: (f32, f32), forward: bool) {
    let n = normal(t);
    let (from, to) = if forward {
        (
            (p.0 - w * n.0, p.1 - w * n.1),
            (p.0 + w * n.0, p.1 + w * n.1),
        )
    } else {
        (
            (p.0 + w * n.0, p.1 + w * n.1),
            (p.0 - w * n.0, p.1 - w * n.1),
        )
    };
    match cap {
        StrokeCap::Butt => out.line_to(to.0, to.1),
        StrokeCap::Round => {
            // The cap bulges away from the line: backwards at the start and
            // forwards at the end. The midpoint disambiguates which half of
            // the circle forms the rounded end.
            let out_dir = if forward { (-t.0, -t.1) } else { t };
            let mid = (p.0 + out_dir.0 * w, p.1 + out_dir.1 * w);
            append_arc_through(out, p, w, from, to, Some(mid));
        }
        StrokeCap::Square => {
            // The cap extends half the width *away* from the line: backwards
            // at the start (forward=true) and forwards at the end.
            let dir = if forward { (-t.0, -t.1) } else { t };
            let a = (from.0 + dir.0 * w, from.1 + dir.1 * w);
            out.line_to(a.0, a.1);
            let b = (to.0 + dir.0 * w, to.1 + dir.1 * w);
            out.line_to(b.0, b.1);
            out.line_to(to.0, to.1);
        }
    }
}

/// Bearings of each point around `center`, normalized to `(-PI, PI]`.
fn arc_angle_of(center: (f32, f32), p: (f32, f32)) -> f32 {
    (p.1 - center.1).atan2(p.0 - center.0)
}

/// Normalizes an angle difference to `(-PI, PI]`.
fn normalize_delta(d: f32) -> f32 {
    let mut d = d % std::f32::consts::TAU;
    if d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    } else if d <= -std::f32::consts::PI {
        d += std::f32::consts::TAU;
    }
    d
}

/// Appends an arc on the circle around `center` at radius `r` from `from` to
/// `to`, split into ≤ quarter-turn cubic Béziers.
///
/// The sweep direction follows the signed angle from `from` to `to`; when that
/// delta is exactly ±π the two half-circles are ambiguous, so the direction is
/// taken from `mid`, a point on the intended arc (e.g. the outer middle of a
/// round cap).
fn append_arc(out: &mut Path, center: (f32, f32), r: f32, from: (f32, f32), to: (f32, f32)) {
    append_arc_through(out, center, r, from, to, None);
}

fn append_arc_through(
    out: &mut Path,
    center: (f32, f32),
    r: f32,
    from: (f32, f32),
    to: (f32, f32),
    mid: Option<(f32, f32)>,
) {
    if r <= 0.0 {
        out.line_to(to.0, to.1);
        return;
    }
    let a0 = arc_angle_of(center, from);
    let a1 = arc_angle_of(center, to);
    let raw = normalize_delta(a1 - a0);
    if raw.abs() < f32::EPSILON {
        return;
    }
    // A delta of ±π doesn't carry its own direction; use the midpoint to
    // decide which way around the circle the short arc goes.
    let sweep = if (raw.abs() - std::f32::consts::PI).abs() < 1e-3 {
        match mid {
            Some(mid) => {
                let am = arc_angle_of(center, mid);
                if normalize_delta(am - a0).signum() > 0.0 {
                    raw.abs()
                } else {
                    -raw.abs()
                }
            }
            None => raw.signum() * std::f32::consts::PI,
        }
    } else {
        raw
    };
    let steps = (sweep.abs() / std::f32::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    for i in 0..steps {
        let a = a0 + sweep * (i as f32 / steps as f32);
        let b = a0 + sweep * ((i + 1) as f32 / steps as f32);
        let k = (4.0 / 3.0) * ((b - a) * 0.25).tan();
        let (sa, ca) = a.sin_cos();
        let (sb, cb) = b.sin_cos();
        let (p1x, p1y) = (center.0 + r * (ca - k * sa), center.1 + r * (sa + k * ca));
        let (p2x, p2y) = (center.0 + r * (cb + k * sb), center.1 + r * (sb - k * cb));
        out.cubic_to(
            (p1x, p1y),
            (p2x, p2y),
            (center.0 + r * cb, center.1 + r * sb),
        );
    }
}

/// Splits the points of a dashed chain into the polylines drawn during the
/// `dash` phases of the (cyclically repeated) pattern, honoring `dash_offset`.
fn dash_chain(chain: &StrokeChain, pattern: &[f32], dash_offset: f32) -> Vec<Vec<(f32, f32)>> {
    if pattern.is_empty() || pattern.iter().all(|d| *d <= 0.0) {
        return Vec::new();
    }
    // Odd-length patterns repeat doubled, per SVG.
    let even: Vec<f32> = if pattern.len() % 2 == 1 {
        pattern
            .iter()
            .copied()
            .chain(pattern.iter().copied())
            .collect()
    } else {
        pattern.to_vec()
    };
    let period: f32 = even.iter().sum();

    let mut segments: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut current: Option<Vec<(f32, f32)>> = None;
    let mut t = dash_offset.rem_euclid(period);
    let n = chain.points.len();
    let last = if chain.closed { n } else { n - 1 };
    for i in 0..last {
        let a = chain.points[i];
        let b = chain.points[(i + 1) % n];
        let seg_len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        if seg_len <= f32::EPSILON {
            continue;
        }
        let mut covered = 0.0;
        while covered < seg_len {
            let t_norm = t.rem_euclid(period);
            let phase_end = dash_phase_end(t_norm, &even);
            let step = (phase_end - t_norm).min(seg_len - covered);
            let local = (covered + step) / seg_len;
            let point = (a.0 + (b.0 - a.0) * local, a.1 + (b.1 - a.1) * local);
            if dash_is_on(t_norm, &even) {
                match &mut current {
                    Some(seg) => seg.push(point),
                    None => {
                        // A dash phase turning on exactly at a boundary starts
                        // its segment at that boundary, not at the step end.
                        let start_local = covered / seg_len;
                        let start_point = (
                            a.0 + (b.0 - a.0) * start_local,
                            a.1 + (b.1 - a.1) * start_local,
                        );
                        let mut seg = vec![start_point];
                        if start_point != point {
                            seg.push(point);
                        }
                        current = Some(seg);
                    }
                }
            } else if let Some(seg) = current.take() {
                segments.push(seg);
            }
            t = t_norm + step;
            covered += step;
        }
    }
    if let Some(seg) = current.take() {
        segments.push(seg);
    }
    segments
}

/// The pattern offset where the phase containing `t` ends.
fn dash_phase_end(t: f32, even: &[f32]) -> f32 {
    let mut acc = 0.0;
    for len in even {
        acc += len;
        if t < acc {
            return acc;
        }
    }
    acc
}

/// Whether the pattern offset `t` falls in a dash (even-indexed phase).
fn dash_is_on(t: f32, even: &[f32]) -> bool {
    let mut acc = 0.0;
    for (idx, len) in even.iter().enumerate() {
        if t < acc + len {
            return idx % 2 == 0;
        }
        acc += len;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_points_on_ellipse(
        points: &[(f32, f32)],
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        tol: f32,
    ) {
        for (px, py) in points {
            let v = ((px - cx) / rx).powi(2) + ((py - cy) / ry).powi(2);
            assert!(
                (v - 1.0).abs() < tol,
                "point ({px},{py}) not on ellipse: {v}"
            );
        }
    }

    #[test]
    fn test_rect_path_vertices_and_bounds() {
        let path = rect_path(10.0, 20.0, 100.0, 50.0);
        assert_eq!(
            path.as_polygon_vertices().unwrap(),
            vec![(10.0, 20.0), (110.0, 20.0), (110.0, 70.0), (10.0, 70.0)]
        );
        let bb = path.bounding_box().unwrap();
        assert!((bb.x - 10.0).abs() < 1e-6);
        assert!((bb.y - 20.0).abs() < 1e-6);
        assert!((bb.width - 100.0).abs() < 1e-6);
        assert!((bb.height - 50.0).abs() < 1e-6);
    }

    #[test]
    fn test_polygon_path() {
        let points = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)];
        let path = polygon_path(&points);
        assert_eq!(path.as_polygon_vertices().unwrap(), points.to_vec());
    }

    #[test]
    fn test_ellipse_path_flattens() {
        let path = ellipse_path(0.0, 0.0, 50.0, 30.0);
        let verts = path.as_polygon_vertices().unwrap();
        assert!(
            verts.len() > 8,
            "expected a flattened ellipse, got {} vertices",
            verts.len()
        );
        assert_eq!(verts.first().copied(), Some((50.0, 0.0)));
        assert_eq!(verts.last().copied(), Some((50.0, 0.0)));
        assert_points_on_ellipse(&verts, 0.0, 0.0, 50.0, 30.0, 0.01);
    }

    #[test]
    fn test_rounded_rect_corners_on_ellipse() {
        // Regression: `append_quarter_ellipse` mirrored the second control
        // point, drifting corner arcs off the true ellipse. The circle stayed
        // exact because `ellipse_path` inlines its own control points.
        let path = rounded_rect_path(
            100.0,
            100.0,
            200.0,
            200.0,
            (50.0, 50.0),
            (50.0, 50.0),
            (50.0, 50.0),
            (50.0, 50.0),
        );
        let verts = path.as_polygon_vertices().unwrap();
        let corners = [
            ((150.0, 150.0), (-1.0, -1.0)),
            ((250.0, 150.0), (1.0, -1.0)),
            ((250.0, 250.0), (1.0, 1.0)),
            ((150.0, 250.0), (-1.0, 1.0)),
        ];
        for (px, py) in verts {
            for ((cx, cy), (sx, sy)) in corners {
                if (px - cx) * sx > 0.0 && (py - cy) * sy > 0.0 {
                    let v = ((px - cx) / 50.0).powi(2) + ((py - cy) / 50.0).powi(2);
                    assert!(
                        (v - 1.0).abs() < 0.01,
                        "corner point ({px},{py}) not on radius-50 arc: {v}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_quad_curve_flattens() {
        let mut path = Path::new();
        path.move_to(0.0, 0.0);
        path.quad_to((10.0, 20.0), (30.0, 0.0));
        let verts = path.as_polygon_vertices().unwrap();
        assert_eq!(verts.first().copied(), Some((0.0, 0.0)));
        assert_eq!(verts.last().copied(), Some((30.0, 0.0)));
        assert!(verts.len() > 2);
        for &(_, y) in &verts[1..verts.len() - 1] {
            assert!(y > 0.0, "quad should bulge upward, got y={y}");
        }
    }

    #[test]
    fn test_cubic_curve_flattens() {
        let mut path = Path::new();
        path.move_to(0.0, 0.0);
        path.cubic_to((10.0, 20.0), (20.0, 20.0), (30.0, 0.0));
        let verts = path.as_polygon_vertices().unwrap();
        assert_eq!(verts.first().copied(), Some((0.0, 0.0)));
        assert_eq!(verts.last().copied(), Some((30.0, 0.0)));
        assert!(verts.len() > 2);
    }

    #[test]
    fn test_empty_and_degenerate_paths() {
        assert_eq!(Path::new().as_polygon_vertices(), None);
        let mut path = Path::new();
        path.move_to(0.0, 0.0);
        path.line_to(10.0, 0.0);
        assert_eq!(path.as_polygon_vertices(), None);
    }

    #[test]
    fn test_curve_without_current_point_moves() {
        let mut path = Path::new();
        path.quad_to((10.0, 10.0), (20.0, 20.0));
        assert_eq!(path.as_polygon_vertices(), None);
        assert_eq!(path.commands().len(), 1);
    }

    #[test]
    fn test_curve_bounding_box_includes_controls() {
        let mut path = Path::new();
        path.move_to(0.0, 0.0);
        path.quad_to((100.0, 0.0), (50.0, 50.0));
        let bb = path.bounding_box().unwrap();
        assert!((bb.x - 0.0).abs() < 1e-6);
        assert!((bb.width - 100.0).abs() < 1e-6);
        assert!((bb.y - 0.0).abs() < 1e-6);
        assert!((bb.height - 50.0).abs() < 1e-6);
    }

    #[test]
    fn test_transform_path_applies_affine_to_all_commands() {
        let mut path = Path::new();
        path.move_to(1.0, 1.0);
        path.cubic_to((2.0, 2.0), (3.0, 2.0), (4.0, 1.0));
        path.close();
        let transform =
            AffineTransform::translate(10.0, 20.0).then(&AffineTransform::scale(2.0, 2.0));
        let out = transform_path(&path, &transform);
        assert_eq!(
            out.commands(),
            &[
                PathCommand::MoveTo { x: 12.0, y: 22.0 },
                PathCommand::CubicTo {
                    c1x: 14.0,
                    c1y: 24.0,
                    c2x: 16.0,
                    c2y: 24.0,
                    x: 18.0,
                    y: 22.0,
                },
                PathCommand::Close,
            ]
        );
    }

    #[test]
    fn test_subpaths_split_on_move_to() {
        let mut path = Path::new();
        path.move_to(0.0, 0.0);
        path.line_to(10.0, 0.0);
        path.line_to(10.0, 10.0);
        path.close();
        path.move_to(20.0, 20.0);
        path.line_to(30.0, 20.0);
        path.line_to(30.0, 30.0);
        path.close();

        let rings = path.subpaths();
        assert_eq!(rings.len(), 2);
        assert_eq!(rings[0].len(), 3);
        assert_eq!(rings[1].len(), 3);
    }

    fn line(x1: f32, y1: f32, x2: f32, y2: f32) -> Path {
        let mut path = Path::new();
        path.move_to(x1, y1);
        path.line_to(x2, y2);
        path
    }

    fn closed_square(x: f32, y: f32, side: f32) -> Path {
        let mut path = Path::new();
        path.move_to(x, y);
        path.line_to(x + side, y);
        path.line_to(x + side, y + side);
        path.line_to(x, y + side);
        path.close();
        path
    }

    fn assert_point(a: (f32, f32), b: (f32, f32)) {
        assert!(
            (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3,
            "expected {b:?}, got {a:?}"
        );
    }

    /// Signed area via the shoelace formula, in y-down coordinates.
    fn signed_area(ring: &[(f32, f32)]) -> f32 {
        let mut sum = 0.0;
        for i in 0..ring.len() {
            let a = ring[i];
            let b = ring[(i + 1) % ring.len()];
            sum += a.0 * b.1 - b.0 * a.1;
        }
        sum * 0.5
    }

    #[test]
    fn test_stroke_open_line_is_a_rectangular_band() {
        // A width-2 horizontal line expands to the 1px-thick band around it.
        let stroked = stroke_path(&line(0.0, 0.0, 10.0, 0.0), &Stroke::new(2.0));
        let ring = &stroked.subpaths()[0];
        assert_eq!(ring.len(), 4);
        // Start cap: (-w side) -> (+w side); +w side forward; end cap back.
        assert_point(ring[0], (0.0, -1.0));
        assert_point(ring[1], (0.0, 1.0));
        assert_point(ring[2], (10.0, 1.0));
        assert_point(ring[3], (10.0, -1.0));
    }

    #[test]
    fn test_stroke_round_caps_bulge_past_the_endpoints() {
        let bob = Stroke {
            cap: StrokeCap::Round,
            ..Stroke::new(2.0)
        };
        let stroked = stroke_path(&line(0.0, 0.0, 10.0, 0.0), &bob);
        let bb = stroked.bounding_box().unwrap();
        assert!((bb.x - -1.0).abs() < 0.5, "round cap left extent: {}", bb.x);
        assert!(
            (bb.x + bb.width - 11.0).abs() < 0.5,
            "round cap right extent"
        );
    }

    #[test]
    fn test_stroke_square_caps_extend_by_half_width() {
        let bob = Stroke {
            cap: StrokeCap::Square,
            ..Stroke::new(2.0)
        };
        let stroked = stroke_path(&line(0.0, 0.0, 10.0, 0.0), &bob);
        let bb = stroked.bounding_box().unwrap();
        assert!((bb.x - -1.0).abs() < 1e-3);
        assert!((bb.x + bb.width - 11.0).abs() < 1e-3);
    }

    #[test]
    fn test_stroke_closed_square_is_a_winding_band() {
        // The stroked closed square is two rings with opposing winding, so the
        // band between the inner and outer offset fills under the nonzero rule.
        let stroked = stroke_path(&closed_square(0.0, 0.0, 10.0), &Stroke::new(2.0));
        let rings = stroked.subpaths();
        assert_eq!(rings.len(), 2, "expected an inner and an outer ring");
        let area0 = signed_area(&rings[0]);
        let area1 = signed_area(&rings[1]);
        assert!(
            area0 * area1 < 0.0,
            "rings must wind in opposite directions, got {area0} and {area1}"
        );
        // The stroke band extends half the width outside the square.
        let bb = stroked.bounding_box().unwrap();
        assert!((bb.x - -1.0).abs() < 1e-3);
        assert!((bb.y - -1.0).abs() < 1e-3);
        assert!((bb.width - 12.0).abs() < 1e-3);
        assert!((bb.height - 12.0).abs() < 1e-3);
    }

    #[test]
    fn test_stroke_miter_apex_sits_on_the_diagonal() {
        // At a right-angle corner (w = 1) an unlimited miter joins the two
        // offset edges through the 45° apex (11,-1); it is the only ring
        // vertex strictly inside the outer diagonal quadrant (x > 10, y < 0).
        let corner = closed_square(0.0, 0.0, 10.0);
        let mitered = stroke_path(&corner, &Stroke::new(2.0));
        let outer = &mitered.subpaths()[1];
        assert!(
            outer.iter().any(|&(px, py)| px > 10.0 + 1e-3 && py < -1e-3),
            "miter apex (11,-1) missing from the outer ring: {outer:?}"
        );
        // A bevel cuts straight across (10,-1) -> (11,0) and never reaches
        // into that quadrant, so its outline must not contain such a vertex.
        let bevel = stroke_path(
            &corner,
            &Stroke {
                join: StrokeJoin::Bevel,
                ..Stroke::new(2.0)
            },
        );
        for ring in bevel.subpaths() {
            assert!(
                !ring.iter().any(|&(px, py)| px > 10.0 + 1e-3 && py < -1e-3),
                "bevel must not contain a diagonal apex: {ring:?}"
            );
        }
    }

    #[test]
    fn test_stroke_miter_limit_collapses_sharp_joins() {
        // A near-straight path turns in a tiny angle whose miter apex would be
        // enormous; the default limit must reject it, matching bevel geometry.
        let sharp = line(0.0, 0.0, 10.0, 0.0);
        let mut sharp = sharp;
        sharp.line_to(20.0, 0.05);
        let mitered = stroke_path(&sharp, &Stroke::new(2.0));
        let bevel = stroke_path(
            &sharp,
            &Stroke {
                join: StrokeJoin::Bevel,
                ..Stroke::new(2.0)
            },
        );
        let mb = mitered.bounding_box().unwrap();
        let bv = bevel.bounding_box().unwrap();
        assert!(
            (mb.width - bv.width).abs() < 0.1,
            "the clamped miter must not poke beyond the bevel"
        );
    }

    #[test]
    fn test_stroke_dash_pattern_splits_into_segments() {
        // Dash 4, gap 2 over a 10px line yields segments [0,4] and [6,10].
        let dashed = stroke_path(
            &line(0.0, 0.0, 10.0, 0.0),
            &Stroke {
                dash: Some(vec![4.0, 2.0]),
                ..Stroke::new(2.0)
            },
        );
        let rings = dashed.subpaths();
        assert_eq!(rings.len(), 2, "expected two dash segments");
        let x0 = rings[0][0].0;
        let x1 = rings[1][0].0;
        let starts = [x0.min(x1), x0.max(x1)];
        assert!((starts[0] - 0.0).abs() < 1e-3, "first dash starts at 0");
        assert!((starts[1] - 6.0).abs() < 1e-3, "second dash starts at 6");
    }

    #[test]
    fn test_stroke_round_join_uses_an_arc_not_a_point() {
        // Round joins interpolate curved corners; miter joins may be a single
        // apex. A round-joined closed triangle produces more commands than a
        // bevel-joined one because the arcs are cubic segments.
        let mut tri = Path::new();
        tri.move_to(0.0, 0.0);
        tri.line_to(20.0, 0.0);
        tri.line_to(10.0, 20.0);
        tri.close();
        let round = stroke_path(
            &tri,
            &Stroke {
                join: StrokeJoin::Round,
                cap: StrokeCap::Round,
                ..Stroke::new(4.0)
            },
        );
        let bevel = stroke_path(
            &tri,
            &Stroke {
                join: StrokeJoin::Bevel,
                ..Stroke::new(4.0)
            },
        );
        assert!(
            round.commands().len() > bevel.commands().len(),
            "round joins must add arc commands"
        );
    }
}
