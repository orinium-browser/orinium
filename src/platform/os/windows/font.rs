//! Windows のシステムフォント取得

use anyhow::Result;
use std::path::PathBuf;

/// システムフォント候補を返す
pub fn system_font_candidates() -> Result<Vec<PathBuf>> {
    Ok(vec![
        // 日本語
        PathBuf::from(r"C:\Windows\Fonts\meiryo.ttc"),
        PathBuf::from(r"C:\Windows\Fonts\msgothic.ttc"),
        PathBuf::from(r"C:\Windows\Fonts\msmincho.ttc"),
        // fallback
        PathBuf::from(r"C:\Windows\Fonts\segoeui.ttf"),
        PathBuf::from(r"C:\Windows\Fonts\seguisym.ttf"),
        PathBuf::from(r"C:\Windows\Fonts\arial.ttf"),
    ])
}
