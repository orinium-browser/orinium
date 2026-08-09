//! 入力処理とヒットテスト。クリック位置の要素判定を行う。

use std::sync::Arc;

use super::layouter::types::{InfoNode, NodeKind};
use super::ui::PointerEvent;
use super::ui::custom_node::CustomNode;
use super::ui::input_text_types::InputTextEvent;
use ui_layout::{LayoutNode, Position};
/// ヒットしたノード情報
pub struct HitItem<'a> {
    pub layout: &'a LayoutNode,
    pub info: &'a InfoNode,
}

/// ヒットパス（子→親の順）
pub type HitPath<'a> = Vec<HitItem<'a>>;

/// Returns the innermost custom node on a hit path, if any.
///
/// The hit path is ordered child→parent, so the first `Custom` node found is
/// the deepest node under the pointer.
pub fn hit_custom_node<'a>(path: &'a HitPath<'a>) -> Option<&'a Arc<dyn CustomNode>> {
    path.iter().find_map(|hit| match &hit.info.kind {
        NodeKind::Custom { node, .. } => Some(node),
        _ => None,
    })
}

/// Returns the innermost DOM node id on a hit path, if any.
///
/// The hit path is ordered child→parent, so the first node carrying a
/// [`InfoNode::dom_id`] is the deepest DOM-backed element under the pointer.
pub fn hit_dom_id(path: &HitPath<'_>) -> Option<u32> {
    path.iter().find_map(|hit| hit.info.dom_id)
}

/// Dispatches a pointer event to the innermost custom node on the hit path.
pub fn dispatch_pointer(path: &HitPath<'_>, event: PointerEvent) -> bool {
    hit_custom_node(path).is_some_and(|node| node.on_pointer_event(event))
}

/// Updates the hover state of custom nodes after a pointer move.
///
/// Clears hover from the previously hovered node (if different) and sets it on
/// the node under the pointer. Returns whether the hover target changed.
pub fn update_hover(path: &HitPath<'_>, previous: Option<&Arc<dyn CustomNode>>) -> bool {
    let current = hit_custom_node(path);
    match (previous, current) {
        (Some(prev), Some(curr)) if Arc::ptr_eq(prev, curr) => false,
        (Some(prev), _) => {
            prev.set_hovered(false);
            if let Some(curr) = current {
                curr.set_hovered(true);
            }
            true
        }
        (None, Some(curr)) => {
            curr.set_hovered(true);
            true
        }
        (None, None) => false,
    }
}

/// x, y: グローバル座標
pub fn hit_test<'a>(layout: &'a LayoutNode, info: &'a InfoNode, x: f32, y: f32) -> HitPath<'a> {
    hit_test_inner(layout, info, x, y, (0.0, 0.0))
}

