//! 入力処理とヒットテスト。クリック位置の要素判定を行う。

use std::sync::Arc;

use crate::engine::layouter::types::TextFlowStyle;

use super::layouter::types::{InfoNode, NodeKind, TextStyle};
use super::ui::PointerEvent;
use super::ui::custom_node::CustomNode;
use super::ui::input_text_types::InputTextEvent;
use ui_layout::{LayoutNode, Position};
/// ヒットしたノード情報
#[derive(Clone, Debug)]
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

/// Converts page-space pointer coordinates into the local content-box space of
/// the custom node at `path[target]`.
///
/// The hit path is ordered child→parent, so each entry's `content_box` origin
/// is expressed in its *parent's* coordinate space. Mirroring
/// [`hit_test_inner`], the chain is walked outermost→innermost, subtracting
/// each level's content-box origin and adding its scroll offsets.
///
/// Entries that share their `LayoutNode` with the next outer entry (the
/// synthetic items `hit_test_inner` emits for an inline custom object) have
/// no coordinate frame of their own: the object was tested in its parent's
/// local coordinates, so they contribute no offset.
fn local_pointer_coords(path: &HitPath<'_>, target: usize, x: f32, y: f32) -> (f32, f32) {
    let mut lx = x;
    let mut ly = y;
    for (offset, hit) in path.iter().rev().take(path.len() - target).enumerate() {
        let index = path.len() - 1 - offset;
        let shares_parent_layout =
            index + 1 < path.len() && std::ptr::eq(hit.layout, path[index + 1].layout);
        // Inline Container boxes live in the parent's coordinate space and
        // push no transform (mirroring `push_box_model`), so they contribute
        // no offset of their own. Inline Custom nodes always push a transform,
        // so they must apply the offset.
        let is_inline_container =
            matches!(hit.layout.layout_box, ui_layout::LayoutBox::InlineBox(_))
                && matches!(&hit.info.kind, NodeKind::Container { .. });
        if shares_parent_layout || is_inline_container {
            continue;
        }
        let (cx, cy) = hit
            .layout
            .layout_box
            .iter()
            .next()
            .map_or((0.0, 0.0), |b| (b.content_box.x, b.content_box.y));
        let (sx, sy) = hit.info.kind.scroll_offsets();
        lx += sx - cx;
        ly += sy - cy;
    }
    (lx, ly)
}

/// Dispatches a pointer event to the innermost custom node on the hit path.
///
/// Event coordinates are translated from page space into the node's local
/// content-box space before delivery.
pub fn dispatch_pointer(path: &HitPath<'_>, event: PointerEvent) -> bool {
    for (target, hit) in path.iter().enumerate() {
        if let NodeKind::Custom {
            node,
            text_style,
            text_flow_style,
            ..
        } = &hit.info.kind
        {
            let (px, py) = match event {
                PointerEvent::Move { x, y } => (x, y),
                PointerEvent::Down { x, y } => (x, y),
                PointerEvent::Up { x, y } => (x, y),
                PointerEvent::Leave => (0.0, 0.0), // no coordinates for Leave
            };
            let (lx, ly) = local_pointer_coords(path, target, px, py);
            let local_event = match event {
                PointerEvent::Move { .. } => PointerEvent::Move { x: lx, y: ly },
                PointerEvent::Down { .. } => PointerEvent::Down { x: lx, y: ly },
                PointerEvent::Up { .. } => PointerEvent::Up { x: lx, y: ly },
                PointerEvent::Leave => PointerEvent::Leave,
            };

            // An open popup intercepts pointer events over it. Its rect is in
            // the same content-box space as the local coordinates.
            if let Some(popup) = node.popup(text_style, text_flow_style) {
                let in_popup = lx >= popup.rect.x
                    && ly >= popup.rect.y
                    && lx <= popup.rect.x + popup.rect.width
                    && ly <= popup.rect.y + popup.rect.height;
                // Popup events are expressed relative to the popup's own
                // top-left corner (`popup.rect` origin).
                let popup_event = match local_event {
                    PointerEvent::Move { x, y } => PointerEvent::Move {
                        x: x - popup.rect.x,
                        y: y - popup.rect.y,
                    },
                    PointerEvent::Down { x, y } => PointerEvent::Down {
                        x: x - popup.rect.x,
                        y: y - popup.rect.y,
                    },
                    PointerEvent::Up { x, y } => PointerEvent::Up {
                        x: x - popup.rect.x,
                        y: y - popup.rect.y,
                    },
                    PointerEvent::Leave => PointerEvent::Leave,
                };

                if in_popup {
                    return node.on_popup_pointer_event(popup_event);
                }
            }
            return node.on_pointer_event(local_event);
        }
    }
    false
}

