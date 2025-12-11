use std::{env, sync::Arc};

use glyphon::{
    Buffer, Cache, FontSystem, PrepareError, SwashCache, TextArea, TextAtlas,
    TextRenderer as TextBrush, Viewport, fontdb,
};

pub type TextSection = Buffer;

pub struct TextRenderer {
    brush: TextBrush,
    viewport: Viewport,
    cache: Cache,
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

        // 代表的な Windows フォント候補
        let candidates = [
            "C:\\Windows\\Fonts\\meiryo.ttc",   // メイリオ
            "C:\\Windows\\Fonts\\msgothic.ttc", // MS ゴシック
            "C:\\Windows\\Fonts\\msmincho.ttc", // MS 明朝
            "C:\\Windows\\Fonts\\arial.ttf",
            "C:\\Windows\\Fonts\\segoeui.ttf",
            "C:\\Windows\\Fonts\\seguisym.ttf",
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

    /// Buffer から TextArea を作って Atlas に転送（GPU へコピー）
    pub fn prepare<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        text_buffers: Vec<Buffer>,
    ) -> Result<(), PrepareError> {
        let mut cache = SwashCache::new();
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

    /// フレームを描画
    pub fn draw<'a>(&mut self, rpass: &mut wgpu::RenderPass<'a>) {
        self.brush.render(&self.atlas, &self.viewport, rpass);
    }
}
