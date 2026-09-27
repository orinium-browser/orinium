//! Fallback text measurer without font engine dependency.

use super::{GlyphCluster, TextMeasureError, TextMeasureRequest, TextMeasurer};
use crate::engine::bridge::text::MeasuredFragment;

/// Fallback text measurer.
///
/// This implementation does not rely on any font engine.
/// It uses a simple heuristic based on font size and character count.
/// Intended for testing, bring-up, and environments without font support.
#[derive(Debug, Default)]
pub struct FallbackTextMeasurer;

impl TextMeasurer for FallbackTextMeasurer {
    fn measure(
        &self,
        request: &TextMeasureRequest,
    ) -> Result<Vec<MeasuredFragment>, TextMeasureError> {
        let font_size = request.attribute.flow_style.font_size.max(1.0);

        // Heuristic constants
        let char_width = font_size * 0.6;
        let line_height = font_size * 1.2;

        let fragments: Vec<MeasuredFragment> = request
            .text
            .split('\n')
            .map(|line| {
                let w = line.len() as f32 * char_width;
                MeasuredFragment {
                    text: line.to_string(),
                    width: w,
                    height: line_height,
                }
            })
            .collect();

        Ok(fragments)
    }

    fn measure_shaped(
        &self,
        request: &TextMeasureRequest,
    ) -> Result<Vec<GlyphCluster>, TextMeasureError> {
        let font_size = request.attribute.flow_style.font_size.max(1.0);
        let char_width = font_size * 0.6;

        let mut clusters = Vec::new();
        let text = &request.text;
        let mut i = 0;

        while i < text.len() {
            let ch = text[i..].chars().next().unwrap();
            let byte_len = ch.len_utf8();
            let is_space = ch.is_whitespace();
            let w = char_width;

            clusters.push(GlyphCluster {
                byte_offset: i,
                width: w,
                break_allowed: is_space,
            });

            i += byte_len;
        }

        Ok(clusters)
    }
}
