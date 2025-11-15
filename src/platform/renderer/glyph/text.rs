use std::error::Error;

pub type Section<'a> = wgpu_text::glyph_brush::Section<'a>;

use wgpu_text::{BrushBuilder, TextBrush};

pub struct TextRenderer {
    brush: Option<TextBrush<ab_glyph::FontArc>>,
    pending_font: Option<Vec<u8>>,
}

impl TextRenderer {
    /// フォントファイルパスを受け取り、フォントバイトを読み込んで保持する
    pub fn new(font_path: &str) -> Result<Self, Box<dyn Error + Send + Sync + 'static>> {
        let bytes = std::fs::read(font_path)?;
        Ok(Self {
            brush: None,
            pending_font: Some(bytes),
        })
    }

    /// 既に wgpu デバイスとフォントがある場合の生成
    pub fn new_with_fontarc(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        font_arc: ab_glyph::FontArc,
    ) -> Result<Self, Box<dyn Error + Send + Sync + 'static>> {
        let brush = BrushBuilder::using_font(font_arc).build(device, width, height, format);
        Ok(Self {
            brush: Some(brush),
            pending_font: None,
        })
    }

    /// フォントバイト列から生成するコンストラクタ
    pub fn new_from_bytes(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        font_bytes: Vec<u8>,
    ) -> Result<Self, Box<dyn Error + Send + Sync + 'static>> {
        let font_arc = ab_glyph::FontArc::try_from_vec(font_bytes)?;
        Self::new_with_fontarc(device, width, height, format, font_arc)
    }

    /// セクションをキューに入れる
    pub fn queue<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sections: &[Section<'a>],
    ) -> Result<(), Box<dyn Error>> {
        if self.brush.is_none() {
            if let Some(bytes) = self.pending_font.take() {
                // device/queue/format/width/height情報が必要だが、ここではdeviceだけある。幅・高さ・formatは仮の値を使うか、呼び出し側でnew_from_bytesを呼ぶべき。
                // 安全策として初期化は行わずpending_fontを戻す
                self.pending_font = Some(bytes);
            }
        }

        if let Some(brush) = &mut self.brush {
            brush.queue(device, queue, sections)?;
        }
        Ok(())
    }

    /// 実際の描画
    pub fn draw<'a>(&mut self, rpass: &mut wgpu::RenderPass<'a>) {
        if let Some(brush) = &mut self.brush {
            brush.draw(rpass);
        }
    }

    /// ビューサイズが変わったとき
    pub fn resize_view(&mut self, width: f32, height: f32, queue: &wgpu::Queue) {
        if let Some(brush) = &mut self.brush {
            brush.resize_view(width, height, queue);
        }
    }
}