//! RenderNode と RenderTree
//! 最低限のレイアウト情報を保持する。

use super::render::Color;
use crate::engine::tree::Tree;

#[derive(Debug, Clone)]
pub enum NodeKind {
    /// テキストノード
    Text {
        text: String,
        font_size: f32,
        color: Color,
    },

    /// ボタンなどのインタラクティブな要素
    Button,

    /// スクロール可能要素（内部にツリーを持つ）
    Scrollable {
        tree: Tree<RenderNode>,
        scroll_offset_x: f32,
        scroll_offset_y: f32,
    },

    /// ブロック要素（幅いっぱい＋縦積み）
    Block,

    /// インライン要素（横方向に並ぶ）
    Inline,

    /// 未知の要素
    Unknown,
}

#[derive(Debug, Clone)]
pub struct RenderNode {
    pub kind: NodeKind,

    /// 計算されたレイアウト位置
    pub x: f32,
    pub y: f32,

    /// 計算されたレイアウトサイズ
    pub width: f32,
    pub height: f32,

    /// レイアウトアルゴリズムが必要とするメタ情報
    pub layout: LayoutInfo,
}

/// レイアウト再計算のための最低限の情報
#[derive(Debug, Clone)]
pub struct LayoutInfo {
    /// 親から与えられた幅
    pub available_width: f32,

    pub preferred_width: Option<f32>,
    pub preferred_height: Option<f32>,

    /// パディングなど（必要最低限）
    pub padding_left: f32,
    pub padding_right: f32,
    pub padding_top: f32,
    pub padding_bottom: f32,
}

impl LayoutInfo {
    pub fn new(available_width: f32) -> Self {
        Self {
            available_width,
            preferred_height: None,
            preferred_width: None,
            padding_left: 0.0,
            padding_right: 0.0,
            padding_top: 0.0,
            padding_bottom: 0.0,
        }
    }
}

impl RenderNode {
    pub fn new(kind: NodeKind, x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            kind,
            x,
            y,
            width,
            height,
            layout: LayoutInfo::new(width),
        }
    }

    /// Scrollable のオフセット変更
    pub fn set_scroll_offset(&mut self, offset_x: f32, offset_y: f32) {
        if let NodeKind::Scrollable {
            scroll_offset_x,
            scroll_offset_y,
            ..
        } = &mut self.kind
        {
            *scroll_offset_x = offset_x;
            *scroll_offset_y = offset_y;
        }
    }
}

pub type RenderTree = Tree<RenderNode>;
