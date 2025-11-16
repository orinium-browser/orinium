//! CSS `display` プロパティ値
//!
//! 必要な値から順に拡張していく。
//! ブラウザの最小構成として、Block / Inline / None を持つ。

/// display: ~~
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Display {
    Block,
    #[default]
    Inline,
    None,
    // 将来的に追加する例：
    // InlineBlock,
    // Flex,
    // Grid,
}
