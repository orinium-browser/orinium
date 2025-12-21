use crate::engine::bridge::text::{
    TextMeasureError, TextMeasurement, TextMeasurementRequest, TextMeasurer,
};
use fontdue::Font as FontDue;
use std::collections::HashMap;
use std::env;

/// テキスト計測のプラットフォーム側実装
pub struct PlatformTextMeasurer {
    fonts: HashMap<String, FontDue>,
    default_font: String,
}

impl PlatformTextMeasurer {
    /// システムフォントから初期化を試みる
    ///
    /// TODO:
    /// - PlatformTextRenderer とfontの共有化
    /// - フォント選択機能を追加
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        // 読み込んだフォントキャッシュ
        let mut fonts: HashMap<String, FontDue> = HashMap::new();

        // もし環境変数あるならそっちのフォントを優先
        if let Ok(p) = env::var("ORINIUM_FONT")
            && let Ok(bytes) = std::fs::read(&p)
        {
            let font = FontDue::from_bytes(&bytes[..], fontdue::FontSettings::default())?;
            fonts.insert("default".to_string(), font);
            return Ok(Self {
                fonts,
                default_font: "default".to_string(),
            });
        }

        for p in crate::platform::font::system_font_candidates()? {
            if let Ok(bytes) = std::fs::read(p) {
                let font = FontDue::from_bytes(&bytes[..], fontdue::FontSettings::default())?;
                fonts.insert("default".to_string(), font);
                return Ok(Self {
                    fonts,
                    default_font: "default".to_string(),
                });
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
    ///
    /// TODO
    /// - Baselineの計算
    fn measure(&self, req: &TextMeasurementRequest) -> Result<TextMeasurement, TextMeasureError> {
        let font = self
            .fonts
            .get(&self.default_font)
            .ok_or_else(|| TextMeasureError::FontNotFound(self.default_font.clone()))?;
        let font_size = req.font.size_px.max(1.0);

        let text = &req.text;

        let line_height = font_size * 1.2;

        // 空文字は高さだけ返す
        if text.is_empty() {
            return Ok(TextMeasurement {
                width: 0.0,
                height: line_height,
                baseline: font_size * 0.8,
                glyphs: None,
            });
        }

        // スペース幅をタブ処理のために取得
        let space_advance = {
            let (m, _b) = font.rasterize(' ', font_size);
            m.advance_width
        };

        // 文字幅cache
        let mut advance_cache: HashMap<char, f32> = HashMap::new();

        let mut max_width: f32 = 0.0;
        let mut cur_width: f32 = 0.0;
        let mut lines: usize = 1;

        for ch in text.chars() {
            if ch == '\r' {
                // CR は無視（CRLF は \n で処理）
                continue;
            }

            if ch == '\n' {
                max_width = max_width.max(cur_width);
                cur_width = 0.0;
                lines = lines.saturating_add(1);
                if let Some(max_lines) = req.constraints.max_lines
                    && lines > max_lines
                {
                    break;
                }
                continue;
            }

            let advance = if ch == '\t' {
                // タブはスペース4個分で扱う
                space_advance * 4.0
            } else {
                *advance_cache.entry(ch).or_insert_with(|| {
                    let (metrics, _) = font.rasterize(ch, font_size);
                    metrics.advance_width
                })
            };

            if req.constraints.wrap {
                if let Some(mw) = req.constraints.max_width {
                    if cur_width + advance > mw && cur_width > 0.0 {
                        max_width = max_width.max(cur_width);
                        cur_width = advance;
                        lines = lines.saturating_add(1);
                        if let Some(max_lines) = req.constraints.max_lines
                            && lines > max_lines
                        {
                            break;
                        }
                    } else {
                        cur_width += advance;
                    }
                } else {
                    cur_width += advance;
                }
            } else {
                cur_width += advance;
            }
        }

        max_width = max_width.max(cur_width);

        if let Some(max_lines) = req.constraints.max_lines
            && lines > max_lines
        {
            lines = max_lines;
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
