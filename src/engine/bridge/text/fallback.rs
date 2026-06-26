//! Fallback text measurer without font engine dependency.

use super::{TextMeasureError, TextMeasureRequest, TextMeasurer};
use crate::engine::{bridge::text::MeasuredFragment, layouter::types::TextStyle};

/// Fallback text measurer.
///
/// This implementation does not rely on any font engine.
/// It uses a simple heuristic based on font size and character count.
/// Intended for testing, bring-up, and environments without font support.
#[derive(Debug, Default)]
pub struct FallbackTextMeasurer;

impl TextMeasurer<TextStyle> for FallbackTextMeasurer {
    fn measure(
        &self,
        request: &TextMeasureRequest<TextStyle>,
    ) -> Result<Vec<MeasuredFragment>, TextMeasureError> {
        let font_size = request.style.font_size.max(1.0);

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
}
