//! engine::renderer - DOM と CSSOM を統合して描画命令を生成

mod render_tree;

#[allow(unused_imports)]
use self::render_tree::{RenderTree, RenderObject};

use std::cell::RefCell;
use std::rc::Rc;

use crate::engine::css::cssom::CssNodeType;
use crate::engine::html::parser::HtmlNodeType;
use crate::engine::html::util as html_util;
use crate::engine::tree::{Tree, TreeNode};

#[derive(Debug, Clone)]
pub enum DrawCommand {
    DrawRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: Color,
    },
    DrawText {
        x: f32,
        y: f32,
        text: String,
        font_size: f32,
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
    pub const WHITE: Color = Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };

    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub fn to_tuple(&self) -> (f32, f32, f32, f32) {
        (self.r, self.g, self.b, self.a)
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

    /// RenderTree を構築して描画命令を生成
    pub fn generate_draw_commands(
        &self,
        dom_tree: &Tree<HtmlNodeType>,
        _css_tree: &Tree<CssNodeType>,
    ) -> Vec<DrawCommand> {
        let mut commands = Vec::new();
        let mut current_x = 10.0;
        let mut current_y = 10.0;

        Renderer::traverse_and_generate(
            dom_tree.clone().root,
            &mut commands,
            &mut current_x,
            &mut current_y,
        );

        commands
    }

    /// DOMツリーを走査して描画命令を生成（再帰的）
    fn traverse_and_generate(
        node: Rc<RefCell<TreeNode<HtmlNodeType>>>,
        commands: &mut Vec<DrawCommand>,
        current_x: &mut f32,
        current_y: &mut f32,
    ) {
        let node_borrow = node.borrow();

        match &node_borrow.value {
            HtmlNodeType::Document => {
                // ドキュメントノードは子要素を処理
                for child in &node_borrow.children {
                    Renderer::traverse_and_generate(child.clone(), commands, current_x, current_y);
                }
            }
            HtmlNodeType::Element { tag_name, .. } => {
                // 要素ノードの処理
                let line_height = 20.0;

                // ブロック要素の場合は改行
                if html_util::is_block_level_element(tag_name.as_str()) {
                    *current_x = 10.0;
                    *current_y += line_height;
                }

                // 子要素を処理
                for child in &node_borrow.children {
                    Renderer::traverse_and_generate(child.clone(), commands, current_x, current_y);
                }

                // ブロック要素の後は改行
                if html_util::is_block_level_element(tag_name.as_str()) {
                    *current_x = 10.0;
                    *current_y += line_height / 2.0;
                }
            }
            HtmlNodeType::Text(text) => {
                // テキストノードの処理
                if !text.trim().is_empty() {
                    commands.push(DrawCommand::DrawText {
                        x: *current_x,
                        y: *current_y,
                        text: text.clone(),
                        font_size: 16.0,
                        color: Color::BLACK,
                    });

                    // 簡易的なテキスト幅計算（実際にはフォントメトリクスが必要）
                    *current_x += text.len() as f32 * 8.0;
                }
            }
            _ => {} // その他のノードタイプを無視
        }
    }
}
