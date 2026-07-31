//! Path model for the renderer: a sequence of move/line/curve commands with
//! helpers for bounding boxes and polygon conversion for GPU rasterization.

use crate::engine::renderer_model::draw_command::Rect;

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
    /// Returns the path as a list of vertices, flattening curved segments
    /// into line segments, or `None` if the path has fewer than three
    /// vertices.
    ///
    /// The GPU rasterizer expects a single, roughly convex subpath; other
    /// paths (multiple subpaths or concave shapes) may be rendered
    /// incorrectly.
    pub fn as_polygon_vertices(&self) -> Option<Vec<(f32, f32)>> {
        let mut vertices = Vec::new();
        let mut current: Option<(f32, f32)> = None;

        for cmd in &self.commands {
            match cmd {
                PathCommand::MoveTo { x, y } | PathCommand::LineTo { x, y } => {
                    vertices.push((*x, *y));
                    current = Some((*x, *y));
                }
                PathCommand::QuadTo { cx, cy, x, y } => {
                    if let Some(p0) = current {
                        flatten_quad(p0, (*cx, *cy), (*x, *y), &mut vertices);
                    } else {
                        vertices.push((*x, *y));
                    }
                    current = Some((*x, *y));
                }
                PathCommand::CubicTo {
                    c1x,
                    c1y,
                    c2x,
                    c2y,
                    x,
                    y,
                } => {
                    if let Some(p0) = current {
                        flatten_cubic(p0, (*c1x, *c1y), (*c2x, *c2y), (*x, *y), &mut vertices);
                    } else {
                        vertices.push((*x, *y));
                    }
                    current = Some((*x, *y));
                }
                PathCommand::Close => {}
            }
        }

        if vertices.len() < 3 {
            None
        } else {
            Some(vertices)
        }
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
