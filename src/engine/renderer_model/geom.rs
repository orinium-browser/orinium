//! Geometry primitives shared across the render model and the renderer.

/// An affine transformation matrix (2D), in row-major convention with an
/// implicit translation.
#[derive(Debug, Clone, Copy)]
pub struct AffineTransform {
    pub m11: f32,
    pub m12: f32,
    pub m21: f32,
    pub m22: f32,
    pub dx: f32,
    pub dy: f32,
}

impl AffineTransform {
    /// The identity transform.
    pub const fn identity() -> Self {
        AffineTransform {
            m11: 1.0,
            m12: 0.0,
            m21: 0.0,
            m22: 1.0,
            dx: 0.0,
            dy: 0.0,
        }
    }

    /// A pure translation transform.
    pub fn translate(dx: f32, dy: f32) -> Self {
        AffineTransform {
            m11: 1.0,
            m12: 0.0,
            m21: 0.0,
            m22: 1.0,
            dx,
            dy,
        }
    }

    /// A pure scale transform.
    pub const fn scale(sx: f32, sy: f32) -> Self {
        AffineTransform {
            m11: sx,
            m12: 0.0,
            m21: 0.0,
            m22: sy,
            dx: 0.0,
            dy: 0.0,
        }
    }

    /// A rotation transform by `angle` radians.
    pub fn rotate(angle: f32) -> Self {
        let c = angle.cos();
        let s = angle.sin();
        AffineTransform {
            m11: c,
            m12: -s,
            m21: s,
            m22: c,
            dx: 0.0,
            dy: 0.0,
        }
    }

    /// Apply this transform to a point.
    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            x * self.m11 + y * self.m12 + self.dx,
            x * self.m21 + y * self.m22 + self.dy,
        )
    }

    /// Compose: `self` after `rhs` (rhs is applied first, then self).
    pub fn then(&self, rhs: &AffineTransform) -> Self {
        AffineTransform {
            m11: self.m11 * rhs.m11 + self.m12 * rhs.m21,
            m12: self.m11 * rhs.m12 + self.m12 * rhs.m22,
            m21: self.m21 * rhs.m11 + self.m22 * rhs.m21,
            m22: self.m21 * rhs.m12 + self.m22 * rhs.m22,
            dx: self.m11 * rhs.dx + self.m12 * rhs.dy + self.dx,
            dy: self.m21 * rhs.dx + self.m22 * rhs.dy + self.dy,
        }
    }

    /// Returns the inverse transform, or `None` when the matrix is singular.
    pub fn inverse(&self) -> Option<Self> {
        let det = self.m11 * self.m22 - self.m12 * self.m21;
        if det.abs() < f32::EPSILON {
            return None;
        }
        let inv_det = 1.0 / det;
        Some(AffineTransform {
            m11: self.m22 * inv_det,
            m12: -self.m12 * inv_det,
            m21: -self.m21 * inv_det,
            m22: self.m11 * inv_det,
            dx: (self.m21 * self.dy - self.m22 * self.dx) * inv_det,
            dy: (self.m12 * self.dx - self.m11 * self.dy) * inv_det,
        })
    }
}

/// An axis-aligned rectangle.
#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    /// Creates a new rectangle.
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Returns `true` if the rectangle contains the given point.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }

    /// Returns the intersection of two rectangles, or `None` if they do not
    /// overlap.
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = (self.x + self.width).min(other.x + other.width);
        let y2 = (self.y + self.height).min(other.y + other.height);
        if x2 > x1 && y2 > y1 {
            Some(Rect::new(x1, y1, x2 - x1, y2 - y1))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_affine_transform_composition_and_inverse() {
        let t = AffineTransform::translate(3.0, 4.0).then(&AffineTransform::translate(5.0, 6.0));
        assert_eq!(t.apply(1.0, 1.0), (9.0, 11.0));

        let inv = t.inverse().unwrap();
        let (x, y) = t.apply(10.0, 20.0);
        let (rx, ry) = inv.apply(x, y);
        assert!((rx - 10.0).abs() < 1e-5, "rx={rx}");
        assert!((ry - 20.0).abs() < 1e-5, "ry={ry}");
    }

    #[test]
    fn test_affine_transform_scale_rotate() {
        // scale.then(rotate) applies rotate first, then scale.
        let t = AffineTransform::scale(2.0, 3.0)
            .then(&AffineTransform::rotate(std::f32::consts::FRAC_PI_2));
        let (x, y) = t.apply(1.0, 0.0);
        assert!((x - 0.0).abs() < 1e-5, "x={x}");
        assert!((y - 3.0).abs() < 1e-5, "y={y}");
    }

    #[test]
    fn test_rect_intersect() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        let i = a.intersect(&b).unwrap();
        assert!((i.x - 5.0).abs() < 1e-6);
        assert!((i.width - 5.0).abs() < 1e-6);
        assert!(Rect::new(20.0, 20.0, 1.0, 1.0).intersect(&a).is_none());
    }
}
