use crate::engine::bridge::text::{
    TextMeasureError, TextMeasureRequest, TextMeasurer, TextMetrics,
};
use crate::engine::layouter::TextStyle;

use std::env;
use std::sync::{Arc, Mutex};

use glyphon::{Attrs, Buffer, Color as GlyphColor, FontSystem, Metrics, Shaping};

/// Platform-backed text measurer using glyphon / cosmic-text.
///
/// This measurer performs real text shaping and line layout,
/// and is intended for production use.
pub struct PlatformTextMeasurer {
    /// Font system used for shaping and metrics
    font_sys: Mutex<FontSystem>,
}

impl PlatformTextMeasurer {
    /// Initialize using system fonts.
    ///
    /// TODO:
    /// - Share font system with PlatformTextRenderer
    /// - Support font family / fallback selection
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let mut maybe_bytes: Option<Vec<u8>> = None;

        if let Ok(p) = env::var("ORINIUM_FONT")
            && let Ok(b) = std::fs::read(&p)
        {
            maybe_bytes = Some(b);
        }

        if maybe_bytes.is_none() {
            for p in crate::platform::font::system_font_candidates()? {
                if let Ok(b) = std::fs::read(p) {
                    maybe_bytes = Some(b);
                    break;
                }
            }
        }

        if let Some(bytes) = maybe_bytes {
            let font_source = Arc::new(bytes);
            let font = glyphon::fontdb::Source::Binary(font_source);
            let font_sys = FontSystem::new_with_fonts(vec![font]);

            return Ok(Self {
                font_sys: Mutex::new(font_sys),
            });
        }

        Err("no system font found".into())
    }

    /// Initialize from raw font bytes.
    pub fn from_bytes(_id: &str, bytes: Vec<u8>) -> Result<Self, Box<dyn std::error::Error>> {
        let font_source = Arc::new(bytes);
        let font = glyphon::fontdb::Source::Binary(font_source);
        let font_sys = FontSystem::new_with_fonts(vec![font]);

        Ok(Self {
            font_sys: Mutex::new(font_sys),
        })
    }
}

impl TextMeasurer<TextStyle> for PlatformTextMeasurer {
    /// Measure text using real shaping and line layout.
    ///
    /// Notes:
    /// - Baseline is currently approximated
    /// - Decorations and alignment are handled at render time
    fn measure(
        &self,
        req: &TextMeasureRequest<TextStyle>,
    ) -> Result<TextMetrics, TextMeasureError> {
        let font_size = req.style.font_size.max(1.0);

        let mut fs = self
            .font_sys
            .lock()
            .map_err(|e| TextMeasureError::Internal(format!("font_sys lock poisoned: {}", e)))?;

        // glyphon metrics: font size + line height
        let metrics = Metrics::relative(font_size, 1.2);
        let mut buffer = Buffer::new(&mut fs, metrics);

        // Attributes used only for shaping / layout
        let attrs = Attrs::new()
            .metrics(metrics)
            .color(GlyphColor::rgba(0, 0, 0, 255));

        buffer.set_text(&mut fs, &req.text, &attrs, Shaping::Advanced, None);

        let mut max_width: f32 = 0.0;
        let mut line_count: usize = 0;

        // Iterate over shaped lines
        for para_i in 0..buffer.lines.len() {
            if let Some(layout_lines) = buffer.line_layout(&mut fs, para_i) {
                for line in layout_lines {
                    max_width = max_width.max(line.w);
                    line_count += 1;
                }
            }
        }

        if line_count == 0 {
            // Empty text
            return Ok(TextMetrics {
                width: 0.0,
                height: metrics.line_height,
                baseline: font_size * 0.8,
                line_count: 1,
            });
        }

        // Apply wrapping constraint
        if let Some(max_width_limit) = req.max_width {
            max_width = max_width.min(max_width_limit);
        }

        let height = metrics.line_height * line_count as f32;

        Ok(TextMetrics {
            width: max_width,
            height,
            baseline: font_size * 0.8, // TODO: precise baseline from font metrics
            line_count,
        })
    }
}
