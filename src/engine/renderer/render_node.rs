//! RenderNode と RenderTree

use crate::engine::tree::Tree;

#[derive(Debug, Clone)]
pub enum NodeKind {
    Text(String),
    Button,
    Scrollable {
        tree: Tree<RenderNode>,
        scroll_offset_y: f32,
        scroll_offset_x: f32,
    }, // スクロール可能ノード
    Block,
    Inline,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct RenderNode {
    pub kind: NodeKind,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
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

    pub fn set_scroll_offset(&mut self,offset_x: f32 ,offset_y: f32) {
        if let NodeKind::Scrollable {
            scroll_offset_x,
            scroll_offset_y, ..
        } = &mut self.kind
        {
            *scroll_offset_x = offset_x;
            *scroll_offset_y = offset_y;
        }
    }
}

pub type RenderTree = Tree<RenderNode>;
