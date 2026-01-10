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

    /// Maximum line width (None = unconstrained)
    pub max_width: Option<f32>,

    /// Enable line wrapping
    pub wrap: bool,
}

/* ============================
 * Measure Result
 * ============================ */

#[derive(Debug, Clone)]
pub struct TextMetrics {
    /// Logical width
    pub width: f32,

    /// Logical height
    pub height: f32,

    /// Baseline position from top
    pub baseline: f32,

    /// Number of layouted lines
    pub line_count: usize,
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
    fn measure(&self, request: &TextMeasureRequest<S>) -> Result<TextMetrics, TextMeasureError>;
}

/* ============================
 * Fallback
 * ============================ */

pub mod fallback;
pub use fallback::FallbackTextMeasurer;
