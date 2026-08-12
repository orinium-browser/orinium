use crate::engine::bridge::text::{
    GlyphCluster, MeasuredFragment, TextMeasureError, TextMeasureRequest, TextMeasurer,
};
use crate::engine::layouter::types::{FontStyle, LineHeight};
use crate::platform::renderer::text::global_font;
use crate::platform::renderer::text::text::*;
use crate::platform::renderer::text_cache::TextShapeCache;

use orinium_text::TextStyle as OriTextStyle;
use orinium_text::{
    BidiMode, Color as OriColor, FontStyle as OriFontStyle, FontWeight as OriFontWeight,
    TextLayouter,
};

fn quantize_font_size(px: f32) -> f32 {
    (px * 64.0).round() / 64.0
}

pub struct PlatformTextMeasurer {
    cache: TextShapeCache,
}

impl PlatformTextMeasurer {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        if global_font::global_font_system_ready() {
            Ok(Self {
                cache: TextShapeCache::new(),
            })
        } else {
            Err("no system font found".into())
        }
    }

    pub fn from_bytes(_id: &str, _bytes: Vec<u8>) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            cache: TextShapeCache::new(),
        })
    }
}

impl TextMeasurer for PlatformTextMeasurer {
    fn measure(&self, req: &TextMeasureRequest) -> Result<Vec<MeasuredFragment>, TextMeasureError> {
        let _t0 = std::time::Instant::now();

        let style = req.attribute.style.clone();
        let flow_style = req.attribute.flow_style;

        let font_size = quantize_font_size(flow_style.font_size.max(1.0));

        let line_height_ratio = match flow_style.line_height {
            LineHeight::Normal => 1.2,
            LineHeight::Number(n) => n,
            LineHeight::Px(px) => px / font_size,
        };

        let font_families = build_family_list(&style.font_families);

        let ori_style = OriTextStyle {
            font_size,
            color: OriColor(style.color.0, style.color.1, style.color.2, style.color.3),
            font_weight: OriFontWeight(style.font_weight.0),
            font_style: match style.font_style {
                FontStyle::Normal => OriFontStyle::Normal,
                FontStyle::Italic => OriFontStyle::Italic,
                FontStyle::Oblique => OriFontStyle::Oblique,
            },
            line_height: line_height_ratio,
            bidi_mode: BidiMode::Auto,
            font_families,
            exact_fonts: Vec::new(),
            variant: orinium_text::FontVariant::Normal,
        };

        let mut layouter = TextLayouter::new();

        let t_shape = std::time::Instant::now();
        let shaped = if let Some(shaped) = self.cache.get(&req.text, &ori_style) {
            shaped
        } else {
            let shaped = global_font::with_global_font_system(|fs| {
                layouter.shape_text(fs, &req.text, &ori_style)
            });

            self.cache.insert(&req.text, &ori_style, shaped.clone());
            shaped
        };
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
        req: &TextMeasureRequest,
    ) -> Result<Vec<GlyphCluster>, TextMeasureError> {
        let _t0 = std::time::Instant::now();

        let style = req.attribute.style.clone();
        let flow_style = req.attribute.flow_style;

        let font_size = quantize_font_size(flow_style.font_size.max(1.0));

        let line_height_ratio = match flow_style.line_height {
            LineHeight::Normal => 1.2,
            LineHeight::Number(n) => n,
            LineHeight::Px(px) => px / font_size,
        };

        let font_families = build_family_list(&style.font_families);

        let ori_style = OriTextStyle {
            font_size,
            color: OriColor(style.color.0, style.color.1, style.color.2, style.color.3),
            font_weight: OriFontWeight(style.font_weight.0),
            font_style: match style.font_style {
                FontStyle::Normal => OriFontStyle::Normal,
                FontStyle::Italic => OriFontStyle::Italic,
                FontStyle::Oblique => OriFontStyle::Oblique,
            },
            line_height: line_height_ratio,
            bidi_mode: BidiMode::Auto,
            font_families,
            exact_fonts: Vec::new(),
            variant: orinium_text::FontVariant::Normal,
        };

        let mut layouter = TextLayouter::new();

        let shaped = if let Some(shaped) = self.cache.get(&req.text, &ori_style) {
            shaped
        } else {
            let shaped = global_font::with_global_font_system(|fs| {
                layouter.shape_text(fs, &req.text, &ori_style)
            });

            self.cache.insert(&req.text, &ori_style, shaped.clone());
            shaped
        };

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
