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
    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
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
        self.traverse_tree(&tree.root, 0.0, 0.0, &mut commands);
        commands
    }

    fn traverse_tree(
        &self,
        node: &Rc<RefCell<TreeNode<RenderNode>>>,
        offset_x: f32,
        offset_y: f32,
        out: &mut Vec<DrawCommand>,
    ) {
        let node_borrow = node.borrow();
        let abs_x = offset_x + node_borrow.value.x;
        let abs_y = offset_y + node_borrow.value.y;

        match &node_borrow.value.kind {
            NodeKind::Text { text, font_size } => {
                out.push(DrawCommand::DrawText {
                    x: abs_x,
                    y: abs_y,
                    text: text.clone(),
                    font_size: *font_size,
                    color: Color::BLACK,
                });
            }
            NodeKind::Button | NodeKind::Block | NodeKind::Unknown => {
                out.push(DrawCommand::DrawRect {
                    x: abs_x,
                    y: abs_y,
                    width: node_borrow.value.width,
                    height: node_borrow.value.height,
                    color: Color::new(0.8, 0.8, 0.8, 1.0),
                });
            }
            NodeKind::Inline => {
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
                // 内部ツリーを再帰描画
                self.traverse_tree(&inner_tree.root, *scroll_offset_x, *scroll_offset_y, out);
            }
        }

        for child in node_borrow.children() {
            self.traverse_tree(child, offset_x, offset_y, out);
        }
    }
}
