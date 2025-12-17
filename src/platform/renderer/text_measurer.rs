use crate::engine::share::text::{
    TextMeasureError, TextMeasurement, TextMeasurementRequest, TextMeasurer,
};
use fontdue::Font as FontDue;
use std::collections::HashMap;
use std::env;
use std::path::Path;

/// テキスト計測のプラットフォーム側実装
pub struct PlatformTextMeasurer {
    fonts: HashMap<String, FontDue>,
    default_font: String,
}

impl PlatformTextMeasurer {
    /// システムフォントから初期化を試みる
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let candidates = [
            "C:\\Windows\\Fonts\\meiryo.ttc",
            "C:\\Windows\\Fonts\\msgothic.ttc",
            "C:\\Windows\\Fonts\\msmincho.ttc",
            "C:\\Windows\\Fonts\\arial.ttf",
            "C:\\Windows\\Fonts\\segoeui.ttf",
        ];

        // 読み込んだフォントキャッシュ
        let mut fonts: HashMap<String, FontDue> = HashMap::new();

        // もし環境変数あるならそっちのフォントを優先
        if let Ok(p) = env::var("ORINIUM_FONT") {
            if let Ok(bytes) = std::fs::read(&p) {
                let font = FontDue::from_bytes(&bytes[..], fontdue::FontSettings::default())?;
                fonts.insert("default".to_string(), font);
                return Ok(Self {
                    fonts,
                    default_font: "default".to_string(),
                });
            }
        }

        for p in &candidates {
            if Path::new(p).exists() {
                if let Ok(bytes) = std::fs::read(p) {
                    let font = FontDue::from_bytes(&bytes[..], fontdue::FontSettings::default())?;
                    fonts.insert("default".to_string(), font);
                    return Ok(Self {
                        fonts,
                        default_font: "default".to_string(),
                    });
                }
            }
        }

        Err("no system font found".into())
    }

    /// バイト列からフォントを読み込んで初期化
    pub fn from_bytes(id: &str, bytes: Vec<u8>) -> Result<Self, Box<dyn std::error::Error>> {
        let font = FontDue::from_bytes(&bytes[..], fontdue::FontSettings::default())?;
        let mut fonts = HashMap::new();
        fonts.insert(id.to_string(), font);
        Ok(Self {
            fonts,
            default_font: id.to_string(),
        })
    }
}

impl TextMeasurer for PlatformTextMeasurer {
    /// テキスト計測を行う
    fn measure(&self, req: &TextMeasurementRequest) -> Result<TextMeasurement, TextMeasureError> {
        let font = self
            .fonts
            .get(&self.default_font)
            .ok_or_else(|| TextMeasureError::FontNotFound(self.default_font.clone()))?;
        let font_size = req.font.size_px.max(1.0);

        let mut advances: Vec<f32> = Vec::new();
        for ch in req.text.chars() {
            let (metrics, _bitmap) = font.rasterize(ch, font_size);
            advances.push(metrics.advance_width);
        }

        let line_height = font_size * 1.2;

        if advances.is_empty() {
            return Ok(TextMeasurement {
                width: 0.0,
                height: line_height,
                baseline: font_size * 0.8,
                glyphs: None,
            });
        }

        let mut max_width: f32 = 0.0;
        let mut cur_width: f32 = 0.0;
        let mut lines: usize = 1;

        if req.constraints.wrap {
            if let Some(mw) = req.constraints.max_width {
                for a in advances.iter() {
                    if cur_width + a > mw && cur_width > 0.0 {
                        max_width = max_width.max(cur_width);
                        cur_width = *a;
                        lines += 1;
                    } else {
                        cur_width += a;
                    }
                }
                max_width = max_width.max(cur_width);
            } else {
                for a in advances.iter() {
                    cur_width += a;
                }
                max_width = cur_width;
            }
        } else {
            for a in advances.iter() {
                cur_width += a;
            }
            max_width = cur_width;
        }

        if let Some(max_lines) = req.constraints.max_lines {
            if lines > max_lines {
                lines = max_lines;
            }
        }

        let height = lines as f32 * line_height;
        Ok(TextMeasurement {
            width: max_width,
            height,
            baseline: font_size * 0.8,
            glyphs: None,
        })
    }
}
