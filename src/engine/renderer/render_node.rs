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

    Container,

    /// 未知の要素
    Unknown,
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeKind::Scrollable { .. } => write!(f, "Scrollable {{..}}"),
            _ => write!(f, "{:?}", self),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RenderNode {
    kind: NodeKind,

    /// 計算されたレイアウト位置
    pub x: f32,
    pub y: f32,

    /// 計算されたレイアウトサイズ
    pub width: f32,
    pub height: f32,
    // NOTE: レイアウトメタ情報と display は ComputedTree 側で扱うため
    // RenderNode からは外して、レンダリングに必要な位置・大きさ・内容のみにする。
}

/// RenderNode のレイアウト結果に対する安定した API
pub trait RenderNodeTrait {
    fn set_layout(&mut self, x: f32, y: f32, width: f32, height: f32);

    fn set_position(&mut self, x: f32, y: f32) {
        let (w, h) = self.size();
        self.set_layout(x, y, w, h);
    }

    fn set_size(&mut self, width: f32, height: f32) {
        let (x, y) = self.position();
        self.set_layout(x, y, width, height);
    }

    fn position(&self) -> (f32, f32);
    fn size(&self) -> (f32, f32);

    fn kind(&self) -> &NodeKind;
    fn kind_mut(&mut self) -> &mut NodeKind;
}

impl RenderNodeTrait for RenderNode {
    fn set_layout(&mut self, x: f32, y: f32, width: f32, height: f32) {
        self.x = x;
        self.y = y;
        self.width = width;
        self.height = height;
    }

    fn position(&self) -> (f32, f32) {
        (self.x, self.y)
    }

    fn size(&self) -> (f32, f32) {
        (self.width, self.height)
    }

    fn kind(&self) -> &NodeKind {
        &self.kind
    }

    fn kind_mut(&mut self) -> &mut NodeKind {
        &mut self.kind
    }
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

#[derive(Debug, Clone, Default)]
pub enum Display {
    #[default]
    Block,
    Inline,
    None,
}

impl Display {
    pub fn is_none(&self) -> bool {
        matches!(self, Display::None)
    }

    pub fn from_css_display(display: crate::engine::css::values::Display) -> Self {
        match display {
            crate::engine::css::values::Display::Block => Display::Block,
            crate::engine::css::values::Display::Inline => Display::Inline,
            crate::engine::css::values::Display::None => Display::None,
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