/// Dismisses every open popup whose owner is not under the pointer press.
///
/// Implements top-layer dismissal: a press whose hit path already contains a
/// popup's owner is that owner's responsibility (the event is routed to the
/// popup, or to the owning box which closes it), while every other open popup
/// is closed. Returns whether any popup was dismissed.
pub fn dismiss_open_popups(info: &InfoNode, path: &HitPath<'_>) -> bool {
    let mut dismissed = false;
    dismiss_open_popups_inner(info, path, &mut dismissed);
    dismissed
}

fn dismiss_open_popups_inner(info: &InfoNode, path: &HitPath<'_>, dismissed: &mut bool) {
    if let NodeKind::Custom { node, .. } = &info.kind
        && node.popup(&TextStyle::default(), &TextFlowStyle::default()).is_some()
        && !path.iter().any(|hit| {
            matches!(&hit.info.kind, NodeKind::Custom { node: owner, .. } if Arc::ptr_eq(node, owner))
        })
    {
        node.dismiss_popup();
        *dismissed = true;
    }
    for child in &info.children {
        dismiss_open_popups_inner(child, path, dismissed);
    }
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

pub fn hit_test<'a>(layout: &'a LayoutNode, info: &'a InfoNode, x: f32, y: f32) -> HitPath<'a> {
    // Open popups are top-layer overlays: they render above every box and
    // escape all ancestor clips, so they are tested first and shadow the box
    // tree at their position.
    if let Some(path) = hit_test_popup(layout, info, x, y) {
        return path;
    }
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

    let is_inline = matches!(layout.layout_box, ui_layout::LayoutBox::InlineBox(_));

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

        // 2. ローカル座標に変換（スクロールオフセット考慮）。
        // Inline Container boxes live in the parent's coordinate space and
        // push no transform (mirroring `push_box_model`), so their children
        // keep the incoming coordinates untouched. Inline Custom nodes always
        // push a transform, so they apply the content-box offset.
        let is_inline_container = is_inline && matches!(&info.kind, NodeKind::Container { .. });
        let (local_x, local_y) = if is_inline_container {
            (x, y)
        } else {
            (
                x - box_model.content_box.x + own_scroll.0,
                y - box_model.content_box.y + own_scroll.1,
            )
        };

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
                    // `line_pos` positions the line in the parent's coordinate
                    // space (where inline content is laid out); `x_range` is
                    // only used for its width.
                    local_x >= span.line_pos.0
                        && local_x <= span.line_pos.0 + span.width()
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

/// Returns the hit path (child→parent) of the topmost open popup containing
/// `(x, y)`, or `None`.
///
/// Popups are top-layer overlays: they render above every box and escape all
/// ancestor clips, so they are hit-tested independently of box containment.
/// The scan mirrors [`hit_test_inner`]'s coordinate descent but skips the
/// padding-box checks; when several popups overlap, the one later in tree
/// order wins because it renders on top.
fn hit_test_popup<'a>(
    layout: &'a LayoutNode,
    info: &'a InfoNode,
    x: f32,
    y: f32,
) -> Option<HitPath<'a>> {
    let mut best: Option<HitPath<'a>> = None;
    let mut prefix: HitPath<'a> = Vec::new();
    hit_test_popup_inner(layout, info, x, y, (0.0, 0.0), &mut prefix, &mut best);
    best
}

