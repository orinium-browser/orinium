//! システムフォント取得の Facade

use anyhow::Result;
use std::path::PathBuf;

#[cfg(target_os = "windows")]
use crate::platform::os::windows;

#[cfg(target_os = "macos")]
use crate::platform::os::macos;

#[allow(unreachable_code)]
pub fn system_font_candidates() -> Result<Vec<PathBuf>> {
    #[cfg(target_os = "windows")]
    {
        return windows::font::system_font_candidates();
    }

    #[cfg(target_os = "macos")]
    {
        return crate::platform::os::macos::font::system_font_candidates();
    }

    #[cfg(target_os = "linux")]
    {
        return crate::platform::os::linux::font::system_font_candidates();
    }

    anyhow::bail!("system font is not supported on this OS yet");
}
