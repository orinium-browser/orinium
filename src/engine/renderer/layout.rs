use std::cell::RefCell;
use std::rc::Rc;

use ui_layout::{Display, ItemStyle, LayoutNode, SizeStyle, Spacing, Style};

use crate::engine::bridge::text::{
    FontDescription, LayoutConstraints, TextMeasurementRequest, TextMeasurer,
};
use crate::engine::renderer::{NodeKind, RenderNode, render_node::RenderNodeTrait};
use crate::engine::tree::TreeNode;

/// RenderNode -> ui_layout::LayoutNode
pub fn render_to_layout(
    t_rn: &Rc<RefCell<TreeNode<RenderNode>>>,
    available_width: f32,
    measurer: &dyn TextMeasurer,
) -> LayoutNode {
    let rn = &t_rn.borrow().value;
    match &rn.kind() {
        NodeKind::Text {
            text, font_size, ..
        } => {
            // Text のサイズを測定
            let req = TextMeasurementRequest {
                text: text.clone(),
                font: FontDescription {
                    family: None,
                    size_px: *font_size,
                },
                constraints: LayoutConstraints {
                    max_width: Some(available_width),
                    wrap: true,
                    max_lines: None,
                },
            };
            let (w, h) = if let Ok(meas) = measurer.measure(&req) {
                (meas.width, meas.height)
            } else {
                (available_width, font_size * 1.2)
            };

            LayoutNode::new(Style {
                display: Display::Block,
                item_style: ItemStyle {
                    flex_grow: 0.0,
                    flex_basis: None,
                },
                size: SizeStyle {
                    width: Some(w),
                    height: Some(h),
                    min_width: None,
                    max_width: None,
                    min_height: None,
                    max_height: None,
                },
                spacing: Spacing::default(),
                justify_content: Default::default(),
                align_items: Default::default(),
                column_gap: 0.0,
                row_gap: 0.0,
            })
        }

        NodeKind::Button => {
            // ボタンは固定サイズで簡易対応
            LayoutNode::new(Style {
                display: Display::Block,
                item_style: ItemStyle {
                    flex_grow: 0.0,
                    flex_basis: None,
                },
                size: SizeStyle {
                    width: Some(100.0),
                    height: Some(30.0),
                    ..Default::default()
                },
                spacing: Spacing::default(),
                justify_content: Default::default(),
                align_items: Default::default(),
                column_gap: 0.0,
                row_gap: 0.0,
            })
        }

        NodeKind::Container => {
            // 子ノードを再帰的に変換
            let children: Vec<LayoutNode> = t_rn
                .borrow()
                .children()
                .iter()
                .map(|c| render_to_layout(c, available_width, measurer))
                .collect();

            LayoutNode::with_children(
                Style {
                    display: Display::Flex {
                        flex_direction: ui_layout::FlexDirection::Column,
                    },
                    item_style: ItemStyle {
                        flex_grow: 1.0,
                        flex_basis: None,
                    },
                    size: SizeStyle {
                        width: Some(available_width),
                        height: None,
                        ..Default::default()
                    },
                    spacing: Spacing::default(),
                    justify_content: Default::default(),
                    align_items: Default::default(),
                    column_gap: 0.0,
                    row_gap: 12.0,
                },
                children,
            )
        }
        NodeKind::Scrollable { tree, .. } => {
            let children = render_to_layout(&tree.root, available_width, measurer);

            LayoutNode::with_children(
                Style {
                    display: Display::Flex {
                        flex_direction: ui_layout::FlexDirection::Column,
                    },
                    item_style: ItemStyle {
                        flex_grow: 1.0,
                        flex_basis: None,
                    },
                    size: SizeStyle {
                        width: Some(available_width),
                        height: None,
                        ..Default::default()
                    },
                    spacing: Spacing::default(),
                    justify_content: Default::default(),
                    align_items: Default::default(),
                    column_gap: 0.0,
                    row_gap: 12.0,
                },
                vec![children],
            )
        }
        NodeKind::Unknown => LayoutNode::new(Style {
            display: Display::None,
            ..Default::default()
        }),
    }
}
