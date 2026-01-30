use std::{env, sync::Arc};

use crate::engine::layouter::types::{FontStyle, TextAlign, TextStyle};
use glyphon::{
    Attrs, Buffer, Cache, Color as GlyphColor, FontSystem, Metrics, PrepareError, Resolution,
    Shaping, Style, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer as TextBrush,
    Viewport, Weight, cosmic_text::Align, fontdb,
};

use crate::platform::font;

/// テキストセクション位置・クリップ・描画するBufferをまとめた構造体
pub struct TextSection {
    /// スクリーン上の位置 (左上原点)
    pub screen_position: (f32, f32),
    /// クリップ領域の左上座標（スクリーン座標）
    pub clip_origin: (f32, f32),
    /// クリップ領域の幅・高さ
    pub bounds: (f32, f32),
    pub buffer: Buffer,
}

/// glyphon使ったテキストレンダラー
pub struct TextRenderer {
    /// glyphonのテキストブラシ
    brush: TextBrush,
    /// ビューポート情報
    viewport: Viewport,
    /// glyphonのテキストアトラス
    atlas: TextAtlas,
    /// rasterize 結果のキャッシュ
    swash_cache: SwashCache,
    font_sys: FontSystem,
}

impl TextRenderer {
    /// 情報を渡してシステムフォントから初期化する
    pub fn new_from_device(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> anyhow::Result<Self> {
        // 後々環境変数とかに設定しているときに使えるようにしてます
        if let Ok(p) = env::var("ORINIUM_FONT")
            && let Ok(bytes) = std::fs::read(&p)
        {
            return Self::new_from_bytes(device, queue, format, bytes);
        }

        for p in font::system_font_candidates()? {
            if let Ok(bytes) = std::fs::read(p) {
                // build brush from bytes
                return Self::new_from_bytes(device, queue, format, bytes);
            }
        }

        anyhow::bail!("no system font found");
    }

    pub fn new_with_fontsys(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        font_sys: FontSystem,
    ) -> anyhow::Result<Self> {
        let cache = Cache::new(device);
        let mut atlas = TextAtlas::new(device, queue, &cache, format);
        let multisample = wgpu::MultisampleState {
            count: 1,                         // MSAA 無効
            mask: !0,                         // 全サンプル有効
            alpha_to_coverage_enabled: false, // glyphon は距離場なので不要
        };
        let brush = TextBrush::new(&mut atlas, device, multisample, None);

        let viewport = Viewport::new(device, &cache);

        let swash_cache = SwashCache::new();

        Ok(Self {
            brush,
            atlas,
            font_sys,
            viewport,
            swash_cache,
        })
    }

    /// フォントバイト列から生成するコンストラクタ
    pub fn new_from_bytes(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        font_bytes: Vec<u8>,
    ) -> anyhow::Result<Self> {
        let font_source = Arc::new(font_bytes);
        let font = fontdb::Source::Binary(font_source);
        let font_sys = FontSystem::new_with_fonts(vec![font]);
        Self::new_with_fontsys(device, queue, format, font_sys)
    }

    /// Create a cosmic-text `Buffer` for the given text using the internal `FontSystem`.
    /// This encapsulates the required `Metrics` and calls `set_text`.
    pub fn create_buffer_for_text(&mut self, text: &str, text_style: TextStyle) -> Buffer {
        let font_size = text_style.font_size;
        let color = text_style.color;
        let color = GlyphColor::rgba(color.0, color.1, color.2, color.3);
        let align = text_style.text_align;
        let weight = text_style.font_weight;
        let style = text_style.font_style;
        // reasonable default line height (1.2x)
        let metrics = Metrics::relative(font_size, 1.2);

        let mut buffer = Buffer::new(&mut self.font_sys, metrics);

        // build attributes (defaults + color + metrics)
        let attrs = Attrs::new()
            .metrics(metrics)
            .color(color)
            .weight(Weight(weight.0))
            .style(Style::from(style));

        // shape and layout
        buffer.set_text(
            &mut self.font_sys,
            text,
            &attrs,
            Shaping::Advanced,
            Some(Align::from(align)),
        );

        buffer
    }

    /// 指定されたセクション群をギリフォン用の TextArea に変換して Atlas に転送する
    pub fn queue<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sections: &'a [TextSection],
    ) -> Result<(), PrepareError> {
        // TextArea は Buffer を参照するライフタイムを持つため、一時的にベクタに詰めて渡す
        let mut text_areas: Vec<TextArea<'a>> = Vec::with_capacity(sections.len());

        for s in sections.iter() {
            let bounds = TextBounds {
                left: s.clip_origin.0.round() as i32,
                top: s.clip_origin.1.round() as i32,
                right: (s.clip_origin.0 + s.bounds.0).round() as i32,
                bottom: (s.clip_origin.1 + s.bounds.1).round() as i32,
            };

            // デフォルト色は Buffer 内の属性が優先されるため適当で良い
            let default_color = GlyphColor::rgba(0, 0, 0, 255);

            let area = TextArea {
                buffer: &s.buffer,
                left: s.screen_position.0,
                top: s.screen_position.1,
                scale: 1.0,
                bounds,
                default_color,
                custom_glyphs: &[],
            };

            text_areas.push(area);
        }

        self.brush.prepare(
            device,
            queue,
            &mut self.font_sys,
            &mut self.atlas,
            &self.viewport,
            text_areas,
            &mut self.swash_cache,
        )
    }

    /// ビューポート（解像度）を更新
    pub fn resize_view(&mut self, width: f32, height: f32, queue: &wgpu::Queue) {
        self.viewport.update(
            queue,
            Resolution {
                width: width as u32,
                height: height as u32,
            },
        );
    }

    /// フレームを描画
    pub fn draw<'a>(&mut self, rpass: &mut wgpu::RenderPass<'a>) {
        self.brush
            .render(&self.atlas, &self.viewport, rpass)
            .expect("PANIC: Text draw failed");
    }
}

impl From<FontStyle> for Style {
    fn from(value: FontStyle) -> Self {
        match value {
            FontStyle::Normal => Style::Normal,
            FontStyle::Italic => Style::Italic,
            FontStyle::Oblique => Style::Oblique,
        }
    }
}

impl From<TextAlign> for Align {
    fn from(value: TextAlign) -> Self {
        match value {
            TextAlign::Center => Align::Center,
            TextAlign::Left => Align::Left,
            TextAlign::Right => Align::Right,
        }
    }
}
