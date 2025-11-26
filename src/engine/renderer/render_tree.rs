use super::render::Color;
use super::render_node::{NodeKind, RenderNode, RenderTree};
use crate::engine::css::Length;
use crate::engine::html::{HtmlNodeType, util as html_util};
use crate::engine::styler::computed_tree::{ComputedStyleNode, ComputedTree};
use crate::engine::tree::{Tree, TreeNode};
use std::cell::RefCell;
use std::rc::Rc;

impl RenderTree {
    /// ComputedTree から RenderTree を生成
    pub fn from_computed_tree(tree: &ComputedTree) -> RenderTree {
        // ルートノードの種類を判定
        let root_kind = Self::detect_kind(&tree.root.borrow().value);
        // RenderNode を作成してツリーのルートとする
        let inner_render_tree = Tree::new(RenderNode::new(root_kind, 0.0, 0.0, 0.0, 0.0));
        // 子ノードを再帰的に変換
        let _ = Self::convert_node(&tree.root, &inner_render_tree.root, 0.0, 0.0);

        let page_root_scrollable = RenderNode::new(
            NodeKind::Scrollable {
                tree: inner_render_tree,
                scroll_offset_y: 0.0,
                scroll_offset_x: 0.0,
            },
            0.0,
            0.0,
            600.0,
            400.0,
        );
        let render_tree = Tree::new(page_root_scrollable);

        render_tree
    }

    /// ComputedStyleNode から NodeKind を判定
    fn detect_kind(node: &ComputedStyleNode) -> NodeKind {
        let computed_style = node.computed.clone().unwrap();
        let html = node.html.upgrade().unwrap();
        let html_ref = html.borrow();
        match &html_ref.value {
            // テキストノードなら NodeKind::Text に
            HtmlNodeType::Text(t) => NodeKind::Text {
                text: t.clone(),
                font_size: computed_style
                    .font_size
                    .unwrap_or(Length::Px(19.0))
                    .to_px(10.0),
                color: Color::from_rgba_tuple(
                    computed_style.color.unwrap_or_default().to_rgba_tuple(None),
                ),
            },
            // Element ノードならタグ名で判定
            HtmlNodeType::Element { tag_name, .. } => match tag_name.as_str() {
                "button" => NodeKind::Button,
                // 将来的に Scrollable などを追加可能
                _ if html_util::is_block_level_element(tag_name) => NodeKind::Block,
                _ if html_util::is_inline_element(tag_name) => NodeKind::Inline,
                _ => {
                    log::warn!(target:"RenderTree::NodeKind", "Unknown element tag: {}", tag_name);
                    // println!("Unknown element tag: {}", tag_name);
                    NodeKind::Unknown
                }
            },
            HtmlNodeType::Document => NodeKind::Block,
            // それ以外は Unknown
            _ => NodeKind::Unknown,
        }
    }

    /// 再帰的に ComputedTree を RenderTree に変換
    /// ブロックの高さを合計して親の pos_y を更新
    fn convert_node(
        src: &Rc<RefCell<TreeNode<ComputedStyleNode>>>,
        dst: &Rc<RefCell<TreeNode<RenderNode>>>,
        mut pos_x: f32,
        mut pos_y: f32,
    ) -> f32 {
        let kind = Self::detect_kind(&src.borrow().value);
        dst.borrow_mut().value.kind = kind.clone();

        if matches!(kind, NodeKind::Block | NodeKind::Inline) {
            for child in src.borrow().children() {
                let child_value = &child.borrow().value;
                let computed = child_value.computed.as_ref().unwrap();
                let child_kind = Self::detect_kind(child_value);

                let new_node = RenderNode::new(child_kind.clone(), pos_x, pos_y, 0.0, 0.0);
                let new_tree = Tree::new(new_node);
                TreeNode::add_child(dst, Rc::clone(&new_tree.root));

                let (child_pos_x, child_pos_y) = match child_kind {
                    NodeKind::Block => (0.0, pos_y),
                    NodeKind::Inline => (pos_x, pos_y),
                    _ => (pos_x, pos_y),
                };

                // 再帰呼び出しして子ノードの最大 y を取得
                let child_bottom_y =
                    Self::convert_node(child, &new_tree.root, child_pos_x, child_pos_y);

                match child_kind {
                    NodeKind::Block => {
                        // ブロックの場合、pos_y を子の下端に更新して次のブロックの開始位置に
                        pos_y = child_bottom_y;
                        pos_x = 0.0; // ブロック後は横位置リセット
                    }
                    NodeKind::Inline => {
                        // インラインは横に積むだけ
                        pos_x += computed.width.unwrap_or(Length::Px(0.0)).to_px(10.0);
                        pos_y = pos_y.max(child_bottom_y); // 子の高さが親より大きい場合調整
                    }
                    _ => {}
                }
            }
        }

        // 現在ノードの高さを加えて返す
        if let Some(computed) = src.borrow().value.computed.as_ref() {
            pos_y + computed.height.unwrap_or(Length::Px(0.0)).to_px(10.0)
        } else {
            pos_y
        }
    }
}
