//! Path model for the renderer: a sequence of move/line/curve commands with
//! helpers for bounding boxes and polygon conversion for GPU rasterization.

use crate::engine::renderer_model::geom::Rect;

/// A single path drawing command.
#[derive(Debug, Clone)]
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
}
