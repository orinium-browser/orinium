//! Text measurement interface and implementations.

use crate::engine::bridge::text::{
    GlyphCluster, MeasuredFragment, TextMeasureError, TextMeasureRequest, TextMeasurer,
};
use crate::engine::layouter::types::{FontStyle, LineHeight, TextStyle as EngineTextStyle};

use std::env;
use std::sync::Mutex;

use orinium_text::TextStyle as OriTextStyle;
use orinium_text::{
    BidiMode, Color as OriColor, FontStyle as OriFontStyle, FontSystem,
    FontWeight as OriFontWeight, TextLayouter, fontdb,
};

/// Quantize font_size to 1/64 px to avoid cache misses from floating-point noise.
fn quantize_font_size(px: f32) -> f32 {
    (px * 64.0).round() / 64.0
}

/// Platform-backed text measurer using orinium_text.
///
/// This measurer performs real text shaping and line layout,
/// and is intended for production use.
pub struct PlatformTextMeasurer {
    font_sys: Mutex<FontSystem>,
}

impl PlatformTextMeasurer {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let font_sys = match env::var("ORINIUM_FONT") {
            Ok(p) => {
                let source = fontdb::Source::File(p.into());
                FontSystem::new_with_fonts(vec![source])
            }
            Err(_) => FontSystem::new(),
        };

        if font_sys.db.len() == 0 {
            return Err("no system font found".into());
        }

        Ok(Self {
            font_sys: Mutex::new(font_sys),
        })
    }

    pub fn from_bytes(_id: &str, bytes: Vec<u8>) -> Result<Self, Box<dyn std::error::Error>> {
        let source = fontdb::Source::Binary(std::sync::Arc::new(bytes));
        let font_sys = FontSystem::new_with_fonts(vec![source]);
        if font_sys.db.len() == 0 {
            return Err("no system font found".into());
        }
        Ok(Self {
            font_sys: Mutex::new(font_sys),
        })
    }
}

impl TextMeasurer<EngineTextStyle> for PlatformTextMeasurer {
    fn measure(
        &self,
        req: &TextMeasureRequest<EngineTextStyle>,
    ) -> Result<Vec<MeasuredFragment>, TextMeasureError> {
        let _t0 = std::time::Instant::now();

        let font_size = quantize_font_size(req.style.font_size.max(1.0));

        let mut fs = self
            .font_sys
            .lock()
            .map_err(|e| TextMeasureError::Internal(format!("font_sys lock poisoned: {}", e)))?;
        let t_lock = _t0.elapsed();

        let line_height_ratio = match req.style.line_height {
            LineHeight::Normal => 1.2,
            LineHeight::Number(n) => n,
            LineHeight::Px(px) => px / font_size,
        };

        let ori_style = OriTextStyle {
            font_size,
            color: OriColor(
                req.style.color.0,
                req.style.color.1,
                req.style.color.2,
                req.style.color.3,
            ),
            font_weight: OriFontWeight(req.style.font_weight.0),
            font_style: match req.style.font_style {
                FontStyle::Normal => OriFontStyle::Normal,
                FontStyle::Italic => OriFontStyle::Italic,
                FontStyle::Oblique => OriFontStyle::Oblique,
            },
            line_height: line_height_ratio,
            bidi_mode: BidiMode::Auto,
            font_families: vec![fontdb::Family::SansSerif],
            exact_fonts: Vec::new(),
        };

        let mut layouter = TextLayouter::new();

        let t_shape = std::time::Instant::now();
        let shaped = layouter.shape_text(&mut *fs, &req.text, &ori_style);
        let t_shape = t_shape.elapsed();

        let line_ranges: Vec<(usize, usize)> = req
            .text
            .split('\n')
            .scan(0usize, |offset, line| {
                let start = *offset;
                *offset += line.len() + 1;
                let end = start + line.len();
                Some((start, end))
            })
            .collect();

        let t_layout = std::time::Instant::now();
        let layout = layouter.layout_lines(&mut *fs, &shaped, &line_ranges, &ori_style);
        let t_layout = t_layout.elapsed();

        let fragments: Vec<MeasuredFragment> = layout
            .lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let line_text = line_ranges[i];
                MeasuredFragment {
                    text: req.text[line_text.0..line_text.1].to_string(),
                    width: line.width,
                    height: line.height,
                }
            })
            .collect();

        let total = _t0.elapsed();
        let preview = if req.text.len() > 40 {
            let cut = req.text.floor_char_boundary(40);
            format!("{}...", &req.text[..cut])
        } else {
            req.text.clone()
        };
        log::info!(
            target: "TextMeasurer",
            "measure: text={:?} len={} font_size={}  lock={:?}  shape={:?}  layout={:?}  total={:?}  fragments={}",
            preview,
            req.text.len(),
            font_size,
            t_lock,
            t_shape,
            t_layout,
            total,
            fragments.len(),
        );
        Ok(fragments)
    }

    fn measure_shaped(
        &self,
        req: &TextMeasureRequest<EngineTextStyle>,
    ) -> Result<Vec<GlyphCluster>, TextMeasureError> {
        let _t0 = std::time::Instant::now();

        let font_size = quantize_font_size(req.style.font_size.max(1.0));

        let mut fs = self
            .font_sys
            .lock()
            .map_err(|e| TextMeasureError::Internal(format!("font_sys lock poisoned: {}", e)))?;

        let line_height_ratio = match req.style.line_height {
            LineHeight::Normal => 1.2,
            LineHeight::Number(n) => n,
            LineHeight::Px(px) => px / font_size,
        };

        let ori_style = OriTextStyle {
            font_size,
            color: OriColor(
                req.style.color.0,
                req.style.color.1,
                req.style.color.2,
                req.style.color.3,
            ),
            font_weight: OriFontWeight(req.style.font_weight.0),
            font_style: match req.style.font_style {
                FontStyle::Normal => OriFontStyle::Normal,
                FontStyle::Italic => OriFontStyle::Italic,
                FontStyle::Oblique => OriFontStyle::Oblique,
            },
            line_height: line_height_ratio,
            bidi_mode: BidiMode::Auto,
            font_families: vec![fontdb::Family::SansSerif],
            exact_fonts: Vec::new(),
        };

        let mut layouter = TextLayouter::new();

        let shaped = layouter.shape_text(&mut *fs, &req.text, &ori_style);

        let clusters: Vec<GlyphCluster> = shaped
            .fragments
            .iter()
            .map(|f| GlyphCluster {
                byte_offset: f.cluster,
                width: f.width,
                break_allowed: f.break_after,
            })
            .collect();

        let total = _t0.elapsed();
        let preview = if req.text.len() > 40 {
            let cut = req.text.floor_char_boundary(40);
            format!("{}...", &req.text[..cut])
        } else {
            req.text.clone()
        };
        log::info!(
            target: "TextMeasurer",
            "measure_shaped: text={:?} len={} font_size={}  total={:?}  clusters={}",
            preview,
            req.text.len(),
            font_size,
            total,
            clusters.len(),
        );

        Ok(clusters)
    }
}
