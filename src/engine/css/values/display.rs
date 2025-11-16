//! CSS `display` プロパティ値
//!
//! 必要な値から順に拡張していく。
//! ブラウザの最小構成として、Block / Inline / None を持つ。

/// display: ~~
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Display {
    Block,
    Inline,
    None,
    // 将来的に追加する例：
    // InlineBlock,
    // Flex,
    // Grid,
}

impl Default for Display {
    fn default() -> Self {
        Display::Inline
    }
}
