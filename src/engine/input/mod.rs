//! 入力処理とヒットテスト。クリック位置の要素判定を行う。

use std::rc::Rc;

use super::layouter::types::{InfoNode, NodeKind};
use super::ui::custom_node::CustomNode;
use super::ui::get_custom_inline_result;
use super::ui::text_input_types::TextInputEvent;
use ui_layout::LayoutNode;

/// ヒットしたノード情報
pub struct HitItem<'a> {
    pub layout: &'a LayoutNode,
    pub info: &'a InfoNode,
}

/// ヒットパス（子→親の順）
pub type HitPath<'a> = Vec<HitItem<'a>>;

/// x, y: グローバル座標
pub fn hit_test<'a>(layout: &'a LayoutNode, info: &'a InfoNode, x: f32, y: f32) -> HitPath<'a> {
    // layout_boxes が空なら何もヒットしない
    if layout.layout_box.is_empty() {
        return Vec::new();
    }

    for box_model in layout
        .layout_box
        .iter()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        // 後ろの box が前面
        let rect = box_model.padding_box;

        // 1. rect 外なら次の box へ
        if x < rect.x || y < rect.y || x > rect.x + rect.width || y > rect.y + rect.height {
            continue;
        }

        // 2. ローカル座標に変換（スクロールオフセット考慮）
        let mut local_x = x - box_model.content_box.x;
        let mut local_y = y - box_model.content_box.y;

        if let NodeKind::Container {
            scroll_offset_x,
            scroll_offset_y,
            ..
        }
        | NodeKind::Custom {
            scroll_offset_x,
            scroll_offset_y,
            ..
        } = &info.kind
        {
            local_x += *scroll_offset_x;
            local_y += *scroll_offset_y;
        }

        // 3. 子ノードを前面から探索
        for (child_layout, child_info) in layout.children.iter().zip(&info.children).rev() {
            if let Some(child_node) = child_layout.node() {
                let mut path = hit_test(child_node, child_info, local_x, local_y);
                if !path.is_empty() {
                    // 子がヒット → 自分を末尾に追加
                    path.push(HitItem { layout, info });
                    return path;
                }
            } else if child_layout.object().is_some()
                && let NodeKind::Custom {
                    layout_id: Some(layout_id),
                    ..
                } = &child_info.kind
                && let Some(result) = get_custom_inline_result(*layout_id)
                && result.spans.iter().any(|span| {
                    local_x >= span.x_range.start
                        && local_x <= span.x_range.end
                        && local_y >= span.line_pos.1
                        && local_y <= span.line_pos.1 + result.height
                })
            {
                return vec![
                    HitItem {
                        layout,
                        info: child_info,
                    },
                    HitItem { layout, info },
                ];
            }
        }

        // 4. 子ノードに当たらなければこの box がヒット
        return vec![HitItem { layout, info }];
    }

    // どの box にもヒットしなかった
    Vec::new()
}

/// Focuses `target` and clears focus from every other text input.
///
/// Returns whether a text input received focus.
pub fn focus_text_input(info: &InfoNode, target: Option<&Rc<dyn CustomNode>>) -> bool {
    let mut focused = false;
    if let NodeKind::Custom { node, .. } = &info.kind
        && node.accepts_text_input()
    {
        let is_target = target.is_some_and(|target| Rc::ptr_eq(node, target));
        node.set_focused(is_target);
        focused |= is_target;
    }
    for child in &info.children {
        focused |= focus_text_input(child, target);
    }
    focused
}

/// Sends an editing event to the focused text input, if one exists.
pub fn dispatch_text_input(info: &InfoNode, event: TextInputEvent) -> bool {
    if let NodeKind::Custom { node, .. } = &info.kind
        && node.accepts_text_input()
        && node.is_focused()
    {
        return node.handle_text_input(event);
    }
    for child in &info.children {
        if dispatch_text_input(child, event.clone()) {
            return true;
        }
    }
    false
}

/// Returns whether the focused text input has an active IME composition.
pub fn focused_text_input_is_composing(info: &InfoNode) -> bool {
    if let NodeKind::Custom { node, .. } = &info.kind
        && node.accepts_text_input()
        && node.is_focused()
    {
        return node.is_composing();
    }
    info.children.iter().any(focused_text_input_is_composing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::bridge::text::FallbackTextMeasurer;
    use crate::engine::layouter::types::{ContainerStyle, TextStyle};
    use crate::engine::ui::text_input::TextInputComponent;
    use crate::engine::ui::text_input_types::TextInputEvent;
    use std::sync::Arc;

    fn input_info(node: Rc<dyn CustomNode>) -> InfoNode {
        InfoNode {
            kind: NodeKind::Custom {
                node,
                scroll_x: false,
                scroll_y: false,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                style: ContainerStyle::default(),
                layout_style: ui_layout::Style::default(),
                text_style: TextStyle::default(),
                layout_id: None,
            },
            children: Vec::new(),
        }
    }

    #[test]
    fn focus_and_dispatch_target_one_input() {
        let measurer = Arc::new(FallbackTextMeasurer::default());
        let first: Rc<dyn CustomNode> = Rc::new(TextInputComponent::new("", "", measurer.clone()));
        let second: Rc<dyn CustomNode> = Rc::new(TextInputComponent::new("", "", measurer));
        let root = InfoNode {
            kind: NodeKind::LineBreak,
            children: vec![
                input_info(Rc::clone(&first)),
                input_info(Rc::clone(&second)),
            ],
        };

        assert!(focus_text_input(&root, Some(&second)));
        assert!(!first.is_focused());
        assert!(second.is_focused());
        assert!(dispatch_text_input(
            &root,
            TextInputEvent::Commit("日本".into())
        ));
    }
}
