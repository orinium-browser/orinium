//! engine::renderer - HTML/CSSレイアウト結果から描画命令を生成する論理描画層

use crate::engine::html::parser::{NodeRef, NodeType};
use crate::engine::html::util as html_util;

/// 描画命令を表す列挙型
#[derive(Debug, Clone)]
pub enum DrawCommand {
    /// 矩形を描画
    DrawRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: Color,
    },
    /// テキストを描画
    DrawText {
        x: f32,
        y: f32,
        text: String,
        font_size: f32,
        color: Color,
    },
}

/// 色を表す構造体
#[derive(Debug, Clone, Copy)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const WHITE: Color = Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };

    pub const BLACK: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };

    pub const GRAY: Color = Color {
        r: 0.5,
        g: 0.5,
        b: 0.5,
        a: 1.0,
    };

    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }
}

/// レンダラー構造体
pub struct Renderer {
    viewport_width: f32,
    viewport_height: f32,
}

impl Renderer {
    /// 新しいレンダラーを作成
    pub fn new(viewport_width: f32, viewport_height: f32) -> Self {
        Self {
            viewport_width,
            viewport_height,
        }
    }

    /// ビューポートサイズを更新
    pub fn resize(&mut self, width: f32, height: f32) {
        self.viewport_width = width;
        self.viewport_height = height;
    }

    /// DOM Treeから描画命令を生成
    pub fn generate_draw_commands(&self, dom_root: &NodeRef) -> Vec<DrawCommand> {
        let mut commands = Vec::new();
        let mut current_x = 10.0;
        let mut current_y = 10.0;

        self.traverse_and_generate(dom_root, &mut commands, &mut current_x, &mut current_y);

        commands
    }

    /// DOMツリーを走査して描画命令を生成（再帰的）
    fn traverse_and_generate(
        &self,
        node: &NodeRef,
        commands: &mut Vec<DrawCommand>,
        current_x: &mut f32,
        current_y: &mut f32,
    ) {
        let node_borrow = node.borrow();

        match &node_borrow.node_type {
            NodeType::Document => {
                // ドキュメントノードは子要素を処理
                for child in &node_borrow.children {
                    self.traverse_and_generate(child, commands, current_x, current_y);
                }
            }
            NodeType::Element { tag_name, .. } => {
                // 要素ノードの処理
                let line_height = 20.0;

                // ブロック要素の場合は改行
                if html_util::is_block_level_element(tag_name) {
                    *current_x = 10.0;
                    *current_y += line_height;
                }

                // 子要素を処理
                for child in &node_borrow.children {
                    self.traverse_and_generate(child, commands, current_x, current_y);
                }

                // ブロック要素の後は改行
                if html_util::is_block_level_element(tag_name) {
                    *current_x = 10.0;
                    *current_y += line_height / 2.0;
                }
            }
            NodeType::Text(text) => {
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
