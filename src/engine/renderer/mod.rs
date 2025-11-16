//! DOM と CSSOM を統合して RenderTree を生成し、描画命令を出力

mod render_tree;

use self::render_tree::{BuildRenderTree, RenderObjectKind, RenderTree};
use std::cell::RefCell;
use std::rc::Rc;

use crate::engine::styler::StyleTree;

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

#[derive(Debug, Clone, Copy)]
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

pub struct Renderer {
    pub viewport_width: f32,
    pub viewport_height: f32,
}

impl Renderer {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            viewport_width: width,
            viewport_height: height,
        }
    }

    /// StyleTree → ComputedStyleTree → RenderTree → DrawCommand
    pub fn generate_draw_commands(&self, style_tree: &mut StyleTree) -> Vec<DrawCommand> {
        let computed_tree = style_tree.compute();
        let render_tree: RenderTree = computed_tree.build_render_tree();

        let mut commands = Vec::new();
        self.traverse_render_tree(&render_tree.root, &mut commands, 0.0, 0.0);
        commands
    }

    fn traverse_render_tree(
        &self,
        node: &Rc<
            RefCell<
                crate::engine::tree::TreeNode<crate::engine::renderer::render_tree::RenderObject>,
            >,
        >,
        commands: &mut Vec<DrawCommand>,
        mut current_x: f32,
        mut current_y: f32,
    ) {
        let obj = node.borrow().value.clone();

        match obj.kind {
            RenderObjectKind::Text => {
                if let Some(text) = obj.text {
                    let color = obj
                        .style
                        .color
                        .map(|c| {
                            let (r, g, b, a) = c.to_rgba_tuple(None);
                            Color::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a)
                        })
                        .unwrap_or(Color::BLACK);

                    commands.push(DrawCommand::DrawText {
                        x: current_x,
                        y: current_y,
                        text: text.clone(),
                        font_size: 16.0,
                        color,
                    });

                    current_x += text.len() as f32 * 8.0;
                }
            }

            RenderObjectKind::Block | RenderObjectKind::Inline => {
                // 簡易レイアウト: Block は改行、Inline は横に並べる
                if let RenderObjectKind::Block = obj.kind {
                    current_x = 0.0;
                    current_y += 20.0;
                }
                if let RenderObjectKind::Inline = obj.kind {
                    let width = obj.layout.content.width;
                    current_x += width;
                    current_y += 0.0;
                }

                for child in &node.borrow().children {
                    self.traverse_render_tree(child, commands, current_x, current_y);
                }

                if let RenderObjectKind::Block = obj.kind {
                    current_x = 0.0;
                    current_y += 10.0;
                }
            }

            RenderObjectKind::Anonymous => {}
        }
    }
}
