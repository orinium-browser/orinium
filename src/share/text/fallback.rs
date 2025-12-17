use crate::share::text::{TextMeasureError, TextMeasurement, TextMeasurementRequest, TextMeasurer};

// エンジンのフォールバックテキスト計測器
pub struct EngineFallbackTextMeasurer {
    pub avg_char_width_ratio: f32,
}

// デフォルト実装
impl Default for EngineFallbackTextMeasurer {
    fn default() -> Self {
        Self {
            avg_char_width_ratio: 0.5,
        }
    }
}

impl TextMeasurer for EngineFallbackTextMeasurer {
    /// テキスト計測を行う
    fn measure(&self, req: &TextMeasurementRequest) -> Result<TextMeasurement, TextMeasureError> {
        let length = req.text.chars().count() as f32;
        let char_w = req.font.size_px * self.avg_char_width_ratio;
        let mut width = length * char_w;
        let line_height = req.font.size_px * 1.0;
        let mut lines = 1.0;

        if req.constraints.wrap {
            if let Some(max_w) = req.constraints.max_width {
                if max_w > 0.0 {
                    lines = (width / max_w).ceil().max(1.0);
                    width = max_w.min(width);
                }
            }
        }

        if let Some(max_lines) = req.constraints.max_lines {
            if lines as usize > max_lines {
                lines = max_lines as f32;
            }
        }

        let height = lines * line_height;
        Ok(TextMeasurement {
            width,
            height,
            baseline: req.font.size_px * 0.8,
            glyphs: None,
        })
    }
}

// ここにテスト書いてるけど許せ
#[cfg(test)]
mod tests {
    use crate::share::text::{FontDescription, LayoutConstraints};
    use super::*;

    #[test]
    fn fallback_measure_simple() {
        let measurer = EngineFallbackTextMeasurer::default();
        let req = TextMeasurementRequest {
            text: "abc".to_string(),
            font: FontDescription { family: None, size_px: 10.0 },
            constraints: LayoutConstraints { max_width: None, wrap: false, max_lines: None },
        };

        let res = measurer.measure(&req).expect("measurement should succeed");
        let expected_width = 3.0 * 10.0 * measurer.avg_char_width_ratio; // 3 chars * font * ratio
        let expected_height = 10.0_f32;
        assert!((res.width - expected_width).abs() < 1e-6, "width mismatch");
        assert!((res.height - expected_height).abs() < 1e-6, "height mismatch");
    }
}