/// Recursive top-layer popup scan. `prefix` holds the root→current hit path;
/// matches recorded later (tree order) overwrite earlier ones.
fn hit_test_popup_inner<'a>(
    layout: &'a LayoutNode,
    info: &'a InfoNode,
    mut x: f32,
    mut y: f32,
    accumulated_scroll: (f32, f32),
    prefix: &mut HitPath<'a>,
    best: &mut Option<HitPath<'a>>,
) {
    if layout.layout_box.is_empty() {
        return;
    }

    let is_inline = matches!(layout.layout_box, ui_layout::LayoutBox::InlineBox(_));

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

    // The popup rect lives in the node's content-box space, the same space as
    // `draw_sized`, which is anchored to the first layout box. Inline
    // Container boxes push no transform, so their content stays in the
    // parent's coordinate space. Inline Custom nodes always push a transform.
    let is_inline_container = is_inline && matches!(&info.kind, NodeKind::Container { .. });
    let (local_x, local_y) = if is_inline_container {
        (x, y)
    } else {
        layout.layout_box.iter().next().map_or((x, y), |b| {
            (
                x - b.content_box.x + own_scroll.0,
                y - b.content_box.y + own_scroll.1,
            )
        })
    };

    if let NodeKind::Custom {
        node,
        text_style,
        text_flow_style,
        ..
    } = &info.kind
        && let Some(popup) = node.popup(text_style, text_flow_style)
        && local_x >= popup.rect.x
        && local_y >= popup.rect.y
        && local_x <= popup.rect.x + popup.rect.width
        && local_y <= popup.rect.y + popup.rect.height
    {
        let mut path = prefix.clone();
        path.push(HitItem { layout, info });
        path.reverse();
        *best = Some(path);
    }

    prefix.push(HitItem { layout, info });
    for (child_layout, child_info) in layout.children.iter().zip(&info.children) {
        if let Some(child_node) = child_layout.node() {
            hit_test_popup_inner(
                child_node,
                child_info,
                local_x,
                local_y,
                child_scroll,
                prefix,
                best,
            );
        }
    }
    prefix.pop();
}

/// Marker returned from [`scroll_at`] when a container scrolled but carries no
/// snapshot dom id (so no `scroll` event can be dispatched to it). Callers can
/// treat any `Some(..)` as "scrolled" and use this value to skip dispatch.
pub const NO_SCROLL_DOM_ID: u32 = u32::MAX;

/// Scrolls the innermost scrollable container under `(x, y)` by `(dx, dy)`.
///
/// Mirrors [`hit_test`]: boxes are tested front-to-back and children are
/// visited before the node itself, so the innermost container wins. Only
/// nodes whose [`NodeKind::Container`] / [`NodeKind::Custom`] flags enable
/// scrolling for an axis are scrolled, clamped to the scrollable range
/// (`children_box` extent minus the visible `content_box`).
///
/// Returns `Some(dom_id)` when any scroll offset actually changed, where
/// `dom_id` names the scrollable container that absorbed the scroll. The value
/// is [`NO_SCROLL_DOM_ID`] when the scrolled container had no snapshot dom id
/// (used to trigger a redraw without dispatching a `scroll` event); `None` when
/// nothing scrolled, so a caller can chain the wheel event to an ancestor
/// (e.g. the root).
pub fn scroll_at(
    layout: &LayoutNode,
    info: &mut InfoNode,
    viewport: (f32, f32),
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
) -> Option<u32> {
    scroll_at_inner(layout, info, viewport, x, y, dx, dy, (0.0, 0.0))
}

