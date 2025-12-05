use super::render::Color;
use super::render_node::{NodeKind, RenderNode, RenderTree};
use crate::engine::css::Length;
use crate::engine::html::{HtmlNodeType, util as html_util};
use crate::engine::styler::computed_tree::{ComputedStyleNode, ComputedTree};
use crate::engine::tree::{Tree, TreeNode};
use std::cell::RefCell;
use std::rc::Rc;

impl RenderTree {
    /// ComputedTree から RenderTree を生成（レイアウト情報はここでは付けない）
    pub fn from_computed_tree(tree: &ComputedTree) -> RenderTree {
        let root_kind = Self::detect_kind(&tree.root.borrow().value);
        let root_node = RenderNode::new(root_kind, 0.0, 0.0, 0.0, 0.0);
        let inner_render_tree = Tree::new(root_node);

        // 子ノード構造だけをコピー
        Self::convert_structure(&tree.root, &inner_render_tree.root);

        // 最終的には Scrollable の中に入れる
        let page_root_scrollable = RenderNode::new(
            NodeKind::Scrollable {
                tree: inner_render_tree,
                scroll_offset_y: 0.0,
                scroll_offset_x: 0.0,
            },
            0.0,
            0.0,
            0.0,
            0.0,
        );

        Tree::new(page_root_scrollable)
    }

    /// ComputedStyleNode から NodeKind を判定
    fn detect_kind(node: &ComputedStyleNode) -> NodeKind {
        let computed_style = node.computed.clone().unwrap();
        let html = node.html.upgrade().unwrap();
        let html_ref = html.borrow();
        match &html_ref.value {
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
            HtmlNodeType::Element { tag_name, .. } => match tag_name.as_str() {
                "button" => NodeKind::Button,
                _ if html_util::is_block_level_element(tag_name) => NodeKind::Block,
                _ if html_util::is_inline_element(tag_name) => NodeKind::Inline,
                _ => {
                    log::warn!(target:"RenderTree::NodeKind", "Unknown element tag: {}", tag_name);
                    NodeKind::Unknown
                }
            },
            HtmlNodeType::Document => NodeKind::Block,
            _ => NodeKind::Unknown,
        }
    }

    /// 再帰的に ComputedTree を RenderTree に変換（構造コピーのみ）
    fn convert_structure(
        src: &Rc<RefCell<TreeNode<ComputedStyleNode>>>,
        dst: &Rc<RefCell<TreeNode<RenderNode>>>,
    ) {
        for child in src.borrow().children() {
            let mut new_node =
                RenderNode::new(Self::detect_kind(&child.borrow().value), 0.0, 0.0, 0.0, 0.0);

            // LayoutInfo に最低限のメタ情報をコピー
            if let Some(computed) = child.borrow().value.computed.as_ref() {
                new_node.layout.available_width = 0.0;
                new_node.layout.preferred_width =
                    computed.width.unwrap_or(Length::Auto).to_px_option(10.0);
                new_node.layout.preferred_height =
                    computed.height.unwrap_or(Length::Auto).to_px_option(10.0);
                new_node.layout.padding_left = computed.padding_left.to_px(10.0);
                new_node.layout.padding_right = computed.padding_right.to_px(10.0);
                new_node.layout.padding_top = computed.padding_top.to_px(10.0);
                new_node.layout.padding_bottom = computed.padding_bottom.to_px(10.0);
            }

            let new_tree = Tree::new(new_node);
            TreeNode::add_child(dst, Rc::clone(&new_tree.root));

            // 再帰
            Self::convert_structure(child, &new_tree.root);
        }
    }
}
