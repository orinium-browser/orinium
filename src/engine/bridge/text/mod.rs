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
 * Style Type
 * ============================ */

#[derive(Debug, Clone)]
pub struct TextAttribute {
    pub style: TextStyle,
    pub flow_style: TextFlowStyle,
}

/* ============================
 * Measure Request
 * ============================ */

#[derive(Debug, Clone)]
pub struct TextMeasureRequest {
    /// UTF-8 text content
    pub text: String,

    /// Opaque, resolved text attributes provided by the caller
    pub attribute: TextAttribute,
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
 * Glyph Cluster (for FlowLayouter)
 * ============================ */

/// A single glyph cluster produced by text shaping.
///
/// Carries the cluster's byte offset in the original text, its advance
/// width, and whether a line break is permitted after it.
#[derive(Debug, Clone)]
pub struct GlyphCluster {
    /// Byte offset of this cluster's first character in the original text.
    pub byte_offset: usize,
    /// Advance width in pixels.
    pub width: f32,
    /// Whether a line break is permitted immediately after this cluster.
    pub break_allowed: bool,
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

pub trait TextMeasurer: Send + Sync {
    /// Measure a single block of text and return its metrics.
    fn measure(
        &self,
        request: &TextMeasureRequest,
    ) -> Result<Vec<MeasuredFragment>, TextMeasureError>;

    /// Shape text and return cluster-level break-opportunity data.
    ///
    /// Unlike [`measure`](Self::measure), this returns per-cluster
    /// data suitable for use with [`TextFlowLayouter`].
    fn measure_shaped(
        &self,
        request: &TextMeasureRequest,
    ) -> Result<Vec<GlyphCluster>, TextMeasureError>;
}

/* ============================
 * Fallback
 * ============================ */

pub mod fallback;
pub use fallback::FallbackTextMeasurer;

use crate::engine::layouter::types::{TextFlowStyle, TextStyle};
