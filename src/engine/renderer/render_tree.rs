use super::Color;
use super::render_node::RenderNodeTrait;
use super::render_node::{NodeKind, RenderNode, RenderTree};
use crate::engine::bridge::text;
use crate::engine::css::values::Display;
use crate::engine::styler::computed_tree::{ComputedStyleNode, ComputedTree};
use crate::engine::tree::{Tree, TreeNode};
use crate::html::HtmlNodeType;
use core::panic;
use std::cell::RefCell;
use std::rc::Rc;

// デバッグ用の変数たち
#[cfg(debug_assertions)]
thread_local! {
    // レイアウトの再帰深度を追跡
    static LAYOUT_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl RenderTree {
    pub fn set_root_size(self, w: f32, h: f32) -> RenderTree {
        {
            let mut root = self.root.borrow_mut();
            // ルートを Scrollable で包まないため、ノード種別に関わらずルートサイズを設定する
            root.value.set_size(w, h);
        }
        RenderTree { root: self.root }
    }

    pub fn wrap_in_scrollable(self, x: f32, y: f32, w: f32, h: f32) -> RenderTree {
        let scrollable_node = RenderNode::new(
            NodeKind::Scrollable {
                tree: self,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
            },
            x,
            y,
            w,
            h,
        );
        let scrollable_tree = Tree::new(scrollable_node);
        RenderTree {
            root: scrollable_tree.root,
        }
    }

    /// ComputedTree から RenderTree を生成（フォールバック測定器）
    pub fn from_computed_tree(tree: &ComputedTree) -> RenderTree {
        let fallback = crate::engine::bridge::text::EngineFallbackTextMeasurer::default();
        Self::from_computed_tree_with_measurer(tree, &fallback, 0.0, 0.0)
    }

    /// ComputedTree から RenderTree を生成（指定の測定器を使用）
    pub fn from_computed_tree_with_measurer(
        tree: &ComputedTree,
        measurer: &dyn text::TextMeasurer,
        available_width: f32,
        available_height: f32,
    ) -> RenderTree {
        // まず構造だけを RenderTree にコピー
        let (root_kind, _display) = Self::detect_kind_display(&tree.root.borrow().value);
        let root_node = RenderNode::new(root_kind, 0.0, 0.0, 0.0, 0.0);
        let inner_render_tree = Tree::new(root_node);
        Self::convert_structure(&tree.root, &inner_render_tree.root);

        // ページルートはそのまま返す（Scrollable でラップしない）
        let render_tree = inner_render_tree;

        // 再帰レイアウト（ComputedTree の情報と測定器を用いる）
        Self::layout_node_recursive(
            &tree.root,
            &render_tree.root,
            0.0,
            0.0,
            available_width,
            available_height,
            measurer,
        );

        render_tree
    }

    /// ComputedStyleNode から NodeKind を判定（RenderNode 用）
    fn detect_kind_display(node: &ComputedStyleNode) -> (NodeKind, Option<Display>) {
        let computed_style = node.computed.clone().unwrap_or_default();
        let html = node.html.upgrade().unwrap();
        let html_ref = html.borrow();
        let kind = match &html_ref.value {
            HtmlNodeType::Text(t) => NodeKind::Text {
                text: t.clone(),
                font_size: computed_style
                    .font_size
                    .unwrap_or(crate::engine::css::values::Length::Px(19.0))
                    .to_px(10.0),
                color: Color::from_rgba_tuple(
                    computed_style.color.unwrap_or_default().to_rgba_tuple(None),
                ),
                max_width: 0.0,
            },
            HtmlNodeType::Element { tag_name, .. } => match tag_name.as_str() {
                "button" => NodeKind::Button,
                _ if crate::engine::html::util::is_block_level_element(tag_name) => {
                    NodeKind::Container
                }
                _ if crate::engine::html::util::is_inline_element(tag_name) => NodeKind::Container,
                _ => {
                    log::warn!(target:"RenderTree::NodeKind", "Unknown element tag: {}", tag_name);
                    NodeKind::Unknown
                }
            },
            HtmlNodeType::Document => NodeKind::Container,
            _ => NodeKind::Unknown,
        };

        let display = computed_style.display;
        (kind, Some(display))
    }

    /// 再帰的に ComputedTree を RenderTree に変換（構造コピーのみ）
    fn convert_structure(
        src: &Rc<RefCell<TreeNode<ComputedStyleNode>>>,
        dst: &Rc<RefCell<TreeNode<RenderNode>>>,
    ) {
        for child in src.borrow().children() {
            let (kind, _display) = Self::detect_kind_display(&child.borrow().value);
            let new_node = RenderNode::new(kind.clone(), 0.0, 0.0, 0.0, 0.0);
            let new_tree = Tree::new(new_node);
            TreeNode::add_child(dst, Rc::clone(&new_tree.root));

            // 再帰処理
            match kind {
                NodeKind::Unknown => {}
                _ => Self::convert_structure(child, &new_tree.root),
            }
        }
    }

    /// 再帰的にノードをレイアウト（ComputedTree の情報を元に RenderTree のサイズ/位置を埋める）
    /// 返り値: (content_width, content_height)
    ///
    /// TODO:
    /// - padding, margin, border の考慮
    fn layout_node_recursive(
        src: &Rc<RefCell<TreeNode<ComputedStyleNode>>>,
        node: &Rc<RefCell<TreeNode<RenderNode>>>,
        start_x: f32,
        start_y: f32,
        mut available_width: f32,
        available_height: f32,
        measurer: &dyn text::TextMeasurer,
    ) -> (f32, f32) {
        // 対応する子をペアで巡回するために src/dst の子を取得
        let src_borrow = src.borrow();
        let src_children: &Vec<_> = src_borrow.children();
        let dst_children: Vec<_> = {
            let r = node.borrow();
            r.children().clone()
        };

        let mut node_ref = node.borrow_mut();
        let render_node = &mut node_ref.value;

        // デバッグ用ログ
        #[cfg(debug_assertions)]
        LAYOUT_DEPTH.with(|d| {
            log::debug!(target: "RenderTree::layout_node_recursive", "{:?}: {} {:?} Start", d.get(), render_node.kind(), src_borrow.value.computed.as_ref().map(|c| c.display).unwrap());
        });

        match &mut render_node.kind_mut() {
            NodeKind::Container => {
                let mut x_offset = start_x;
                let mut y_offset = start_y;
                let mut width: f32 = 0.0;
                let mut height: f32 = 0.0;
                let origin_available_width = available_width;
                #[cfg(debug_assertions)]
                LAYOUT_DEPTH.with(|d| {
                    d.set(d.get() + 1);
                    log::debug!(target: "RenderTree::layout_node_recursive", "  Laying out Container node with {} children", src_children.len());
                });
                for (s_child, d_child) in src_children.iter().zip(dst_children.iter()) {
                    let (child_w, child_h) = Self::layout_node_recursive(
                        s_child,
                        d_child,
                        x_offset,
                        y_offset,
                        available_width,
                        available_height,
                        measurer,
                    );

                    // 表示種別は ComputedStyle から取得
                    if let Some(computed) = s_child.borrow().value.computed.as_ref() {
                        let disp = computed.display;
                        match disp {
                            Display::Block => {
                                y_offset += child_h;
                                x_offset = start_x;
                                width = width.max(child_w);
                            }
                            Display::Inline => {
                                x_offset += child_w;
                                height = height.max(child_h);
                                available_width -= child_w;
                                if x_offset - start_x > origin_available_width {
                                    // 折り返し
                                    x_offset = start_x + child_w;
                                    y_offset += child_h;
                                    available_width = origin_available_width;
                                    // 子供も改行
                                    d_child.borrow_mut().value.set_position(start_x, y_offset);
                                }

                            }
                            Display::None => {}
                        }
                    } else {
                        panic!(
                            "ComputedStyle missing for node during layout: {:?}; Should not happen",
                            s_child.borrow().value
                        );
                    }
                }
                render_node.set_layout(
                    start_x,
                    start_y,
                    width.max(x_offset - start_x),
                    height.max(y_offset - start_y),
                );
                #[cfg(debug_assertions)]
                LAYOUT_DEPTH.with(|d| {
                    d.set(d.get() - 1);
                });
            }

            NodeKind::Scrollable { tree, .. } => {
                // Scrollable の内部は同じ ComputedTree のルートを使ってレイアウト
                let _ = Self::layout_node_recursive(
                    src,
                    &tree.root,
                    start_x,
                    start_y,
                    available_width,
                    available_height,
                    measurer,
                );
                render_node.set_layout(
                    start_x,
                    start_y,
                    available_width,
                    render_node.size().1.max(available_height),
                );
            }

            NodeKind::Text {
                text,
                font_size,
                color: _,
                max_width,
            } => {
                *max_width = available_width;
                let req = text::TextMeasurementRequest {
                    text: text.clone(),
                    font: text::FontDescription {
                        family: None,
                        size_px: *font_size,
                    },
                    constraints: text::LayoutConstraints {
                        max_width: Some(*max_width),
                        wrap: true,
                        max_lines: None,
                    },
                };
                let (width, height) = if let Ok(meas) = measurer.measure(&req) {
                    (meas.width, meas.height)
                } else if let Some(computed) = src.borrow().value.computed.as_ref() {
                    // サイズ解決は ComputedStyle の責務
                    (
                        computed
                            .resolved_width_px(available_width, *font_size)
                            .unwrap_or(0.0),
                        computed
                            .resolved_height_px(available_height, *font_size)
                            .unwrap_or(20.0),
                    )
                } else {
                    (0.0, 20.0)
                };
                render_node.set_layout(start_x, start_y, width, height);
            }

            NodeKind::Button => {
                let (width, height) = if let Some(computed) = src.borrow().value.computed.as_ref() {
                    (
                        computed
                            .resolved_width_px(available_width, 10.0)
                            .unwrap_or(0.0),
                        computed
                            .resolved_height_px(available_height, 10.0)
                            .unwrap_or(20.0),
                    )
                } else {
                    (0.0, 20.0)
                };
                render_node.set_layout(start_x, start_y, width, height);
            }

            NodeKind::Unknown => {
                render_node.set_layout(start_x, start_y, 0.0, 0.0);
            }
        }

        #[cfg(debug_assertions)]
        LAYOUT_DEPTH.with(|d| {
            log::debug!(target: "RenderTree::layout_node_recursive", "{:?}: Laid out node: {} at {:?} size={:?}", d.get(), render_node.kind(), render_node.position(), render_node.size());
        });
        render_node.size()
    }
}
