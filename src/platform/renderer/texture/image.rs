use std::path::Path;
use std::io::Cursor;

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImagePixelFormat {
    Rgba8,
    Bgra8,
    Luma8,
}

#[derive(Debug, Error)]
pub enum ImageError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Decode error: {0}")]
    Decode(#[from] image::ImageError),

    #[error("Unsupported image format")]
    UnsupportedFormat,

    #[error("Image is too large: {0}x{1}")]
    TooLarge(u32, u32),
}

/// テクスチャ画像データ
pub struct TextureImage {
    /// 高さ
    pub width: u32,
    /// 幅
    pub height: u32,
    /// ピクセルデータ（フォーマットはformat参照）
    pub pixels: Vec<u8>,
    /// ピクセルフォーマット
    pub format: ImagePixelFormat,
}

impl TextureImage {
    /// 画像をバイト列から読み込み、RGBA8形式にデコードする。
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ImageError> {
        // Try to guess format and decode
        let reader = image::io::Reader::new(Cursor::new(bytes)).with_guessed_format()?;
        let dyn_img = reader.decode()?;
        let rgba = dyn_img.to_rgba8();
        let (width, height) = (rgba.width(), rgba.height());

        // sanity limit to avoid blowing memory (arbitrary, can be tuned)
        const MAX_DIM: u32 = 10000;
        if width > MAX_DIM || height > MAX_DIM {
            return Err(ImageError::TooLarge(width, height));
        }

        Ok(Self {
            width,
            height,
            pixels: rgba.into_raw(),
            format: ImagePixelFormat::Rgba8,
        })
    }

    /// 画像をファイルパスから読み込み、RGBA8形式にデコードする。
    pub fn from_path(path: &Path) -> Result<Self, ImageError> {
        let bytes = std::fs::read(path)?;
        Self::from_bytes(&bytes)
    }

    /// 画像データをRGBA8形式のバイト列として取り出す。
    pub fn into_rgba8(self) -> (Vec<u8>, u32, u32) {
        (self.pixels, self.width, self.height)
    }

    /// 画像データをビューとして取得する。
    pub fn as_view(&self) -> (&[u8], u32, u32, ImagePixelFormat) {
        (&self.pixels, self.width, self.height, self.format)
    }
}
