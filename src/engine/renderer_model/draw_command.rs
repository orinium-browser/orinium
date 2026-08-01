//! Draw command model: the rendering instructions produced from layout.
//!
//! The geometry primitives live in [`crate::engine::renderer_model::geom`],
//! paths in [`crate::engine::renderer_model::path`], and the layout → command
//! generation in [`crate::engine::renderer_model::box_model`].

use smol_str::SmolStr;

use crate::engine::layouter::types::{Color, Gradient, TextStyle};
use crate::engine::renderer_model::geom::{AffineTransform, Rect};
use crate::engine::renderer_model::path::Path;

/// Fill rule for path filling.
///
/// The GPU rasterizer currently only uses this to select the winding mode of
/// the ear-clipping triangulation; a fully correct stencil-based fill for
/// arbitrary self-intersecting paths is not implemented.
#[derive(Debug, Clone, Copy)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

/// A fill source: a solid color or a gradient.
#[derive(Debug, Clone)]
pub enum Brush {
    Solid(Color),
    Gradient(Gradient),
}

/// How a path is painted: the brush plus an opacity multiplier.
#[derive(Debug, Clone)]
pub struct Paint {
    pub brush: Brush,
    pub opacity: f32,
}

/// A drawing instruction for the GPU renderer.
#[derive(Debug, Clone)]
pub enum DrawCommand {
    /// Fill a path.
    ///
    /// The `opacity` is applied to solid color fills.
    Fill {
        path: Path,
        paint: Paint,
        rule: FillRule,
    },
    /// Draw a text run at `(x, y)`.
    DrawText {
        x: f32,
        y: f32,
        text: SmolStr,
        style: TextStyle,
    },
    /// Push a clip region given by a path.
    ///
    /// Non-rectangular paths are approximated by their bounding box.
    PushClip {
        path: Path,
        rule: FillRule,
    },
    PopClip,
    /// Push a coordinate transform.
    PushTransform {
        transform: AffineTransform,
    },
    PopTransform,

    /// Delegate rendering to a platform-native system UI element.
    ///
    /// The renderer composites or renders the element identified by
    /// [`SystemUiKind`] at the given rectangle within the current
    /// coordinate space.
    SystemUi {
        kind: SystemUiKind,
        rect: Rect,
    },
}

/// Discriminator for [`DrawCommand::SystemUi`].
/// Stub
#[derive(Debug, Clone)]
pub enum SystemUiKind {
    /// Composite an external surface (WebView, iframe, …).
    WebView { surface_id: usize },
    /// Render a platform-native input widget.
    Input {
        value: SmolStr,
        placeholder: SmolStr,
    },
}
