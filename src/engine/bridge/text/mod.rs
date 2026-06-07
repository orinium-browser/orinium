//! Text measurement abstraction for layout and rendering.
//!
//! # Overview
//!
//! This module defines the interface between the layout engine and
//! platform-specific text measurement implementations.
//!
//! It does **not** own or define visual text styles.
//! Instead, it consumes already-resolved text attributes provided
//! by higher-level layout or rendering layers.
//!
//! # Responsibilities
//!
//! - Accept text content and layout-related parameters
//! - Measure intrinsic text size (width, height, baseline)
//! - Provide a backend-agnostic text measurement abstraction
//!
//! # Non-Responsibilities
//!
//! - CSS resolution or inheritance
//! - Interpretation of visual styling semantics
//! - Rendering or draw command generation
//!
//! # Data Flow
//!
//! ```text
//! CSS → Layout → TextMeasurer → TextMetrics
//! ```

use std::fmt;

/* ============================
 * Measure Request
 * ============================ */

#[derive(Debug, Clone)]
pub struct TextMeasureRequest<S> {
    /// UTF-8 text content
    pub text: String,

    /// Opaque, resolved text attributes provided by the caller
    pub style: S,
}

/* ============================
 * Measured Result
 * ============================ */

/// A measured text fragment produced by [`TextMeasurer::measure_fragments`].
///
/// Contains the original text segment along with its measured dimensions,
/// so callers can both retrieve the split text and obtain fragment widths
/// for inline layout.
#[derive(Debug, Clone)]
pub struct MeasuredFragment {
    pub text: String,
    pub width: f32,
    pub height: f32,
}

/* ============================
 * Optional Glyph Info (Future)
 * ============================ */

#[derive(Debug, Clone)]
pub struct GlyphMetrics {
    pub glyph_id: u32,
    pub x: f32,
    pub y: f32,
    pub advance: f32,
}

/* ============================
 * Errors
 * ============================ */

#[derive(Debug)]
pub enum TextMeasureError {
    FontUnavailable,
    UnsupportedScript,
    LayoutOverflow,
    Internal(String),
}

impl fmt::Display for TextMeasureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FontUnavailable => write!(f, "Font unavailable"),
            Self::UnsupportedScript => write!(f, "Unsupported script"),
            Self::LayoutOverflow => write!(f, "Layout overflow"),
            Self::Internal(s) => write!(f, "Internal error: {s}"),
        }
    }
}

impl std::error::Error for TextMeasureError {}

/* ============================
 * Trait
 * ============================ */

pub trait TextMeasurer<S>: Send + Sync {
    /// Measure a single block of text and return its metrics.
    fn measure(
        &self,
        request: &TextMeasureRequest<S>,
    ) -> Result<Vec<MeasuredFragment>, TextMeasureError>;
}

/* ============================
 * Fallback
 * ============================ */

pub mod fallback;
pub use fallback::FallbackTextMeasurer;