fn hit_test_inner<'a>(
    layout: &'a LayoutNode,
    info: &'a InfoNode,
    mut x: f32,
    mut y: f32,
    accumulated_scroll: (f32, f32),
) -> HitPath<'a> {
    // layout_boxes が空なら何もヒットしない
    if layout.layout_box.is_empty() {
        return Vec::new();
    }

    let is_fixed = layout.style.position.kind == Position::Fixed;
    if is_fixed {
        x -= accumulated_scroll.0;
        y -= accumulated_scroll.1;
    }

    let own_scroll = info.kind.scroll_offsets();
    let child_scroll = if is_fixed {
        own_scroll
    } else {
        (
            accumulated_scroll.0 + own_scroll.0,
            accumulated_scroll.1 + own_scroll.1,
        )
    };

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

        local_x += own_scroll.0;
        local_y += own_scroll.1;

        // 3. 子ノードを前面から探索
        for (child_layout, child_info) in layout.children.iter().zip(&info.children).rev() {
            if let Some(child_node) = child_layout.node() {
                let mut path =
                    hit_test_inner(child_node, child_info, local_x, local_y, child_scroll);
                if !path.is_empty() {
                    // 子がヒット → 自分を末尾に追加
                    path.push(HitItem { layout, info });
                    return path;
                }
            } else if let Some(result) = child_layout.custom_result()
                && result.spans.iter().any(|span| {
                    local_x >= span.x_range.start
                        && local_x <= span.x_range.end
                        && local_y >= span.line_pos.1
                        && local_y <= span.line_pos.1 + result.box_model.content_box.height
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

/// Scrolls the deepest scrollable container under `(x, y)` by `(dx, dy)`.
///
/// Mirrors [`hit_test`]: boxes are tested front-to-back and children are
/// visited before the node itself, so the innermost container wins. Only
/// nodes whose [`NodeKind::Container`] / [`NodeKind::Custom`] flags enable
/// scrolling for an axis are scrolled, clamped to the scrollable range
/// (`children_box` extent minus the visible `content_box`).
///
/// Returns whether any scroll offset actually changed. Callers can use a
/// `false` result to chain the wheel event to an ancestor (e.g. the root).
pub fn scroll_at(
    layout: &LayoutNode,
    info: &mut InfoNode,
    viewport: (f32, f32),
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
) -> bool {
    scroll_at_inner(layout, info, viewport, x, y, dx, dy, (0.0, 0.0))
}

fn scroll_at_inner(
    layout: &LayoutNode,
    info: &mut InfoNode,
    viewport: (f32, f32),
    mut x: f32,
    mut y: f32,
    dx: f32,
    dy: f32,
    accumulated_scroll: (f32, f32),
) -> bool {
    if layout.layout_box.is_empty() {
        return false;
    }

    let is_fixed = layout.style.position.kind == Position::Fixed;
    if is_fixed {
        x -= accumulated_scroll.0;
        y -= accumulated_scroll.1;
    }

    let own_scroll = info.kind.scroll_offsets();
    let child_scroll = if is_fixed {
        own_scroll
    } else {
        (
            accumulated_scroll.0 + own_scroll.0,
            accumulated_scroll.1 + own_scroll.1,
        )
    };

    for box_model in layout
        .layout_box
        .iter()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let rect = box_model.padding_box;
        if x < rect.x || y < rect.y || x > rect.x + rect.width || y > rect.y + rect.height {
            continue;
        }

        let mut local_x = x - box_model.content_box.x;
        let mut local_y = y - box_model.content_box.y;
        local_x += own_scroll.0;
        local_y += own_scroll.1;

        for (child_layout, child_info) in layout.children.iter().zip(&mut info.children).rev() {
            if let Some(child_node) = child_layout.node()
                && scroll_at_inner(
                    child_node,
                    child_info,
                    viewport,
                    local_x,
                    local_y,
                    dx,
                    dy,
                    child_scroll,
                )
            {
                return true;
            }
        }

        let scrolled = match &mut info.kind {
            NodeKind::Container {
                scroll_x,
                scroll_y,
                scroll_offset_x,
                scroll_offset_y,
                ..
            }
            | NodeKind::Custom {
                scroll_x,
                scroll_y,
                scroll_offset_x,
                scroll_offset_y,
                ..
            } => {
                let mut changed = false;

                let (vw, vh) = viewport;

                if *scroll_y {
                    let max_scroll = (box_model.children_box.height
                        - box_model.content_box.height.min(vh))
                    .max(0.0);
                    let next = (*scroll_offset_y + dy).clamp(0.0, dbg!(max_scroll));
                    if (next - *scroll_offset_y).abs() > f32::EPSILON {
                        changed = true;
                    }
                    *scroll_offset_y = next;
                }
                if *scroll_x {
                    let max_scroll = (box_model.children_box.width
                        - box_model.content_box.width.min(vw))
                    .max(0.0);
                    let next = (*scroll_offset_x + dx).clamp(0.0, max_scroll);
                    if (next - *scroll_offset_x).abs() > f32::EPSILON {
                        changed = true;
                    }
                    *scroll_offset_x = next;
                }
                changed
            }
            _ => false,
        };
        if scrolled {
            return true;
        }
    }

    false
}

/// Focuses `target` and clears focus from every other text input.
///
/// Returns whether a text input received focus.
pub fn focus_text_input(info: &InfoNode, target: Option<&Arc<dyn CustomNode>>) -> bool {
    let mut focused = false;
    if let NodeKind::Custom { node, .. } = &info.kind
        && node.accepts_text_input()
    {
        let is_target = target.is_some_and(|target| Arc::ptr_eq(node, target));
        node.set_focused(is_target);
        focused |= is_target;
    }
    for child in &info.children {
        focused |= focus_text_input(child, target);
    }
    focused
}

/// Sends an editing event to the focused text input, if one exists.
pub fn dispatch_text_input(info: &InfoNode, event: InputTextEvent) -> bool {
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

/// Returns whether any custom node in the tree reports a pending repaint.
///
/// Consumes the repaint flags of the nodes it visits, so callers should only
/// invoke this once per frame, right before deciding whether to redraw.
pub fn any_custom_node_needs_repaint(info: &InfoNode) -> bool {
    if let NodeKind::Custom { node, .. } = &info.kind
        && node.needs_repaint()
    {
        return true;
    }
    info.children.iter().any(any_custom_node_needs_repaint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::bridge::text::{FallbackTextMeasurer, TextMeasurer};
    use crate::engine::layouter::types::{Color, ContainerStyle, TextStyle};
    use crate::engine::ui::button::ButtonComponent;
    use crate::engine::ui::input_text::InputTextComponent;
    use crate::engine::ui::input_text_types::InputTextEvent;
    use std::sync::Arc;

    fn input_info(node: Arc<dyn CustomNode>) -> InfoNode {
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
            },
            children: Vec::new(),
            dom_id: None,
        }
    }

    #[test]
    fn focus_and_dispatch_target_one_input() {
        let measurer: Arc<dyn TextMeasurer<TextStyle>> = Arc::new(FallbackTextMeasurer);
        let first: Arc<dyn CustomNode> =
            Arc::new(InputTextComponent::new("", "", Arc::clone(&measurer)));
        let second: Arc<dyn CustomNode> = Arc::new(InputTextComponent::new("", "", measurer));
        let root = InfoNode {
            kind: NodeKind::LineBreak,
            children: vec![
                input_info(Arc::clone(&first)),
                input_info(Arc::clone(&second)),
            ],
            dom_id: None,
        };

        assert!(focus_text_input(&root, Some(&second)));
        assert!(!first.is_focused());
        assert!(second.is_focused());
        assert!(dispatch_text_input(
            &root,
            InputTextEvent::Commit("日本".into())
        ));
    }

    #[test]
    fn hit_custom_node_finds_innermost_custom() {
        let node: Arc<dyn CustomNode> = Arc::new(InputTextComponent::new(
            "",
            "",
            Arc::new(FallbackTextMeasurer),
        ));
        let info = input_info(Arc::clone(&node));
        let layout = LayoutNode::new(ui_layout::Style::default());
        let path = vec![
            HitItem {
                layout: &layout,
                info: &info,
            },
            HitItem {
                layout: &layout,
                info: &info,
            },
        ];
        assert!(Arc::ptr_eq(hit_custom_node(&path).unwrap(), &node));
    }

    #[test]
    fn update_hover_switches_target() {
        let measurer: Arc<dyn TextMeasurer<TextStyle>> = Arc::new(FallbackTextMeasurer);
        let a: Arc<dyn CustomNode> = Arc::new(ButtonComponent::new(
            "A",
            Color(0, 0, 0, 255),
            Color(255, 255, 255, 255),
            Arc::clone(&measurer),
        ));
        let b: Arc<dyn CustomNode> = Arc::new(ButtonComponent::new(
            "B",
            Color(0, 0, 0, 255),
            Color(255, 255, 255, 255),
            measurer,
        ));
        let info_a = input_info(Arc::clone(&a));
        let info_b = input_info(Arc::clone(&b));
        let layout = LayoutNode::new(ui_layout::Style::default());
        let path_a = vec![HitItem {
            layout: &layout,
            info: &info_a,
        }];
        let path_b = vec![HitItem {
            layout: &layout,
            info: &info_b,
        }];

        assert!(update_hover(&path_a, None));
        assert!(a.is_hovered());
        assert!(!b.is_hovered());

        assert!(update_hover(&path_b, Some(&a)));
        assert!(!a.is_hovered());
        assert!(b.is_hovered());

        assert!(!update_hover(&path_b, Some(&b)));
    }

    #[test]
    fn any_custom_node_needs_repaint_tracks_dirty_nodes() {
        let measurer: Arc<dyn TextMeasurer<TextStyle>> = Arc::new(FallbackTextMeasurer);
        let a: Arc<dyn CustomNode> = Arc::new(ButtonComponent::new(
            "A",
            Color(0, 0, 0, 255),
            Color(255, 255, 255, 255),
            Arc::clone(&measurer),
        ));
        let info_a = input_info(Arc::clone(&a));

        // Fresh nodes are dirty (initial paint).
        assert!(any_custom_node_needs_repaint(&info_a));
        assert!(!any_custom_node_needs_repaint(&info_a));

        // A pointer event that changes visual state marks it dirty again.
        a.on_pointer_event(PointerEvent::Down { x: 0.0, y: 0.0 });
        assert!(any_custom_node_needs_repaint(&info_a));
        assert!(!any_custom_node_needs_repaint(&info_a));
    }

    fn box_model(
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        children_width: f32,
        children_height: f32,
    ) -> ui_layout::BoxModel {
        let rect = ui_layout::Rect {
            x,
            y,
            width,
            height,
        };
        ui_layout::BoxModel {
            sticky_edges: None,
            border_box: rect,
            padding_box: rect,
            content_box: rect,
            children_box: ui_layout::Rect {
                x,
                y,
                width: children_width,
                height: children_height,
            },
        }
    }

    fn scrollable_info() -> InfoNode {
        InfoNode {
            kind: NodeKind::Container {
                scroll_x: false,
                scroll_y: true,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                style: ContainerStyle::default(),
                role: crate::engine::layouter::types::ContainerRole::Normal,
            },
            children: Vec::new(),
            dom_id: None,
        }
    }

    fn set_vertical_scroll(info: &mut InfoNode, offset: f32) {
        let NodeKind::Container {
            scroll_offset_y, ..
        } = &mut info.kind
        else {
            panic!("expected container");
        };
        *scroll_offset_y = offset;
    }

    fn fixed_child_layout(children_height: f32) -> LayoutNode {
        let mut style = ui_layout::Style::default();
        style.position.kind = Position::Fixed;
        let mut child = LayoutNode::new(style);
        child.layout_box = ui_layout::LayoutBox::BlockBox(box_model(
            10.0,
            10.0,
            30.0,
            20.0,
            30.0,
            children_height,
        ));
        child
    }

    #[test]
    fn hit_test_fixed_child_ignores_ancestor_scroll() {
        let mut layout =
            LayoutNode::with_children(ui_layout::Style::default(), [fixed_child_layout(20.0)]);
        layout.layout_box =
            ui_layout::LayoutBox::BlockBox(box_model(0.0, 0.0, 200.0, 100.0, 200.0, 300.0));

        let mut info = scrollable_info();
        set_vertical_scroll(&mut info, 50.0);
        let mut fixed_info = scrollable_info();
        fixed_info.dom_id = Some(42);
        info.children.push(fixed_info);

        let path = hit_test(&layout, &info, 15.0, 15.0);
        assert_eq!(hit_dom_id(&path), Some(42));
    }

    #[test]
    fn scroll_at_fixed_child_ignores_ancestor_scroll() {
        let mut layout =
            LayoutNode::with_children(ui_layout::Style::default(), [fixed_child_layout(80.0)]);
        layout.layout_box =
            ui_layout::LayoutBox::BlockBox(box_model(0.0, 0.0, 200.0, 100.0, 200.0, 300.0));

        let mut info = scrollable_info();
        set_vertical_scroll(&mut info, 50.0);
        info.children.push(scrollable_info());

        assert!(scroll_at(&layout, &mut info, 15.0, 15.0, 0.0, 10.0));
        let NodeKind::Container {
            scroll_offset_y: child_scroll,
            ..
        } = &info.children[0].kind
        else {
            panic!("expected fixed child container");
        };
        assert_eq!(*child_scroll, 10.0);
        let NodeKind::Container {
            scroll_offset_y: parent_scroll,
            ..
        } = &info.kind
        else {
            panic!("expected parent container");
        };
        assert_eq!(*parent_scroll, 50.0);
    }

    #[test]
    fn scroll_at_scrolls_under_point_clamped_to_range() {
        let mut layout = LayoutNode::new(ui_layout::Style::default());
        layout.layout_box =
            ui_layout::LayoutBox::BlockBox(box_model(0.0, 0.0, 200.0, 100.0, 200.0, 300.0));
        let mut info = scrollable_info();

        // Cursor inside the box. Positive dy scrolls down.
        assert!(scroll_at(&layout, &mut info, 50.0, 50.0, 0.0, 100.0));
        let NodeKind::Container {
            scroll_offset_y, ..
        } = &info.kind
        else {
            panic!("expected container");
        };
        assert_eq!(*scroll_offset_y, 100.0);

        // Clamp to children_box.height - content_box.height = 200.
        assert!(scroll_at(&layout, &mut info, 50.0, 50.0, 0.0, 300.0));
        let NodeKind::Container {
            scroll_offset_y, ..
        } = &info.kind
        else {
            panic!("expected container");
        };
        assert_eq!(*scroll_offset_y, 200.0);

        // Cannot scroll past 0 (negative dy scrolls up).
        assert!(scroll_at(&layout, &mut info, 50.0, 50.0, 0.0, -500.0));
        let NodeKind::Container {
            scroll_offset_y, ..
        } = &info.kind
        else {
            panic!("expected container");
        };
        assert_eq!(*scroll_offset_y, 0.0);
    }

    #[test]
    fn scroll_at_ignores_cursor_outside_box() {
        let mut layout = LayoutNode::new(ui_layout::Style::default());
        layout.layout_box =
            ui_layout::LayoutBox::BlockBox(box_model(0.0, 0.0, 200.0, 100.0, 200.0, 300.0));
        let mut info = scrollable_info();

        assert!(!scroll_at(&layout, &mut info, 250.0, 50.0, 0.0, -100.0));
    }

    #[test]
    fn scroll_at_ignores_non_scrollable_containers() {
        let mut layout = LayoutNode::new(ui_layout::Style::default());
        layout.layout_box =
            ui_layout::LayoutBox::BlockBox(box_model(0.0, 0.0, 200.0, 100.0, 200.0, 300.0));
        let mut info = InfoNode {
            kind: NodeKind::Container {
                scroll_x: false,
                scroll_y: false,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                style: ContainerStyle::default(),
                role: crate::engine::layouter::types::ContainerRole::Normal,
            },
            children: Vec::new(),
            dom_id: None,
        };

        assert!(!scroll_at(&layout, &mut info, 50.0, 50.0, 0.0, -100.0));
        let NodeKind::Container {
            scroll_offset_y, ..
        } = &info.kind
        else {
            panic!("expected container");
        };
        assert_eq!(*scroll_offset_y, 0.0);
    }

    #[test]
    fn scroll_at_prefers_innermost_scrollable_container() {
        // Outer container with scrollable content; inner container is
        // scrollable and sits inside it. A scroll over the inner container
        // should move the inner one, not the outer.
        let outer_children_box = box_model(0.0, 0.0, 400.0, 300.0, 400.0, 900.0);
        let inner_children_box = box_model(10.0, 10.0, 100.0, 80.0, 100.0, 240.0);

        let mut outer_layout = LayoutNode::with_children(
            ui_layout::Style::default(),
            [LayoutNode::new(ui_layout::Style::default())],
        );
        outer_layout.layout_box = ui_layout::LayoutBox::BlockBox(outer_children_box);

        let mut inner_layout = LayoutNode::new(ui_layout::Style::default());
        inner_layout.layout_box = ui_layout::LayoutBox::BlockBox(inner_children_box);

        outer_layout.children[0] = ui_layout::LayoutChild::Node(Box::new(inner_layout));

        let mut outer_info = scrollable_info();
        let inner_info = scrollable_info();
        outer_info.children.push(inner_info);

        // Cursor over the inner container. Positive dy scrolls down.
        assert!(scroll_at(
            &outer_layout,
            &mut outer_info,
            50.0,
            50.0,
            0.0,
            30.0
        ));
        let NodeKind::Container {
            scroll_offset_y, ..
        } = &outer_info.children[0].kind
        else {
            panic!("expected inner container");
        };
        assert_eq!(*scroll_offset_y, 30.0);
        let NodeKind::Container {
            scroll_offset_y: outer_off,
            ..
        } = &outer_info.kind
        else {
            panic!("expected outer container");
        };
        assert_eq!(*outer_off, 0.0);
    }
}
