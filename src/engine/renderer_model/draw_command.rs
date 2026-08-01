//! Draw command model: the rendering instructions produced from layout.
//!
//! The geometry primitives live in [`crate::engine::renderer_model::geom`],
//! paths in [`crate::engine::renderer_model::path`], and the layout → command
//! generation in [`crate::engine::renderer_model::box_model`].

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
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
    /// A decoded RGBA image sampled across the fill path's bounds.
    Image(Image),
}

/// Decoded image pixels shared between the engine and platform renderer.
#[derive(Debug, Clone)]
pub struct Image {
    id: u64,
    width: u32,
    height: u32,
    rgba: Arc<[u8]>,
}

impl Image {
    /// Decodes encoded image bytes into RGBA8 pixels.
    ///
    /// Returns an error when the bytes are not a supported image format.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let decoded = image::load_from_memory(bytes).context("failed to decode image")?;
        let rgba = decoded.to_rgba8();
        let (width, height) = rgba.dimensions();
        Self::from_rgba(width, height, rgba.into_raw())
    }

    /// Creates an image from decoded RGBA8 pixels.
    ///
    /// Returns an error when the byte length does not match the dimensions.
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self> {
        static NEXT_IMAGE_ID: AtomicU64 = AtomicU64::new(1);

        let expected_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .context("image dimensions exceed addressable memory")?;
        if rgba.len() != expected_len {
            anyhow::bail!(
                "invalid RGBA byte length: expected {expected_len}, got {}",
                rgba.len()
            );
        }
        Ok(Self {
            id: NEXT_IMAGE_ID.fetch_add(1, Ordering::Relaxed),
            width,
            height,
            rgba: Arc::from(rgba),
        })
    }

    /// Returns the renderer-unique image identifier.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Returns the decoded image width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Returns the decoded image height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Returns the decoded RGBA8 pixel bytes.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
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
    /// Draw single-line input text and decorations using shaped text widths.
    DrawTextInput {
        x: f32,
        y: f32,
        text: SmolStr,
        style: TextStyle,
        /// UTF-8 byte offset at which to draw the caret.
        caret: Option<usize>,
        /// UTF-8 byte range occupied by the active IME preedit text.
        preedit: Option<(usize, usize)>,
        decoration_color: Color,
        caret_top: f32,
        caret_height: f32,
        underline_y: f32,
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
