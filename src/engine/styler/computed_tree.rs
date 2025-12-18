use super::style_tree::Style;

use crate::engine::css::values::{Border, Color, Display, Length};
use crate::html::HtmlNodeType;
use std::cell::RefCell;
use std::rc::Rc;
use std::rc::Weak;

use crate::engine::bridge::text;
use crate::engine::renderer::render::Color as RenderColor;
use crate::engine::renderer::render_node::{NodeKind, RenderNode, RenderTree};
use crate::engine::tree::{Tree, TreeNode};

pub type ComputedTree = Tree<ComputedStyleNode>;

/// 計算済みスタイルを持つノード
#[derive(Debug, Clone)]
pub struct ComputedStyleNode {
    pub html: Weak<RefCell<TreeNode<HtmlNodeType>>>,
    pub computed: Option<ComputedStyle>,
}

impl ComputedTree {
    /// フォールバックの測定器でレイアウトを行い RenderTree を返す
    pub fn layout_with_fallback(&self, root_width: f32, root_height: f32) -> RenderTree {
        let fallback = text::EngineFallbackTextMeasurer::default();
        self.layout_with_measurer(&fallback, root_width, root_height)
    }

    /// 指定の TextMeasurer でレイアウトを行い RenderTree を返す
    pub fn layout_with_measurer(
        &self,
        measurer: &dyn text::TextMeasurer,
        root_width: f32,
        root_height: f32,
    ) -> RenderTree {
        // まず構造だけを RenderTree にコピー
        let (root_kind, _display) = Self::detect_kind_display(&self.root.borrow().value);
        let root_node = RenderNode::new(root_kind, 0.0, 0.0, 0.0, 0.0);
        let inner_render_tree = Tree::new(root_node);
        Self::convert_structure(&self.root, &inner_render_tree.root);

        // ページルートは Scrollable でラップ
        let page_root = RenderNode::new(
            NodeKind::Scrollable {
                tree: inner_render_tree,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
            },
            0.0,
            0.0,
            0.0,
            0.0,
        );
        let render_tree = Tree::new(page_root);

        // 再帰レイアウト（ComputedTree の情報と測定器を用いる）
        Self::layout_node_with_measurer(
            &self.root,
            &render_tree.root,
            0.0,
            0.0,
            root_width,
            root_height,
            measurer,
        );

        render_tree
    }

