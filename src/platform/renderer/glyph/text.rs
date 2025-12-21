use std::{env, sync::Arc};

use glyphon::{
    Attrs, Buffer, Cache, Color as GlyphColor, FontSystem, Metrics, PrepareError, Resolution,
    Shaping, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer as TextBrush, Viewport,
    fontdb,
};

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
    /// glyphonのキャッシュ
    cache: Cache,
    /// glyphonのテキストアトラス
    atlas: TextAtlas,
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

        let candidates = [
            "C:\\Windows\\Fonts\\meiryo.ttc",   // メイリオ
            "C:\\Windows\\Fonts\\msgothic.ttc", // MS ゴシック
            "C:\\Windows\\Fonts\\msmincho.ttc", // MS 明朝
            "C:\\Windows\\Fonts\\arial.ttf",    // Arial
            "C:\\Windows\\Fonts\\segoeui.ttf",  // Segoe UI
            "C:\\Windows\\Fonts\\seguisym.ttf", // Segoe UI Symbol
        ];

        for p in &candidates {
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

        Ok(Self {
            brush,
            cache,
            atlas,
            font_sys,
            viewport,
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
    pub fn create_buffer_for_text(
        &mut self,
        text: &str,
        font_size: f32,
        color: GlyphColor,
    ) -> Buffer {
        // reasonable default line height (1.2x)
        let metrics = Metrics::relative(font_size, 1.2);

        let mut buffer = Buffer::new(&mut self.font_sys, metrics);

        // build attributes (defaults + color + metrics)
        let attrs = Attrs::new().metrics(metrics).color(color);

        // shape and layout
        buffer.set_text(&mut self.font_sys, text, &attrs, Shaping::Advanced);

        buffer
    }

    /// 指定されたセクション群をギリフォン用の TextArea に変換して Atlas に転送する
    pub fn queue<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sections: &'a [TextSection],
    ) -> Result<(), PrepareError> {
        let mut cache = SwashCache::new();

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
            &mut cache,
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
