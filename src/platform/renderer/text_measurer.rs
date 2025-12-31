use crate::engine::bridge::text::{
    TextMeasureError, TextMeasurement, TextMeasurementRequest, TextMeasurer,
};

use std::env;
use std::sync::{Arc, Mutex};

use glyphon::{Attrs, Buffer, Color as GlyphColor, FontSystem, Metrics, Shaping};

/// テキスト計測のプラットフォーム側実装
pub struct PlatformTextMeasurer {
    // font system used for shaping/metrics via cosmic-text (glyphon re-exports)
    font_sys: Mutex<FontSystem>,
}

impl PlatformTextMeasurer {
    /// システムフォントから初期化を試みる
    ///
    /// TODO:
    /// - PlatformTextRenderer とfontの共有化
    /// - フォント選択機能を追加
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

    /// バイト列からフォントを読み込んで初期化
    pub fn from_bytes(_id: &str, bytes: Vec<u8>) -> Result<Self, Box<dyn std::error::Error>> {
        let font_source = Arc::new(bytes);
        let font = glyphon::fontdb::Source::Binary(font_source);
        let font_sys = FontSystem::new_with_fonts(vec![font]);
        Ok(Self {
            font_sys: Mutex::new(font_sys),
        })
    }
}

impl TextMeasurer for PlatformTextMeasurer {
    /// テキスト計測を行う
    ///
    /// TODO
    /// - Baselineの計算
    fn measure(&self, req: &TextMeasurementRequest) -> Result<TextMeasurement, TextMeasureError> {
        let font_size = req.font.size_px.max(1.0);

        // create a glyphon/cosmic-text Buffer to shape & layout
        let mut fs = self
            .font_sys
            .lock()
            .map_err(|e| TextMeasureError::Internal(format!("font_sys lock poisoned: {}", e)))?;

        let metrics = Metrics::relative(font_size, 1.2);
        let mut buffer = Buffer::new(&mut fs, metrics);

        // attrs: only metrics needed for layout here
        let attrs = Attrs::new()
            .metrics(metrics)
            .color(GlyphColor::rgba(0, 0, 0, 255));

        buffer.set_text(&mut fs, &req.text, &attrs, Shaping::Advanced, None);

        // compute width and height from layout using Buffer::line_layout()
        let mut max_width: f32 = 0.0;
        let mut lines: usize = 0;

        // iterate over buffer lines (paragraphs) and accumulate their laid-out lines
        for line_i in 0..buffer.lines.len() {
            if let Some(layout_lines) = buffer.line_layout(&mut fs, line_i) {
                for ll in layout_lines.iter() {
                    max_width = max_width.max(ll.w);
                    lines += 1;
                }
            }
        }

        if lines == 0 {
            // empty text
            let line_height = metrics.line_height;
            return Ok(TextMeasurement {
                width: 0.0,
                height: line_height,
                baseline: font_size * 0.8,
                glyphs: None,
            });
        }

        // handle wrap / max_lines constraint
        if let Some(max_lines) = req.constraints.max_lines
            && lines > max_lines
        {
            lines = max_lines;
        }

        let line_height = metrics.line_height;
        let height = lines as f32 * line_height;
        Ok(TextMeasurement {
            width: max_width,
            height,
            baseline: font_size * 0.8,
            glyphs: None,
        })
    }
}
