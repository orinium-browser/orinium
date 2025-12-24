//! Linux のシステムフォント取得
//!
//! ディストリ差が大きいため、
//! ある確率が高そうなものを列挙する

use anyhow::Result;
use std::path::PathBuf;

pub fn system_font_candidates() -> Result<Vec<PathBuf>> {
    Ok(vec![
        // DejaVu
        PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
        // Noto
        PathBuf::from("/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf"),
        PathBuf::from("/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc"),
        // FreeFont
        PathBuf::from("/usr/share/fonts/truetype/freefont/FreeSans.ttf"),
    ])
}
