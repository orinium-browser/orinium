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
        let render_tree = Tree::new(RenderNode::new(root_kind, 0.0, 0.0, 0.0, 0.0));
        // 子ノードを再帰的に変換
        Self::convert_node(&tree.root, &render_tree.root, 0.0);
        render_tree
    }

    /// ComputedStyleNode から NodeKind を判定
    fn detect_kind(node: &ComputedStyleNode) -> NodeKind {
        let html = node.html.upgrade().unwrap();
        let html_ref = html.borrow();
        match &html_ref.value {
            // テキストノードなら NodeKind::Text に
            HtmlNodeType::Text(t) => NodeKind::Text(t.clone()),
            // Element ノードならタグ名で判定
            HtmlNodeType::Element { tag_name, .. } => match tag_name.as_str() {
                "button" => NodeKind::Button,
                // 将来的に Scrollable などを追加可能
                _ if html_util::is_block_level_element(tag_name) => NodeKind::Block,
                _ if html_util::is_inline_element(tag_name) => NodeKind::Inline,
                _ => NodeKind::Unknown,
            },
            HtmlNodeType::Document => NodeKind::Block,
            // それ以外は Unknown
            _ => NodeKind::Unknown,
        }
    }

    /// 再帰的に ComputedTree を RenderTree に変換
    fn convert_node(
        src: &Rc<RefCell<TreeNode<ComputedStyleNode>>>,
        dst: &Rc<RefCell<TreeNode<RenderNode>>>,
        mut pos_y: f32,
    ) {
        // 現在のノードの種類を設定
        let kind = Self::detect_kind(&src.borrow().value);
        dst.borrow_mut().value.kind = kind.clone();

        match kind {
            NodeKind::Block | NodeKind::Inline => {
                // 子ノードを再帰的に変換
                for child in src.borrow().children() {
                    let child_value = &child.borrow().value;
                    let computed = &child_value.computed.clone().unwrap();
                    let child_kind = Self::detect_kind(child_value);
                    let new_node = RenderNode::new(child_kind, 0.0, pos_y, 0.0, 0.0);
                    pos_y += computed.height.unwrap_or(Length::Px(0.0)).to_px(10.0);
                    let new_tree = Tree::new(new_node);
                    // ツリーに子を追加
                    TreeNode::add_child(dst, Rc::clone(&new_tree.root));
                    // 再帰的に変換
                    Self::convert_node(child, &new_tree.root, pos_y);
                }
            }
            _ => { /* 他のノードは子供への処理はしない */ }
        }
    }
}
