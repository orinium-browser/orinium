//! システムフォント取得の Facade

use anyhow::Result;
use std::path::PathBuf;

#[cfg(target_os = "windows")]
use crate::platform::os::windows;

#[allow(unreachable_code)]
pub fn system_font_candidates() -> Result<Vec<PathBuf>> {
    #[cfg(target_os = "windows")]
    {
        return windows::font::system_font_candidates();
    }

    anyhow::bail!("system font is not supported on this OS yet");
}