    /// ComputedStyleNode から NodeKind を判定（RenderNode 用）
    fn detect_kind_display(node: &ComputedStyleNode) -> (NodeKind, Display) {
        let computed_style = node
            .computed
            .clone()
            .unwrap_or_else(|| ComputedStyle::compute(super::style_tree::Style::default()));
        let html = node.html.upgrade().unwrap();
        let html_ref = html.borrow();
        let kind = match &html_ref.value {
            HtmlNodeType::Text(t) => NodeKind::Text {
                text: t.clone(),
                font_size: computed_style
                    .font_size
                    .unwrap_or(Length::Px(19.0))
                    .to_px(10.0),
                color: RenderColor::from_rgba_tuple(
                    computed_style.color.unwrap_or_default().to_rgba_tuple(None),
                ),
            },
            HtmlNodeType::Element { tag_name, .. } => match tag_name.as_str() {
                "button" => NodeKind::Button,
                _ if crate::engine::html::util::is_block_level_element(tag_name) => {
                    NodeKind::Container
                }
                _ if crate::engine::html::util::is_inline_element(tag_name) => NodeKind::Container,
                _ => {
                    log::warn!(target:"ComputedTree::NodeKind", "Unknown element tag: {}", tag_name);
                    NodeKind::Unknown
                }
            },
            HtmlNodeType::Document => NodeKind::Container,
            _ => NodeKind::Unknown,
        };

        let display = computed_style.display;
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
    fn layout_node_with_measurer(
        src: &Rc<RefCell<TreeNode<ComputedStyleNode>>>,
        node: &Rc<RefCell<TreeNode<RenderNode>>>,
        start_x: f32,
        start_y: f32,
        available_width: f32,
        available_height: f32,
        measurer: &dyn text::TextMeasurer,
    ) -> (f32, f32) {
        // 対応する子をペアで巡回するために src/dst の子を取得
        let src_children = src.borrow();
        let src_children: &Vec<_> = src_children.children();
        let dst_children: Vec<_> = {
            let r = node.borrow();
            r.children().clone()
        };

        let mut node_ref = node.borrow_mut();
        let render_node = &mut node_ref.value;

        match &mut render_node.kind {
            NodeKind::Container => {
                let mut x_offset = start_x;
                let mut y_offset = start_y;
                for (s_child, d_child) in src_children.iter().zip(dst_children.iter()) {
                    let (child_h, child_w) = Self::layout_node_with_measurer(
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
                            }
                            Display::Inline => {
                                x_offset += child_w;
                            }
                            Display::None => {}
                        }
                    } else {
                        // default: treat as block
                        y_offset += child_h;
                    }
                }
                render_node.x = start_x;
                render_node.y = start_y;
                render_node.width = x_offset - start_x;
                render_node.height = y_offset - start_y;
            }

            NodeKind::Scrollable { tree, .. } => {
                // Scrollable の内部は同じ ComputedTree のルートを使ってレイアウト
                let _ = Self::layout_node_with_measurer(
                    src,
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
            }

            NodeKind::Text {
                text,
                font_size,
                color: _,
            } => {
                let req = text::TextMeasurementRequest {
                    text: text.clone(),
                    font: text::FontDescription {
                        family: None,
                        size_px: *font_size,
                    },
                    constraints: text::LayoutConstraints {
                        max_width: Some(available_width),
                        wrap: true,
                        max_lines: None,
                    },
                };
                if let Ok(meas) = measurer.measure(&req) {
                    render_node.width = meas.width;
                    render_node.height = meas.height;
                } else if let Some(computed) = src.borrow().value.computed.as_ref() {
                    render_node.width = computed.width.unwrap_or(Length::Auto).to_px(10.0);
                    render_node.height = computed.height.unwrap_or(Length::Auto).to_px(10.0);
                } else {
                    render_node.width = 0.0;
                    render_node.height = 20.0;
                }
                render_node.x = start_x;
                render_node.y = start_y;
            }

            NodeKind::Button => {
                if let Some(computed) = src.borrow().value.computed.as_ref() {
                    render_node.width = computed.width.unwrap_or(Length::Auto).to_px(10.0);
                    render_node.height = computed.height.unwrap_or(Length::Auto).to_px(10.0);
                } else {
                    render_node.width = 0.0;
                    render_node.height = 20.0;
                }
                render_node.x = start_x;
                render_node.y = start_y;
            }

            NodeKind::Unknown => {
                render_node.width = 0.0;
                render_node.height = 0.0;
                render_node.x = start_x;
                render_node.y = start_y;
            }
        }

        (render_node.width, render_node.height)
    }
}

/// 計算済みスタイル
#[derive(Debug, Clone)]
pub struct ComputedStyle {
    pub display: Display,
    pub width: Option<Length>,
    pub height: Option<Length>,

    pub margin_top: Length,
    pub margin_right: Length,
    pub margin_bottom: Length,
    pub margin_left: Length,

    pub padding_top: Length,
    pub padding_right: Length,
    pub padding_bottom: Length,
    pub padding_left: Length,

    pub color: Option<Color>,
    pub background_color: Option<Color>,

    pub border: Option<Border>,

    pub font_size: Option<Length>,
}

impl ComputedStyle {
    /// Style から計算済みスタイルを作る
    pub fn compute(style: Style) -> Self {
        Self {
            display: style.display.unwrap_or(Display::Inline),
            width: style.width,
            height: style.height,

            // margin/padding は None の場合 0 にフォールバック
            margin_top: style.margin_top.unwrap_or(Length::Px(0.0)),
            margin_right: style.margin_right.unwrap_or(Length::Px(0.0)),
            margin_bottom: style.margin_bottom.unwrap_or(Length::Px(0.0)),
            margin_left: style.margin_left.unwrap_or(Length::Px(0.0)),

            padding_top: style.padding_top.unwrap_or(Length::Px(0.0)),
            padding_right: style.padding_right.unwrap_or(Length::Px(0.0)),
            padding_bottom: style.padding_bottom.unwrap_or(Length::Px(0.0)),
            padding_left: style.padding_left.unwrap_or(Length::Px(0.0)),

            color: style.color,
            background_color: style.background_color,

            border: style.border,

            font_size: style.font_size,
        }
    }
}
