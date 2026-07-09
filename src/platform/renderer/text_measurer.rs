use crate::engine::bridge::text::{
    GlyphCluster, MeasuredFragment, TextMeasureError, TextMeasureRequest, TextMeasurer,
};
use crate::engine::layouter::types::{FontStyle, LineHeight, TextStyle as EngineTextStyle};
use crate::platform::renderer::text::global_font;
use crate::platform::renderer::text::text::*;

use orinium_text::TextStyle as OriTextStyle;
use orinium_text::{
    BidiMode, Color as OriColor, FontStyle as OriFontStyle, FontWeight as OriFontWeight,
    TextLayouter,
};

fn quantize_font_size(px: f32) -> f32 {
    (px * 64.0).round() / 64.0
}

pub struct PlatformTextMeasurer;

impl PlatformTextMeasurer {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        if global_font::global_font_system_ready() {
            Ok(Self)
        } else {
            Err("no system font found".into())
        }
    }

    pub fn from_bytes(_id: &str, _bytes: Vec<u8>) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self)
    }
}

impl TextMeasurer<EngineTextStyle> for PlatformTextMeasurer {
    fn measure(
        &self,
        req: &TextMeasureRequest<EngineTextStyle>,
    ) -> Result<Vec<MeasuredFragment>, TextMeasureError> {
        let _t0 = std::time::Instant::now();

        let font_size = quantize_font_size(req.style.font_size.max(1.0));

        let line_height_ratio = match req.style.line_height {
            LineHeight::Normal => 1.2,
            LineHeight::Number(n) => n,
            LineHeight::Px(px) => px / font_size,
        };

        let font_families = build_family_list(&req.style.font_families);

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
            font_families,
            exact_fonts: Vec::new(),
        };

        let mut layouter = TextLayouter::new();

        let t_shape = std::time::Instant::now();
        let shaped = global_font::with_global_font_system(|fs| {
            layouter.shape_text(fs, &req.text, &ori_style)
        });
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
        let layout = global_font::with_global_font_system(|fs| {
            layouter.layout_lines(fs, &shaped, &line_ranges, &ori_style)
        });
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
            "measure: text={:?} len={} font_size={}  shape={:?}  layout={:?}  total={:?}  fragments={}",
            preview,
            req.text.len(),
            font_size,
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

        let line_height_ratio = match req.style.line_height {
            LineHeight::Normal => 1.2,
            LineHeight::Number(n) => n,
            LineHeight::Px(px) => px / font_size,
        };

        let font_families = build_family_list(&req.style.font_families);

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
            font_families,
            exact_fonts: Vec::new(),
        };

        let mut layouter = TextLayouter::new();

        let shaped = global_font::with_global_font_system(|fs| {
            layouter.shape_text(fs, &req.text, &ori_style)
        });

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
