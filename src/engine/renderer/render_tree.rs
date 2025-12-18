use super::render::Color;
use super::render_node::{Display, NodeKind, RenderNode, RenderTree};
use crate::engine::css::Length;
use crate::engine::html::{HtmlNodeType, util as html_util};
use crate::engine::styler::computed_tree::{ComputedStyleNode, ComputedTree};
use crate::engine::tree::{Tree, TreeNode};
use std::cell::RefCell;
use std::rc::Rc;

impl RenderTree {
    pub fn set_root_size(&mut self, w: f32, h: f32) {
        let mut root = self.root.borrow_mut();
        if let NodeKind::Scrollable { .. } = root.value.kind {
            root.value.width = w;
            root.value.height = h;
        }
    }

    /// RenderTree を再レイアウト
    pub fn layout_with_fallback(&mut self) {
        // 測定器が指定されていない場合はエンジンのフォールバックを使う
        let fallback = crate::engine::bridge::text::EngineFallbackTextMeasurer::default();
        self.layout_with_measurer(&fallback);
    }

    /// RenderTree を指定の TextMeasurer でレイアウト
    pub fn layout_with_measurer(
        &mut self,
        measurer: &dyn crate::engine::bridge::text::TextMeasurer,
    ) {
        let root_width = self.root.borrow().value.width;
        let root_height = self.root.borrow().value.height;
        Self::layout_node_with_measurer(&self.root, 0.0, 0.0, root_width, root_height, measurer);
    }

    /// 再帰的にノードをレイアウト
    fn layout_node_with_measurer(
        node: &Rc<RefCell<TreeNode<RenderNode>>>,
        start_x: f32,
        start_y: f32,
        available_width: f32,
        available_height: f32,
        measurer: &dyn crate::engine::bridge::text::TextMeasurer,
    ) -> (f32, f32) {
        // immutable borrow で子ノードをクローン
        let children: Vec<_> = {
            let node_ref = node.borrow();
            node_ref.children().clone()
        };

        // mutable borrow で自身の RenderNode にアクセス
        let mut node_ref = node.borrow_mut();
        let render_node = &mut node_ref.value;
        render_node.layout.available_width = available_width;

        match &mut render_node.kind {
            NodeKind::Container => {
                let mut x_offset = start_x;
                let mut y_offset = start_y;
                for child in children {
                    (x_offset, y_offset) = Self::layout_node_with_measurer(
                        &child,
                        x_offset,
                        y_offset,
                        available_width,
                        available_height,
                        measurer,
                    );
                    match render_node.display {
                        Display::Block => {
                            // ブロック要素は改行
                            x_offset = start_x;
                        }
                        Display::Inline => {
                            // インライン要素は横並び
                            y_offset = start_y;
                            // TODO: 折り返し処理
                        }
                        Display::None => {
                            // 非表示要素は無視
                        }
                    }
                }
                render_node.x = start_x;
                render_node.y = start_y;
                render_node.width = x_offset - start_x;
                render_node.height = y_offset - start_y;
                (start_x, start_y + render_node.height)
            }

            NodeKind::Scrollable { tree, .. } => {
                // Scrollable 内部を再帰レイアウト
                let _ = Self::layout_node_with_measurer(
                    &tree.root,
                    start_x,
                    start_y,
                    available_width,
                    available_height,
                    measurer,
                );
                render_node.x = start_x;
                render_node.y = start_y;
                render_node.width = available_width;
                render_node.height = render_node.height.max(available_height);
                (start_x, start_y + render_node.height)
            }

            NodeKind::Text {
                text,
                font_size,
                color: _,
            } => {
                // テキストノードは TextMeasurer でサイズを求める
                let req = crate::engine::bridge::text::TextMeasurementRequest {
                    text: text.clone(),
                    font: crate::engine::bridge::text::FontDescription {
                        family: None,
                        size_px: *font_size,
                    },
                    constraints: crate::engine::bridge::text::LayoutConstraints {
                        max_width: Some(available_width),
                        wrap: true,
                        max_lines: None,
                    },
                };

                if let Ok(meas) = measurer.measure(&req) {
                    render_node.width = meas.width;
                    render_node.height = meas.height;
                } else {
                    // 測定に失敗したら既存の値を使う
                    render_node.width = render_node.layout.preferred_width.unwrap_or(0.0);
                    render_node.height = render_node.layout.preferred_height.unwrap_or(20.0);
                }
                render_node.x = start_x;
                render_node.y = start_y;
                (start_x, start_y + render_node.height)
            }

            NodeKind::Button => {
                // preferred_width / preferred_height を使う
                render_node.width = render_node.layout.preferred_width.unwrap_or(0.0);
                render_node.height = render_node.layout.preferred_height.unwrap_or(20.0);
                render_node.x = start_x;
                render_node.y = start_y;
                (start_x, start_y + render_node.height)
            }

            NodeKind::Unknown => {
                // 不明なノードはサイズ0で無視
                render_node.width = 0.0;
                render_node.height = 0.0;
                render_node.x = start_x;
                render_node.y = start_y;
                (start_x, start_y)
            }
        }
    }

    /// ComputedTree から RenderTree を生成（レイアウト情報はここでは付けない）
    pub fn from_computed_tree(tree: &ComputedTree) -> RenderTree {
        let (root_kind, _display) = Self::detect_kind_display(&tree.root.borrow().value);
        let root_node = RenderNode::new(root_kind, Display::Block, 0.0, 0.0, 0.0, 0.0);
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
            Display::Block,
            0.0,
            0.0,
            0.0,
            0.0,
        );

        Tree::new(page_root_scrollable)
    }

    /// ComputedStyleNode から NodeKind を判定
    fn detect_kind_display(node: &ComputedStyleNode) -> (NodeKind, Display) {
        let computed_style = node.computed.clone().unwrap();
        let html = node.html.upgrade().unwrap();
        let html_ref = html.borrow();
        let kind = match &html_ref.value {
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
                _ if html_util::is_block_level_element(tag_name) => NodeKind::Container,
                _ if html_util::is_inline_element(tag_name) => NodeKind::Container,
                _ => {
                    log::warn!(target:"RenderTree::NodeKind", "Unknown element tag: {}", tag_name);
                    NodeKind::Unknown
                }
            },
            HtmlNodeType::Document => NodeKind::Container,
            _ => NodeKind::Unknown,
        };

        let display = Display::from_css_display(computed_style.display);
        if display.is_none() {
            return (NodeKind::Unknown, display);
        }
        (kind, display)
    }

    /// 再帰的に ComputedTree を RenderTree に変換（構造コピーのみ）
    fn convert_structure(
        src: &Rc<RefCell<TreeNode<ComputedStyleNode>>>,
        dst: &Rc<RefCell<TreeNode<RenderNode>>>,
    ) {
        for child in src.borrow().children() {
            let (kind, display) = Self::detect_kind_display(&child.borrow().value);
            let mut new_node = RenderNode::new(kind.clone(), display, 0.0, 0.0, 0.0, 0.0);

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

            // 再帰処理
            match kind {
                NodeKind::Unknown => {}
                _ => Self::convert_structure(child, &new_tree.root),
            }
        }
    }
}
