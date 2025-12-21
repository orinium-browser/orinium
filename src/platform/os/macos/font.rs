//! macOS のシステムフォント取得

use anyhow::Result;
use std::path::PathBuf;

/// macOS のシステムフォント候補を返す
pub fn system_font_candidates() -> Result<Vec<PathBuf>> {
    Ok(vec![
        // San Francisco（macOS 標準）
        PathBuf::from("/System/Library/Fonts/SFNS.ttf"),

        // 日本語（ヒラギノ）
        PathBuf::from("/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc"),
        PathBuf::from("/System/Library/Fonts/ヒラギノ明朝 ProN.ttc"),

        // fallback
        PathBuf::from("/System/Library/Fonts/Helvetica.ttc"),
        PathBuf::from("/System/Library/Fonts/AppleSDGothicNeo.ttc"),
    ])
}
