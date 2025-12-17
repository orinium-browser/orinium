use std::fmt;

/// フォントの説明
#[derive(Debug, Clone)]
pub struct FontDescription {
    /// フォントファミリ名（None の場合はデフォルトフォント）
    pub family: Option<String>,
    /// フォントサイズ（ピクセル単位）
    pub size_px: f32,
}

/// レイアウト制約
#[derive(Debug, Clone)]
pub struct LayoutConstraints {
    /// 最大幅（None の場合は無制限）
    pub max_width: Option<f32>,
    /// 折り返しを有効にするかどうか
    pub wrap: bool,
    /// 最大行数（None の場合は無制限）
    pub max_lines: Option<usize>,
}

/// テキスト測定リクエスト
#[derive(Debug, Clone)]
pub struct TextMeasurementRequest {
    /// 測定するテキスト
    pub text: String,
    /// フォントの説明
    pub font: FontDescription,
    /// レイアウト制約
    pub constraints: LayoutConstraints,
}

/// グリフのメトリクス情報
#[derive(Debug, Clone)]
pub struct GlyphMetric {
    /// グリフID
    pub glyph_id: u32,
    /// Xオフセット
    pub x_offset: f32,
    /// Yオフセット
    pub y_offset: f32,
    /// アドバンス幅
    pub advance: f32,
    /// グリフの幅
    pub width: f32,
    /// グリフの高さ
    pub height: f32,
}

/// テキスト測定結果
#[derive(Debug, Clone)]
pub struct TextMeasurement {
    /// 全体の幅
    pub width: f32,
    /// 全体の高さ
    pub height: f32,
    /// ベースライン位置
    pub baseline: f32,
    /// グリフごとのメトリクス情報（存在しない場合もある）
    pub glyphs: Option<Vec<GlyphMetric>>,
}

/// テキスト測定エラー
#[derive(Debug, Clone)]
pub enum TextMeasureError {
    /// フォントが見つからない
    FontNotFound(String),
    /// フォントの読み込みエラー
    FontLoadError(String),
    /// サポートされていない機能
    UnsupportedFeature(String),
    /// レイアウトオーバーフロー
    LayoutOverflow,
    /// 内部エラー
    Internal(String),
}

impl fmt::Display for TextMeasureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TextMeasureError::FontNotFound(s) => write!(f, "Font not found: {}", s),
            TextMeasureError::FontLoadError(s) => write!(f, "Font load error: {}", s),
            TextMeasureError::UnsupportedFeature(s) => write!(f, "Unsupported feature: {}", s),
            TextMeasureError::LayoutOverflow => write!(f, "Layout overflow"),
            TextMeasureError::Internal(s) => write!(f, "Internal error: {}", s),
        }
    }
}

impl std::error::Error for TextMeasureError {}

pub trait TextMeasurer: Send + Sync {
    fn measure(&self, req: &TextMeasurementRequest) -> Result<TextMeasurement, TextMeasureError>;
}

pub mod fallback;
pub use fallback::EngineFallbackTextMeasurer;