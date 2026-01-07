use super::{TextMeasureError, TextMeasureRequest, TextMeasurer, TextMetrics};
use crate::engine::layouter::TextStyle;

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
    ) -> Result<TextMetrics, TextMeasureError> {
        let font_size = request.style.font_size.max(1.0);

        // Heuristic constants
        let char_width = font_size * 0.6;
        let line_height = font_size * 1.2;

        let mut current_line_width = 0.0;
        let mut max_line_width: f32 = 0.0;
        let mut line_count = 1;

        for ch in request.text.chars() {
            if ch == '\n' {
                max_line_width = max_line_width.max(current_line_width);
                current_line_width = 0.0;
                line_count += 1;
                continue;
            }

            current_line_width += char_width;

            if request.wrap
                && let Some(max_width) = request.max_width
                && current_line_width > max_width
            {
                max_line_width = max_line_width.max(current_line_width - char_width);
                current_line_width = char_width;
                line_count += 1;
            }
        }

        max_line_width = max_line_width.max(current_line_width);

        Ok(TextMetrics {
            width: max_line_width,
            height: line_height * line_count as f32,
            baseline: font_size,
            line_count,
        })
    }
}
