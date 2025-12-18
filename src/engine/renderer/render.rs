//! RenderTree を DrawCommand に変換する Renderer

use std::{cell::RefCell, rc::Rc};

use super::render_node::{NodeKind, RenderNode, RenderTree};
use crate::engine::tree::TreeNode;

#[derive(Debug, Clone)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const BLACK: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };

    pub const WHITE: Color = Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };

    pub const RED: Color = Color {
        r: 1.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };

    pub const GREEN: Color = Color {
        r: 0.0,
        g: 1.0,
        b: 0.0,
        a: 1.0,
    };

    pub const BLUE: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 1.0,
        a: 1.0,
    };

    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub fn from_rgba_tuple(rgba: (u8, u8, u8, f32)) -> Self {
        Self {
            r: rgba.0 as f32 / 255.0,
            g: rgba.1 as f32 / 255.0,
            b: rgba.2 as f32 / 255.0,
            a: rgba.3,
        }
    }
}

#[derive(Debug, Clone)]
pub enum DrawCommand {
    DrawText {
        x: f32,
        y: f32,
        text: String,
        font_size: f32,
        color: Color,
    },

    DrawRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: Color,
    },

    DrawPolygon {
        points: Vec<(f32, f32)>,
        color: Color,
    },

    DrawEllipse {
        center: (f32, f32),
        radius_x: f32, // 円なら radius_x == radius_y
        radius_y: f32,
        color: Color,
    },

    /// クリッピング領域（ネスト可能）
    PushClip {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    PopClip,

    /// 座標変換（スクロールや入れ子レイアウト）
    PushTransform {
        dx: f32,
        dy: f32,
    },
    PopTransform,
}

pub struct Renderer;

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    pub fn new() -> Self {
        Self
    }

    pub fn generate_draw_commands(&self, tree: &RenderTree) -> Vec<DrawCommand> {
        let mut commands = vec![];
        Self::traverse_tree(&tree.root, &mut commands);
        commands
    }

    fn traverse_tree(node: &Rc<RefCell<TreeNode<RenderNode>>>, out: &mut Vec<DrawCommand>) {
        let node_borrow = node.borrow();
        let abs_x = node_borrow.value.x;
        let abs_y = node_borrow.value.y;

        match &node_borrow.value.kind {
            NodeKind::Text {
                text,
                font_size,
                color,
            } => {
                out.push(DrawCommand::DrawText {
                    x: abs_x,
                    y: abs_y,
                    text: text.clone(),
                    font_size: *font_size,
                    color: color.clone(),
                });
                /*
                out.push(DrawCommand::DrawRect {
                    x: abs_x,
                    y: abs_y,
                    width: node_borrow.value.width,
                    height: node_borrow.value.height,
                    color: Color::new(0.9, 0.0, 0.0, 1.0),
                });
                */
            }
            NodeKind::Button => {
                out.push(DrawCommand::DrawRect {
                    x: abs_x,
                    y: abs_y,
                    width: node_borrow.value.width,
                    height: node_borrow.value.height,
                    color: Color::new(0.8, 0.8, 0.8, 1.0),
                });
            }
            NodeKind::Container => {
                out.push(DrawCommand::DrawRect {
                    x: abs_x,
                    y: abs_y,
                    width: node_borrow.value.width,
                    height: node_borrow.value.height,
                    color: Color::new(0.9, 0.9, 0.9, 1.0),
                });
            }
            NodeKind::Scrollable {
                tree: inner_tree,
                scroll_offset_x,
                scroll_offset_y,
                ..
            } => {
                out.push(DrawCommand::DrawRect {
                    x: abs_x,
                    y: abs_y,
                    width: node_borrow.value.width,
                    height: node_borrow.value.height,
                    color: Color::new(0.95, 0.95, 0.95, 1.0),
                });

                out.push(DrawCommand::PushClip {
                    x: abs_x,
                    y: abs_y,
                    width: node_borrow.value.width,
                    height: node_borrow.value.height,
                });
                out.push(DrawCommand::PushTransform {
                    dx: -*scroll_offset_x,
                    dy: -*scroll_offset_y,
                });

                // 内部ツリーを再帰描画
                Self::traverse_tree(&inner_tree.root, out);

                out.push(DrawCommand::PopTransform);
                out.push(DrawCommand::PopClip);
            }
            NodeKind::Unknown => {
                // 無視
            }
        }

        for child in node_borrow.children() {
            Self::traverse_tree(child, out);
        }
    }
}
