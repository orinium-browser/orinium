//! render_tree.rs (更新版)
//! ComputedStyleTree から RenderTree を生成する
#![allow(dead_code, unused_imports)]
use std::cell::RefCell;
use std::rc::{Rc, Weak};

use crate::engine::tree::*;
use crate::engine::styler::computed_tree::{ComputedStyleNode, ComputedStyle};
use crate::engine::css::values::Color;
use crate::engine::css::values::Display;
use crate::html::HtmlNodeType;

/// レンダリング用ノード
#[derive(Debug, Clone)]
pub struct RenderObject {
    pub kind: RenderObjectKind,
    pub style: RenderStyle,
    pub layout: LayoutBox,
    pub text: Option<String>,
}

/// ノードの種類
#[derive(Debug, Clone)]
pub enum RenderObjectKind {
    Block,
    Inline,
    Text,
    Anonymous,
}

/// 最小限のレンダリングスタイル
#[derive(Debug, Clone)]
pub struct RenderStyle {
    pub color: Option<Color>,
    pub background_color: Option<Color>,
    pub font_size: Option<f32>, // 仮に px 単位
}

/// レイアウト矩形
#[derive(Debug, Clone)]
pub struct LayoutBox {
    pub content: Rect,
    pub padding: EdgeSizes,
    pub border: EdgeSizes,
    pub margin: EdgeSizes,
}

#[derive(Debug, Clone, Default)]
pub struct EdgeSizes {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

#[derive(Debug, Clone, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// レンダーツリー
pub type RenderTree = Tree<RenderObject>;

/// ComputedStyleTree から RenderTree を生成
pub trait BuildRenderTree {
    fn build_render_tree(&self) -> RenderTree;
}

impl BuildRenderTree for Tree<ComputedStyleNode> {
    fn build_render_tree(&self) -> RenderTree {
        self.map(&|node: &ComputedStyleNode| {
            let html_node = node.html.upgrade().unwrap();
            let html_node_ref = html_node.borrow();

            // display:none の場合は Anonymous にする（レンダーツリーに表示させない）
            let display = node.computed.as_ref().map(|c| c.display).unwrap_or_default();
            if let Display::None = display {
                return RenderObject {
                    kind: RenderObjectKind::Anonymous,
                    style: RenderStyle {
                        color: None,
                        background_color: None,
                        font_size: None,
                    },
                    layout: LayoutBox {
                        content: Rect::default(),
                        padding: EdgeSizes::default(),
                        border: EdgeSizes::default(),
                        margin: EdgeSizes::default(),
                    },
                    text: None,
                };
            }

            // ノードの種類を判定
            let kind = match &html_node_ref.value {
                HtmlNodeType::Text(_text) => RenderObjectKind::Text,
                _ => match display {
                    Display::Block => RenderObjectKind::Block,
                    Display::Inline => RenderObjectKind::Inline,
                    _ => RenderObjectKind::Inline,
                },
            };

            // テキストノードなら text フィールドに文字列を格納
            let text = match &html_node_ref.value {
                HtmlNodeType::Text(t) => Some(t.clone()),
                _ => None,
            };

            // レンダースタイル仮初期化
            let computed = node.computed.clone().unwrap();
            let style = RenderStyle {
                color: computed.color,
                background_color: computed.background_color,
                font_size: None,
            };

            let layout = LayoutBox {
                content: Rect::default(),
                padding: EdgeSizes::default(),
                border: EdgeSizes::default(),
                margin: EdgeSizes::default(),
            };

            RenderObject {
                kind,
                style,
                layout,
                text,
            }
        })
    }
}