#[allow(clippy::too_many_arguments)]
fn scroll_at_inner(
    layout: &LayoutNode,
    info: &mut InfoNode,
    viewport: (f32, f32),
    mut x: f32,
    mut y: f32,
    dx: f32,
    dy: f32,
    accumulated_scroll: (f32, f32),
) -> Option<u32> {
    if layout.layout_box.is_empty() {
        return None;
    }

    let is_inline = matches!(layout.layout_box, ui_layout::LayoutBox::InlineBox(_));

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

        // Inline Container boxes live in the parent's coordinate space and push
        // no transform (mirroring `push_box_model`), so their children keep
        // the incoming coordinates untouched. Inline Custom nodes always push
        // a transform, so they apply the content-box offset.
        let is_inline_container = is_inline && matches!(&info.kind, NodeKind::Container { .. });
        let (local_x, local_y) = if is_inline_container {
            (x, y)
        } else {
            (
                x - box_model.content_box.x + own_scroll.0,
                y - box_model.content_box.y + own_scroll.1,
            )
        };

        for (child_layout, child_info) in layout.children.iter().zip(&mut info.children).rev() {
            if let Some(child_node) = child_layout.node()
                && let Some(scrolled_id) = scroll_at_inner(
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
                return Some(scrolled_id);
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
                    let next = (*scroll_offset_y + dy).clamp(0.0, max_scroll);
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
            return Some(info.dom_id.unwrap_or(NO_SCROLL_DOM_ID));
        }
    }

    None
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
    use crate::engine::layouter::types::{Color, ContainerRole, ContainerStyle, TextStyle};
    use crate::engine::renderer_model::{DrawCommand, Rect};
    use crate::engine::ui::button::ButtonComponent;
    use crate::engine::ui::input_text::InputTextComponent;
    use crate::engine::ui::input_text_types::InputTextEvent;
    use crate::engine::ui::{ContentSize, Popup};
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use ui_layout::{LayoutChild, Style};

    const VIEWPORT_WIDTH: f32 = 800.0;
    const VIEWPORT_HEIGHT: f32 = 600.0;

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
                text_flow_style: TextFlowStyle::default(),
            },
            children: Vec::new(),
            dom_id: None,
        }
    }

    #[test]
    fn focus_and_dispatch_target_one_input() {
        let measurer: Arc<dyn TextMeasurer> = Arc::new(FallbackTextMeasurer);
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
        let measurer: Arc<dyn TextMeasurer> = Arc::new(FallbackTextMeasurer);
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
        let measurer: Arc<dyn TextMeasurer> = Arc::new(FallbackTextMeasurer);
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

    fn container_info(scroll_y: bool, dom_id: Option<u32>) -> InfoNode {
        InfoNode {
            kind: NodeKind::Container {
                scroll_x: false,
                scroll_y,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                style: ContainerStyle::default(),
                role: crate::engine::layouter::types::ContainerRole::Normal,
            },
            children: Vec::new(),
            dom_id,
        }
    }

    /// An inline container laid out in its parent's content space: its box
    /// model and line spans use absolute coordinates (mirroring how the flow
    /// engine positions inline content).
    fn inline_node(children: Vec<LayoutChild>) -> LayoutNode {
        let mut node = LayoutNode::with_children(ui_layout::Style::default(), children);
        node.layout_box = ui_layout::LayoutBox::InlineBox(ui_layout::InlineBox {
            box_model: box_model(10.0, 10.0, 200.0, 20.0, 200.0, 20.0),
            line_spans: vec![ui_layout::LineSpan {
                x_range: 0.0..200.0,
                line_pos: (10.0, 10.0),
                line_index: 0,
            }],
        });
        node
    }

    fn scroll_offset_y_of(info: &InfoNode) -> f32 {
        let NodeKind::Container {
            scroll_offset_y, ..
        } = &info.kind
        else {
            panic!("expected container");
        };
        *scroll_offset_y
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

        assert!(
            scroll_at(
                &layout,
                &mut info,
                (VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
                15.0,
                15.0,
                0.0,
                10.0
            )
            .is_some()
        );
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
        assert!(
            scroll_at(
                &layout,
                &mut info,
                (VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
                50.0,
                50.0,
                0.0,
                100.0
            )
            .is_some()
        );
        let NodeKind::Container {
            scroll_offset_y, ..
        } = &info.kind
        else {
            panic!("expected container");
        };
        assert_eq!(*scroll_offset_y, 100.0);

        // Clamp to children_box.height - content_box.height = 200.
        assert!(
            scroll_at(
                &layout,
                &mut info,
                (VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
                50.0,
                50.0,
                0.0,
                300.0
            )
            .is_some()
        );
        let NodeKind::Container {
            scroll_offset_y, ..
        } = &info.kind
        else {
            panic!("expected container");
        };
        assert_eq!(*scroll_offset_y, 200.0);

        // Cannot scroll past 0 (negative dy scrolls up).
        assert!(
            scroll_at(
                &layout,
                &mut info,
                (VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
                50.0,
                50.0,
                0.0,
                -500.0
            )
            .is_some()
        );
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

        assert!(
            scroll_at(
                &layout,
                &mut info,
                (VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
                250.0,
                50.0,
                0.0,
                -100.0
            )
            .is_none()
        );
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

        assert!(
            scroll_at(
                &layout,
                &mut info,
                (VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
                50.0,
                50.0,
                0.0,
                -100.0
            )
            .is_none()
        );
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
        assert!(
            scroll_at(
                &outer_layout,
                &mut outer_info,
                (VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
                50.0,
                50.0,
                0.0,
                30.0
            )
            .is_some()
        );
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

    #[test]
    fn scroll_at_reports_the_scrolled_containers_dom_id() {
        // Outer (dom 7) with scrollable inner (dom 9): scrolling over the inner
        // reports the inner's dom id.
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

        let mut outer_info = container_info(true, Some(7));
        let inner_info = container_info(true, Some(9));
        outer_info.children.push(inner_info);

        let scrolled = scroll_at(
            &outer_layout,
            &mut outer_info,
            (VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
            50.0,
            50.0,
            0.0,
            30.0,
        );
        assert_eq!(scrolled, Some(9));
    }

    #[test]
    fn scroll_at_reports_marker_when_scrolled_container_has_no_dom_id() {
        let mut layout = LayoutNode::new(ui_layout::Style::default());
        layout.layout_box =
            ui_layout::LayoutBox::BlockBox(box_model(0.0, 0.0, 200.0, 100.0, 200.0, 300.0));
        let mut info = container_info(true, None);
        let scrolled = scroll_at(
            &layout,
            &mut info,
            (VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
            50.0,
            50.0,
            0.0,
            10.0,
        );
        assert_eq!(scrolled, Some(NO_SCROLL_DOM_ID));
    }

    /// Records pointer events for asserting `dispatch_pointer` coordinates.
    #[derive(Debug)]
    struct RecordingNode {
        pointer_events: Mutex<Vec<PointerEvent>>,
        popup_events: Mutex<Vec<PointerEvent>>,
        popup_rect: Rect,
        popup_open: AtomicBool,
        dismissed: AtomicBool,
    }

    impl RecordingNode {
        fn new(popup_rect: Rect) -> Self {
            RecordingNode {
                pointer_events: Mutex::new(Vec::new()),
                popup_events: Mutex::new(Vec::new()),
                popup_rect,
                popup_open: AtomicBool::new(true),
                dismissed: AtomicBool::new(false),
            }
        }

        fn events(&self) -> Vec<PointerEvent> {
            self.pointer_events.lock().unwrap().clone()
        }

        fn popup_events(&self) -> Vec<PointerEvent> {
            self.popup_events.lock().unwrap().clone()
        }
    }

    impl CustomNode for RecordingNode {
        fn draw_sized(
            &self,
            _cmd_buf: &mut Vec<DrawCommand>,
            _text_style: &TextStyle,
            _text_flow_style: &TextFlowStyle,
            _style: &Style,
            _size: ContentSize,
        ) {
        }

        fn intrinsic_size(&self) -> ContentSize {
            ContentSize {
                width: 120.0,
                height: 28.0,
            }
        }

        fn on_pointer_event(&self, event: PointerEvent) -> bool {
            self.pointer_events.lock().unwrap().push(event);
            true
        }

        fn popup(
            &self,
            _text_style: &TextStyle,
            _text_flow_style: &TextFlowStyle,
        ) -> Option<Popup> {
            self.popup_open.load(Ordering::Relaxed).then(|| Popup {
                rect: self.popup_rect,
                commands: Vec::new(),
            })
        }

        fn on_popup_pointer_event(&self, event: PointerEvent) -> bool {
            self.popup_events.lock().unwrap().push(event);
            true
        }

        fn dismiss_popup(&self) {
            self.dismissed.store(true, Ordering::Relaxed);
        }
    }

    /// Tree: root (0,0,200,200) → container (10,20,160,100, scroll_y) →
    /// custom node (5,0,120,28) with an open popup at (0,28,120,84).
    fn make_tree(a_scroll_y: f32) -> (LayoutNode, InfoNode, Arc<RecordingNode>) {
        let node: Arc<RecordingNode> = Arc::new(RecordingNode::new(Rect {
            x: 0.0,
            y: 28.0,
            width: 120.0,
            height: 84.0,
        }));
        let custom_info = input_info(Arc::clone(&node) as Arc<dyn CustomNode>);

        let mut a_layout = LayoutNode::new(ui_layout::Style::default());
        a_layout.layout_box =
            ui_layout::LayoutBox::BlockBox(box_model(10.0, 20.0, 160.0, 100.0, 160.0, 300.0));
        let mut custom_layout = LayoutNode::new(ui_layout::Style::default());
        custom_layout.layout_box =
            ui_layout::LayoutBox::BlockBox(box_model(5.0, 0.0, 120.0, 28.0, 120.0, 28.0));
        a_layout.children = vec![LayoutChild::Node(Box::new(custom_layout))];

        let mut root_layout = LayoutNode::with_children(ui_layout::Style::default(), [a_layout]);
        root_layout.layout_box =
            ui_layout::LayoutBox::BlockBox(box_model(0.0, 0.0, 200.0, 200.0, 200.0, 200.0));

        let a_info = InfoNode {
            kind: NodeKind::Container {
                scroll_x: false,
                scroll_y: true,
                scroll_offset_x: 0.0,
                scroll_offset_y: a_scroll_y,
                style: ContainerStyle::default(),
                role: ContainerRole::Normal,
            },
            children: vec![custom_info],
            dom_id: None,
        };
        let root_info = InfoNode {
            kind: NodeKind::Container {
                scroll_x: false,
                scroll_y: false,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                style: ContainerStyle::default(),
                role: ContainerRole::Normal,
            },
            children: vec![a_info],
            dom_id: None,
        };

        (root_layout, root_info, node)
    }

    /// Builds the child→parent hit path matching `make_tree`.
    fn build_path<'a>(root_layout: &'a LayoutNode, root_info: &'a InfoNode) -> HitPath<'a> {
        let a_layout = match &root_layout.children[0] {
            LayoutChild::Node(node) => node,
            _ => unreachable!("expected container child"),
        };
        let custom_layout = match &a_layout.children[0] {
            LayoutChild::Node(node) => node,
            _ => unreachable!("expected custom child"),
        };
        let a_info = &root_info.children[0];
        let custom_info = &a_info.children[0];
        vec![
            HitItem {
                layout: custom_layout,
                info: custom_info,
            },
            HitItem {
                layout: a_layout,
                info: a_info,
            },
            HitItem {
                layout: root_layout,
                info: root_info,
            },
        ]
    }

    #[test]
    fn dispatch_pointer_uses_node_local_coords() {
        let (root_layout, root_info, node) = make_tree(0.0);
        node.popup_open.store(false, Ordering::Relaxed);
        let path = build_path(&root_layout, &root_info);

        // Content origin (15,20); no scroll: (20,25) → local (5,5).
        dispatch_pointer(&path, PointerEvent::Down { x: 20.0, y: 25.0 });
        assert_eq!(node.events(), vec![PointerEvent::Down { x: 5.0, y: 5.0 }]);
    }

    #[test]
    fn dispatch_pointer_folds_ancestor_scroll_into_local_coords() {
        let (root_layout, root_info, node) = make_tree(50.0);
        node.popup_open.store(false, Ordering::Relaxed);
        let path = build_path(&root_layout, &root_info);

        // Same click with a 50px ancestor scroll: local y += 50.
        dispatch_pointer(&path, PointerEvent::Move { x: 20.0, y: 25.0 });
        assert_eq!(node.events(), vec![PointerEvent::Move { x: 5.0, y: 55.0 }]);
    }

    #[test]
    fn popup_events_use_popup_local_coords() {
        let (root_layout, root_info, node) = make_tree(0.0);
        let path = build_path(&root_layout, &root_info);

        // Global (20,60) → local (5,40) → inside popup (y in 28..112) →
        // popup-local (5, 40-28=12).
        dispatch_pointer(&path, PointerEvent::Down { x: 20.0, y: 60.0 });
        assert_eq!(
            node.popup_events(),
            vec![PointerEvent::Down { x: 5.0, y: 12.0 }]
        );
        assert!(node.events().is_empty());
        assert!(!node.dismissed.load(Ordering::Relaxed));
    }

    #[test]
    fn down_outside_popup_routes_to_node() {
        let (root_layout, root_info, node) = make_tree(0.0);
        let path = build_path(&root_layout, &root_info);

        // Global (20,30) → local (5,10), above the popup: the node receives
        // the press (dismissal is handled globally by `dismiss_open_popups`).
        dispatch_pointer(&path, PointerEvent::Down { x: 20.0, y: 30.0 });
        assert!(!node.dismissed.load(Ordering::Relaxed));
        assert_eq!(node.events(), vec![PointerEvent::Down { x: 5.0, y: 10.0 }]);
        assert!(node.popup_events().is_empty());
    }

    #[test]
    fn move_outside_popup_routes_to_node() {
        let (root_layout, root_info, node) = make_tree(0.0);
        let path = build_path(&root_layout, &root_info);

        // Global (20,30) → local (5,10), above the popup: the node receives
        // the move and the popup stays open.
        dispatch_pointer(&path, PointerEvent::Move { x: 20.0, y: 30.0 });
        assert!(!node.dismissed.load(Ordering::Relaxed));
        assert_eq!(node.events(), vec![PointerEvent::Move { x: 5.0, y: 10.0 }]);
        assert!(node.popup_events().is_empty());
    }

    fn popup_hit_assert(
        root_layout: &LayoutNode,
        root_info: &InfoNode,
        node: &Arc<RecordingNode>,
        x: f32,
        y: f32,
    ) {
        let path = hit_test(root_layout, root_info, x, y);
        let hit = hit_custom_node(&path).unwrap();
        let expected: Arc<dyn CustomNode> = node.clone();
        assert!(Arc::ptr_eq(hit, &expected));
    }

    #[test]
    fn hit_test_finds_open_popup() {
        let (root_layout, root_info, node) = make_tree(0.0);
        // Custom page origin (15,20); popup page rect (15,48)-(135,132).
        // Global (20,60) → local (5,40), inside the popup (y in 28..112).
        popup_hit_assert(&root_layout, &root_info, &node, 20.0, 60.0);
    }

    #[test]
    fn hit_test_ignores_closed_popup() {
        let (root_layout, root_info, node) = make_tree(0.0);
        node.popup_open.store(false, Ordering::Relaxed);
        // Local (5,40) is below the custom box (y in 0..28); with the popup
        // closed the click lands on the container instead.
        let path = hit_test(&root_layout, &root_info, 20.0, 60.0);
        assert_eq!(path.len(), 2);
        assert!(hit_custom_node(&path).is_none());
    }

    #[test]
    fn hit_test_finds_popup_escaping_ancestor_box() {
        let (root_layout, root_info, node) = make_tree(0.0);
        // The container box ends at y=120 but the popup reaches y=132. A click
        // past the container is still a popup hit because popups render above
        // ancestor boxes and clips.
        popup_hit_assert(&root_layout, &root_info, &node, 20.0, 125.0);
    }

    #[test]
    fn hit_test_folds_scroll_into_popup_hit() {
        let (root_layout, root_info, node) = make_tree(50.0);
        // With the 50px ancestor scroll, a click at page y=25 maps to local
        // (5,55), inside the popup (y in 28..112).
        popup_hit_assert(&root_layout, &root_info, &node, 20.0, 25.0);
    }

    #[test]
    fn dismiss_open_popups_closes_popup_on_outside_press() {
        let (root_layout, root_info, node) = make_tree(0.0);
        // (10,25) lands on the container, outside the custom box and popup.
        let path = hit_test(&root_layout, &root_info, 10.0, 25.0);
        assert!(dismiss_open_popups(&root_info, &path));
        assert!(node.dismissed.load(Ordering::Relaxed));
    }

    #[test]
    fn dismiss_open_popups_keeps_popup_under_press() {
        let (root_layout, root_info, node) = make_tree(0.0);
        // Press on the open popup itself.
        let path = hit_test(&root_layout, &root_info, 20.0, 60.0);
        assert!(!dismiss_open_popups(&root_info, &path));
        assert!(!node.dismissed.load(Ordering::Relaxed));
    }

    #[test]
    fn dismiss_open_popups_keeps_popup_when_press_on_owner_box() {
        let (root_layout, root_info, node) = make_tree(0.0);
        // Press on the owning box (not the popup): the owner closes it itself,
        // so no separate dismissal happens.
        let path = hit_test(&root_layout, &root_info, 20.0, 30.0);
        assert!(!dismiss_open_popups(&root_info, &path));
        assert!(!node.dismissed.load(Ordering::Relaxed));
    }

    #[test]
    fn dismiss_open_popups_ignores_closed_popup() {
        let (root_layout, root_info, node) = make_tree(0.0);
        node.popup_open.store(false, Ordering::Relaxed);
        let path = hit_test(&root_layout, &root_info, 10.0, 25.0);
        assert!(!dismiss_open_popups(&root_info, &path));
        assert!(!node.dismissed.load(Ordering::Relaxed));
    }

    /// Tree: block `b` (0,0,300,100) → inline `i` (line box 10,10,200,20) →
    /// block child `c` (30,10,50,20). Inline content shares the block's
    /// content space, so `c` sits at absolute (30,10) inside `b`.
    fn make_inline_tree() -> (LayoutNode, InfoNode) {
        let mut c = LayoutNode::new(ui_layout::Style::default());
        c.layout_box =
            ui_layout::LayoutBox::BlockBox(box_model(30.0, 10.0, 50.0, 20.0, 50.0, 20.0));
        let mut b =
            LayoutNode::with_children(ui_layout::Style::default(), [inline_node(vec![c.into()])]);
        b.layout_box =
            ui_layout::LayoutBox::BlockBox(box_model(0.0, 0.0, 300.0, 100.0, 300.0, 100.0));

        let c_info = container_info(false, Some(7));
        let mut i_info = container_info(false, None);
        i_info.children.push(c_info);
        let mut b_info = container_info(false, None);
        b_info.children.push(i_info);

        (b, b_info)
    }

    #[test]
    fn hit_test_inside_inline_keeps_parent_coords() {
        let (b, b_info) = make_inline_tree();
        // Inline boxes push no transform, so the child's coordinates are
        // resolved in the block's content space: (40,15) falls inside `c` at
        // (30,10,50,20) even though `i`'s content origin is (10,10).
        let path = hit_test(&b, &b_info, 40.0, 15.0);
        assert_eq!(hit_dom_id(&path), Some(7));
        assert_eq!(path.len(), 3);
    }

    #[test]
    fn hit_test_inside_inline_but_outside_child_hits_inline() {
        let (b, b_info) = make_inline_tree();
        // (12,12) is inside `i`'s line box but left of `c`; the inline
        // container itself is hit instead.
        let path = hit_test(&b, &b_info, 12.0, 12.0);
        assert_eq!(path.len(), 2);
        assert_eq!(hit_dom_id(&path), None);
    }

    #[test]
    fn hit_test_outside_inline_line_boxes_falls_back_to_parent() {
        let (b, b_info) = make_inline_tree();
        // (40,50) is inside `b` but below `i`'s line box (10..30).
        let path = hit_test(&b, &b_info, 40.0, 50.0);
        assert_eq!(path.len(), 1);
        assert_eq!(hit_dom_id(&path), None);
    }

    #[test]
    fn dispatch_pointer_to_inline_custom_keeps_parent_content_coords() {
        let node: Arc<RecordingNode> = Arc::new(RecordingNode::new(Rect {
            x: 0.0,
            y: 28.0,
            width: 120.0,
            height: 84.0,
        }));
        let custom_info = input_info(Arc::clone(&node) as Arc<dyn CustomNode>);

        let mut inline_layout = LayoutNode::new(ui_layout::Style::default());
        inline_layout.layout_box = ui_layout::LayoutBox::InlineBox(ui_layout::InlineBox {
            box_model: box_model(10.0, 20.0, 160.0, 100.0, 160.0, 100.0),
            line_spans: vec![ui_layout::LineSpan {
                x_range: 0.0..160.0,
                line_pos: (10.0, 20.0),
                line_index: 0,
            }],
        });
        let mut block_layout = LayoutNode::new(ui_layout::Style::default());
        block_layout.layout_box =
            ui_layout::LayoutBox::BlockBox(box_model(0.0, 0.0, 200.0, 200.0, 200.0, 200.0));

        let inline_info = container_info(false, None);
        let root_info = container_info(false, None);

        node.popup_open.store(false, Ordering::Relaxed);
        let path = vec![
            HitItem {
                layout: &inline_layout,
                info: &custom_info,
            },
            HitItem {
                layout: &inline_layout,
                info: &inline_info,
            },
            HitItem {
                layout: &block_layout,
                info: &root_info,
            },
        ];

        // Inline boxes contribute no content-origin offset, so the event is
        // delivered in the block's content space, not shifted by `i`'s (10,20)
        // content origin.
        dispatch_pointer(&path, PointerEvent::Down { x: 30.0, y: 35.0 });
        assert_eq!(node.events(), vec![PointerEvent::Down { x: 30.0, y: 35.0 }]);
    }

    #[test]
    fn scroll_at_inside_inline_scrolls_child_in_parent_coords() {
        let mut c = LayoutNode::new(ui_layout::Style::default());
        c.layout_box =
            ui_layout::LayoutBox::BlockBox(box_model(30.0, 10.0, 50.0, 80.0, 50.0, 240.0));
        let mut b =
            LayoutNode::with_children(ui_layout::Style::default(), [inline_node(vec![c.into()])]);
        b.layout_box =
            ui_layout::LayoutBox::BlockBox(box_model(0.0, 0.0, 200.0, 100.0, 200.0, 300.0));

        let c_info = container_info(true, None);
        let mut i_info = container_info(false, None);
        i_info.children.push(c_info);
        let mut b_info = scrollable_info();
        b_info.children.push(i_info);

        assert!(
            scroll_at(
                &b,
                &mut b_info,
                (VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
                40.0,
                15.0,
                0.0,
                10.0
            )
            .is_some()
        );
        // The child under the cursor scrolls; the block parent does not.
        assert_eq!(scroll_offset_y_of(&b_info.children[0].children[0]), 10.0);
        assert_eq!(scroll_offset_y_of(&b_info), 0.0);
    }
}
