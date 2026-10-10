//! CSS Anchor Positioning resolution.

use std::collections::HashMap;

use crate::engine::css::values::CssValue;
use crate::engine::layouter::builder::layout_fix::shift_layout_box;
use crate::engine::layouter::types::{
    AnchorSide, AnchorSpec, ContainerStyle, Direction, InfoNode, NodeKind, WritingMode,
};

use ui_layout::{LayoutBox, LayoutChild, LayoutNode, LengthOrAuto, Rect};

/// Parse a single `<anchor()>` function value into `AnchorSpec`.
pub fn parse_anchor_spec(value: &CssValue) -> Option<AnchorSpec> {
    let (name, args_groups) = match value {
        CssValue::Function(name, args) if name.eq_ignore_ascii_case("anchor") => {
            (name.clone(), args.clone())
        }
        _ => return None,
    };

    let _ = name;

    // Take the first comma-separated argument list (top-level arguments).
    let mut groups = args_groups;
    if groups.is_empty() {
        return None;
    }
    let first_group = groups.remove(0);

    let mut element: Option<String> = None;
    let mut side: Option<AnchorSide> = None;
    let mut fallback: Option<LengthOrAuto> = None;

    for v in first_group {
        match v {
            CssValue::Keyword(ref k) => {
                let lower = k.to_ascii_lowercase();
                match lower.as_str() {
                    "top" => side = Some(AnchorSide::Top),
                    "right" => side = Some(AnchorSide::Right),
                    "bottom" => side = Some(AnchorSide::Bottom),
                    "left" => side = Some(AnchorSide::Left),
                    "center" => side = Some(AnchorSide::Center),
                    "start" => side = Some(AnchorSide::Start),
                    "end" => side = Some(AnchorSide::End),
                    "self-start" => side = Some(AnchorSide::SelfStart),
                    "self-end" => side = Some(AnchorSide::SelfEnd),
                    _ => {
                        // Could be a dashed ident.
                        if lower.starts_with("--") {
                            element = Some(k.to_string());
                        }
                    }
                }
            }
            CssValue::Length(val, unit) => {
                let len = match unit {
                    crate::engine::css::values::Unit::Px => ui_layout::Length::Px(val),
                    crate::engine::css::values::Unit::Percent => ui_layout::Length::Percent(val),
                    _ => ui_layout::Length::Px(val),
                };
                fallback = Some(LengthOrAuto::Length(len));
            }
            CssValue::Number(0.0) => {
                fallback = Some(LengthOrAuto::Length(ui_layout::Length::Px(0.0)));
            }
            _ => {}
        }
    }

    Some(AnchorSpec {
        element,
        side: side.unwrap_or(AnchorSide::Top),
        fallback,
    })
}

#[derive(Clone, Copy, Debug)]
struct Point {
    x: f32,
    y: f32,
}

/// Which axis an inset property resolves along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Vertical,
    Horizontal,
}

/// A collected anchor: its page-space border box, the writing mode and
/// direction of the anchor element, and the innermost `anchor-scope` name that
/// was active where the anchor was declared (`None` means globally scoped).
#[derive(Clone)]
struct AnchorEntry {
    rect: Rect,
    writing_mode: WritingMode,
    direction: Direction,
    scope: Option<String>,
}

fn box_border_rect(layout_box: &LayoutBox) -> Option<Rect> {
    match layout_box {
        LayoutBox::BlockBox(m) => Some(m.border_box),
        LayoutBox::InlineBox(i) => Some(i.box_model.border_box),
        LayoutBox::None => None,
    }
}

/// Offsets of a layout box relative to its parent's content box: the
/// `(border, content)` origins. Children are laid out relative to the parent's
/// content box, so a node's own border origin uses `border_box`, while the
/// origin handed to its children uses `content_box`.
fn box_offsets(layout_box: &LayoutBox) -> (Point, Point) {
    match layout_box {
        LayoutBox::BlockBox(m) => (
            Point {
                x: m.border_box.x,
                y: m.border_box.y,
            },
            Point {
                x: m.content_box.x,
                y: m.content_box.y,
            },
        ),
        LayoutBox::InlineBox(i) => (
            Point {
                x: i.box_model.border_box.x,
                y: i.box_model.border_box.y,
            },
            Point {
                x: i.box_model.content_box.x,
                y: i.box_model.content_box.y,
            },
        ),
        LayoutBox::None => (Point { x: 0.0, y: 0.0 }, Point { x: 0.0, y: 0.0 }),
    }
}

fn custom_border_rect(child: &ui_layout::CustomChild) -> Option<Rect> {
    child.result().map(|r| r.box_model.border_box)
}

/// Map an `AnchorSide` to a coordinate along `axis`, honoring logical sides.
fn anchor_side_coord(
    rect: &Rect,
    side: AnchorSide,
    axis: Axis,
    writing_mode: WritingMode,
    direction: Direction,
) -> f32 {
    let low = match axis {
        Axis::Vertical => rect.y,
        Axis::Horizontal => rect.x,
    };
    let high = match axis {
        Axis::Vertical => rect.bottom(),
        Axis::Horizontal => rect.right(),
    };
    let mid = low + (high - low) * 0.5;
    match side {
        AnchorSide::Top | AnchorSide::Left => low,
        AnchorSide::Bottom | AnchorSide::Right => high,
        AnchorSide::Center => mid,
        AnchorSide::Percent(p) => low + (high - low) * (p / 100.0),
        AnchorSide::Start | AnchorSide::SelfStart => {
            if logical_start_is_low(writing_mode, direction, axis) {
                low
            } else {
                high
            }
        }
        AnchorSide::End | AnchorSide::SelfEnd => {
            if logical_start_is_low(writing_mode, direction, axis) {
                high
            } else {
                low
            }
        }
    }
}

/// Whether the logical-start edge is the low-coordinate edge along `axis` for
/// the given writing mode and direction.
fn logical_start_is_low(writing_mode: WritingMode, direction: Direction, axis: Axis) -> bool {
    match writing_mode {
        WritingMode::HorizontalTb => match axis {
            // block-start = top
            Axis::Vertical => true,
            // inline-start = left (ltr) / right (rtl)
            Axis::Horizontal => direction == Direction::Ltr,
        },
        WritingMode::VerticalRl => match axis {
            // block flow runs right-to-left
            Axis::Horizontal => false,
            // inline flow runs top-to-bottom
            Axis::Vertical => true,
        },
        WritingMode::VerticalLr => match axis {
            // block flow runs left-to-right
            Axis::Horizontal => true,
            // inline flow runs top-to-bottom
            Axis::Vertical => true,
        },
    }
}

/// Extract the anchor-relevant style plus writing mode/direction for a node.
fn style_of(info: &InfoNode) -> Option<&ContainerStyle> {
    match &info.kind {
        NodeKind::Container { style, .. } => Some(style),
        NodeKind::Custom { style, .. } => Some(style),
        NodeKind::Svg { style, .. } => Some(style),
        _ => None,
    }
}

/// Push the scope active for this node's subtree. Returns `true` if pushed.
fn push_scope(style: Option<&ContainerStyle>, scopes: &mut Vec<String>) -> bool {
    if let Some(cs) = style
        && let Some(name) = &cs.anchor.anchor_scope
    {
        scopes.push(name.clone());
        return true;
    }
    false
}

fn collect_anchors(
    layout: &LayoutNode,
    info: &InfoNode,
    parent_origin: Point,
    anchors: &mut HashMap<String, Vec<AnchorEntry>>,
    scopes: &mut Vec<String>,
) {
    let (border_off, content_off) = box_offsets(&layout.layout_box);
    let origin = Point {
        x: parent_origin.x + content_off.x,
        y: parent_origin.y + content_off.y,
    };

    let style = style_of(info);
    let pushed = push_scope(style, scopes);

    if let Some(cs) = style
        && let Some(b) = box_border_rect(&layout.layout_box)
    {
        let entry = AnchorEntry {
            rect: Rect {
                x: parent_origin.x + border_off.x,
                y: parent_origin.y + border_off.y,
                width: b.width,
                height: b.height,
            },
            writing_mode: cs.writing_mode,
            direction: cs.direction,
            scope: scopes.last().cloned(),
        };
        for name in &cs.anchor.anchors {
            anchors.entry(name.clone()).or_default().push(entry.clone());
        }
    }

    for (child_idx, child) in layout.children.iter().enumerate() {
        match child {
            LayoutChild::Node(n) => {
                if let Some(info_child) = info.children.get(child_idx) {
                    collect_anchors(n, info_child, origin, anchors, scopes);
                }
            }
            LayoutChild::Custom(c) => {
                if let Some(info_child) = info.children.get(child_idx)
                    && let Some(cs) = style_of(info_child)
                    && let Some(b) = custom_border_rect(c)
                {
                    let entry = AnchorEntry {
                        rect: Rect {
                            x: origin.x + b.x,
                            y: origin.y + b.y,
                            width: b.width,
                            height: b.height,
                        },
                        writing_mode: cs.writing_mode,
                        direction: cs.direction,
                        scope: scopes.last().cloned(),
                    };
                    for name in &cs.anchor.anchors {
                        anchors.entry(name.clone()).or_default().push(entry.clone());
                    }
                }
            }
            LayoutChild::Fragment(_) => {}
        }
    }

    if pushed {
        scopes.pop();
    }
}

/// Whether an anchor scoped to `scope` is visible from an element whose active
/// scope chain is `active`.
fn scope_visible(scope: Option<&str>, active: &[String]) -> bool {
    match scope {
        None => true,
        Some(name) => active.iter().any(|a| a == name),
    }
}

/// Whether `name` is among the element's active scopes.
fn scope_rank(name: &str, active: &[String]) -> Option<usize> {
    active.iter().rposition(|a| a == name)
}

fn find_anchor_entry<'a>(
    spec: &AnchorSpec,
    container_style: &ContainerStyle,
    anchors: &'a HashMap<String, Vec<AnchorEntry>>,
    active: &[String],
) -> Option<&'a AnchorEntry> {
    let name = spec
        .element
        .as_ref()
        .or(container_style.anchor.position_anchor.as_ref())?;
    let candidates = anchors.get(name)?;
    // Prefer the candidate scoped to the innermost active scope; ties are
    // broken by document order (the last matching candidate).
    let mut best: Option<&AnchorEntry> = None;
    let mut best_rank: isize = -1;
    for entry in candidates {
        if !scope_visible(entry.scope.as_deref(), active) {
            continue;
        }
        let rank = match entry.scope.as_deref() {
            None => 0,
            Some(n) => scope_rank(n, active).map(|p| p as isize + 1).unwrap_or(0),
        };
        if rank >= best_rank {
            best_rank = rank;
            best = Some(entry);
        }
    }
    best
}

#[derive(Clone, Copy)]
enum Edge {
    Low,
    High,
}

/// Extract a pixel margin, treating `auto` and non-pixel lengths as `0`.
fn margin_px(value: &ui_layout::LengthOrAuto) -> f32 {
    match value {
        ui_layout::LengthOrAuto::Length(ui_layout::Length::Px(v)) => *v,
        _ => 0.0,
    }
}

fn resolve_node(
    layout: &mut LayoutNode,
    info: &InfoNode,
    parent_origin: Point,
    anchors: &HashMap<String, Vec<AnchorEntry>>,
    scopes: &mut Vec<String>,
) -> bool {
    let (border_off, content_off) = box_offsets(&layout.layout_box);
    let border_origin = Point {
        x: parent_origin.x + border_off.x,
        y: parent_origin.y + border_off.y,
    };
    let origin = Point {
        x: parent_origin.x + content_off.x,
        y: parent_origin.y + content_off.y,
    };
    let border = box_border_rect(&layout.layout_box);

    let container_style = style_of(info).cloned();
    let mut dx: f32 = 0.0;
    let mut dy: f32 = 0.0;

    // Scope chain for this element's own lookups (excludes its own
    // `anchor-scope`, which only affects descendants).
    let active: Vec<String> = scopes.clone();

    if let Some(cs) = &container_style {
        // The inset positions the margin box, so a border edge target is
        // offset by the corresponding margin (`auto` margins stay `0`).
        let margin_top = margin_px(&layout.style.spacing.margin_top);
        let margin_bottom = margin_px(&layout.style.spacing.margin_bottom);
        let margin_left = margin_px(&layout.style.spacing.margin_left);
        let margin_right = margin_px(&layout.style.spacing.margin_right);

        let mut apply = |spec: &AnchorSpec, axis: Axis, edge: Edge| {
            let Some(entry) = find_anchor_entry(spec, cs, anchors, &active) else {
                return;
            };
            let (wm, dir) = match spec.side {
                AnchorSide::SelfStart | AnchorSide::SelfEnd => (cs.writing_mode, cs.direction),
                _ => (entry.writing_mode, entry.direction),
            };
            let target = anchor_side_coord(&entry.rect, spec.side, axis, wm, dir);
            if let Some(b) = border {
                let current = match edge {
                    Edge::Low => match axis {
                        Axis::Vertical => border_origin.y - margin_top,
                        Axis::Horizontal => border_origin.x - margin_left,
                    },
                    Edge::High => match axis {
                        Axis::Vertical => border_origin.y + b.height + margin_bottom,
                        Axis::Horizontal => border_origin.x + b.width + margin_right,
                    },
                };
                match axis {
                    Axis::Vertical => dy += target - current,
                    Axis::Horizontal => dx += target - current,
                }
            }
        };

        // Only one edge per axis is honored: translation cannot satisfy both
        // simultaneously (that would require resizing the box).
        match (&cs.anchor.top, &cs.anchor.bottom) {
            (Some(spec), _) => apply(spec, Axis::Vertical, Edge::Low),
            (None, Some(spec)) => apply(spec, Axis::Vertical, Edge::High),
            (None, None) => {}
        }
        match (&cs.anchor.left, &cs.anchor.right) {
            (Some(spec), _) => apply(spec, Axis::Horizontal, Edge::Low),
            (None, Some(spec)) => apply(spec, Axis::Horizontal, Edge::High),
            (None, None) => {}
        }
    }

    let shifted = dx.abs() > 0.001 || dy.abs() > 0.001;
    if shifted {
        shift_layout_box(&mut layout.layout_box, dx, dy);
    }

    // Children are positioned relative to this node's content box, so a shift
    // of this node moves them too.
    let origin = Point {
        x: origin.x + dx,
        y: origin.y + dy,
    };

    let pushed = push_scope(container_style.as_ref(), scopes);

    let mut child_shifted = false;
    for (child_idx, child) in layout.children.iter_mut().enumerate() {
        if let LayoutChild::Node(n) = child
            && let Some(info_child) = info.children.get(child_idx)
        {
            child_shifted |= resolve_node(n, info_child, origin, anchors, scopes);
        }
    }

    if pushed {
        scopes.pop();
    }

    shifted || child_shifted
}

/// Resolve CSS anchor positioning for all positioned elements.
///
/// Walks the tree after layout to collect anchor rects (by `anchor-name`),
/// then shifts elements that use `anchor()` inset functions.
pub fn resolve_anchor_positioning(layout: &mut LayoutNode, info: &InfoNode) {
    let origin = Point { x: 0.0, y: 0.0 };
    // Anchors may themselves move while resolving (e.g. an anchor nested in a
    // subtree that also gets shifted), so re-collect and re-resolve until the
    // layout stabilizes. The pass count is bounded to guarantee termination.
    const MAX_PASSES: usize = 4;
    for _ in 0..MAX_PASSES {
        let mut anchors: HashMap<String, Vec<AnchorEntry>> = HashMap::new();
        let mut collect_scopes: Vec<String> = Vec::new();
        collect_anchors(layout, info, origin, &mut anchors, &mut collect_scopes);
        let mut resolve_scopes: Vec<String> = Vec::new();
        if !resolve_node(layout, info, origin, &anchors, &mut resolve_scopes) {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 40.0,
        }
    }

    #[test]
    fn logical_start_low_horizontal_ltr() {
        assert!(logical_start_is_low(
            WritingMode::HorizontalTb,
            Direction::Ltr,
            Axis::Vertical
        ));
        assert!(logical_start_is_low(
            WritingMode::HorizontalTb,
            Direction::Ltr,
            Axis::Horizontal
        ));
    }

    #[test]
    fn logical_start_low_horizontal_rtl() {
        assert!(logical_start_is_low(
            WritingMode::HorizontalTb,
            Direction::Rtl,
            Axis::Vertical
        ));
        assert!(!logical_start_is_low(
            WritingMode::HorizontalTb,
            Direction::Rtl,
            Axis::Horizontal
        ));
    }

    #[test]
    fn logical_start_low_vertical_rl() {
        assert!(!logical_start_is_low(
            WritingMode::VerticalRl,
            Direction::Ltr,
            Axis::Horizontal
        ));
        assert!(logical_start_is_low(
            WritingMode::VerticalRl,
            Direction::Ltr,
            Axis::Vertical
        ));
    }

    #[test]
    fn anchor_side_coord_logical_inline_rtl() {
        let r = rect();
        // Inline-start in RTL horizontal is the right edge.
        assert_eq!(
            anchor_side_coord(
                &r,
                AnchorSide::Start,
                Axis::Horizontal,
                WritingMode::HorizontalTb,
                Direction::Rtl
            ),
            r.right()
        );
        assert_eq!(
            anchor_side_coord(
                &r,
                AnchorSide::End,
                Axis::Horizontal,
                WritingMode::HorizontalTb,
                Direction::Rtl
            ),
            r.x
        );
    }

    #[test]
    fn anchor_side_coord_physical_and_percent() {
        let r = rect();
        assert_eq!(
            anchor_side_coord(
                &r,
                AnchorSide::Top,
                Axis::Vertical,
                WritingMode::HorizontalTb,
                Direction::Ltr
            ),
            r.y
        );
        assert_eq!(
            anchor_side_coord(
                &r,
                AnchorSide::Bottom,
                Axis::Vertical,
                WritingMode::HorizontalTb,
                Direction::Ltr
            ),
            r.bottom()
        );
        assert_eq!(
            anchor_side_coord(
                &r,
                AnchorSide::Percent(50.0),
                Axis::Vertical,
                WritingMode::HorizontalTb,
                Direction::Ltr
            ),
            r.y + r.height * 0.5
        );
    }

    use crate::engine::layouter::types::{ContainerRole, InfoNode, NodeKind};
    use ui_layout::BoxModel;

    fn model(x: f32, y: f32, w: f32, h: f32) -> BoxModel {
        let r = Rect {
            x,
            y,
            width: w,
            height: h,
        };
        BoxModel {
            sticky_edges: None,
            border_box: r,
            padding_box: r,
            content_box: r,
            children_box: r,
        }
    }

    fn container(style: ContainerStyle, children: Vec<InfoNode>) -> InfoNode {
        InfoNode {
            kind: NodeKind::Container {
                scroll_x: false,
                scroll_y: false,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                style,
                role: ContainerRole::Normal,
            },
            children,
            dom_id: None,
        }
    }

    fn block(children: Vec<LayoutChild>, m: BoxModel) -> LayoutNode {
        let mut n = LayoutNode::new(ui_layout::Style::default());
        n.children = children;
        n.layout_box = LayoutBox::BlockBox(m);
        n
    }

    fn top_spec() -> AnchorSpec {
        AnchorSpec {
            element: None,
            side: AnchorSide::Top,
            fallback: None,
        }
    }

    fn bottom_spec() -> AnchorSpec {
        AnchorSpec {
            element: None,
            side: AnchorSide::Bottom,
            fallback: None,
        }
    }

    #[test]
    fn resolve_shifts_target_top_to_anchor_top_across_padding() {
        let mut anchor_style = ContainerStyle::default();
        anchor_style.anchor.anchors = vec!["--a".to_string()];
        let mut target_style = ContainerStyle::default();
        target_style.anchor.position_anchor = Some("--a".to_string());
        target_style.anchor.top = Some(top_spec());

        let anchor_node = block(vec![], model(10.0, 20.0, 100.0, 40.0));
        let target_node = block(vec![], model(10.0, 200.0, 50.0, 30.0));
        let mut root = block(
            vec![
                LayoutChild::Node(Box::new(anchor_node)),
                LayoutChild::Node(Box::new(target_node)),
            ],
            BoxModel {
                sticky_edges: None,
                border_box: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 500.0,
                    height: 500.0,
                },
                padding_box: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 500.0,
                    height: 500.0,
                },
                // Non-zero content offset: verifies children are resolved
                // relative to the parent content box.
                content_box: Rect {
                    x: 5.0,
                    y: 7.0,
                    width: 490.0,
                    height: 486.0,
                },
                children_box: Rect {
                    x: 5.0,
                    y: 7.0,
                    width: 490.0,
                    height: 486.0,
                },
            },
        );

        let info = container(
            ContainerStyle::default(),
            vec![
                container(anchor_style, vec![]),
                container(target_style, vec![]),
            ],
        );

        resolve_anchor_positioning(&mut root, &info);

        // Anchor page origin = root content (5,7) + anchor border (10,20) = (15,27).
        let target = root.children[1].node().unwrap();
        let target_box = target.layout_box.iter().next().unwrap();
        // Target border page origin = root content (5,7) + target border y (20) = 27.
        assert!((target_box.border_box.y - 20.0).abs() < 0.01);
    }

    #[test]
    fn resolve_shifts_target_bottom_to_anchor_bottom() {
        let mut anchor_style = ContainerStyle::default();
        anchor_style.anchor.anchors = vec!["--a".to_string()];
        let mut target_style = ContainerStyle::default();
        target_style.anchor.position_anchor = Some("--a".to_string());
        target_style.anchor.bottom = Some(bottom_spec());

        let anchor_node = block(vec![], model(10.0, 20.0, 100.0, 40.0));
        let target_node = block(vec![], model(10.0, 200.0, 50.0, 30.0));
        let mut root = block(
            vec![
                LayoutChild::Node(Box::new(anchor_node)),
                LayoutChild::Node(Box::new(target_node)),
            ],
            model(0.0, 0.0, 500.0, 500.0),
        );

        let info = container(
            ContainerStyle::default(),
            vec![
                container(anchor_style, vec![]),
                container(target_style, vec![]),
            ],
        );

        resolve_anchor_positioning(&mut root, &info);

        // Anchor bottom page = 20 + 40 = 60. Target border y should become 30 so
        // its bottom edge lands on the anchor bottom.
        let target = root.children[1].node().unwrap();
        let target_box = target.layout_box.iter().next().unwrap();
        assert!((target_box.border_box.y - 30.0).abs() < 0.01);
    }

    #[test]
    fn anchor_scope_hides_anchor_from_outside_subtree() {
        let mut scoped_anchor_style = ContainerStyle::default();
        scoped_anchor_style.anchor.anchors = vec!["--a".to_string()];
        let mut scope_holder_style = ContainerStyle::default();
        scope_holder_style.anchor.anchor_scope = Some("--a".to_string());
        let mut target_style = ContainerStyle::default();
        target_style.anchor.position_anchor = Some("--a".to_string());
        target_style.anchor.top = Some(top_spec());

        // root -> [scope_holder -> anchor, target]
        let scope_holder = block(
            vec![LayoutChild::Node(Box::new(block(
                vec![],
                model(0.0, 0.0, 10.0, 10.0),
            )))],
            model(0.0, 0.0, 10.0, 10.0),
        );
        let target_node = block(vec![], model(0.0, 100.0, 10.0, 10.0));
        let mut root = block(
            vec![
                LayoutChild::Node(Box::new(scope_holder)),
                LayoutChild::Node(Box::new(target_node)),
            ],
            model(0.0, 0.0, 500.0, 500.0),
        );

        let info = container(
            ContainerStyle::default(),
            vec![
                container(
                    scope_holder_style,
                    vec![container(scoped_anchor_style, vec![])],
                ),
                container(target_style, vec![]),
            ],
        );

        resolve_anchor_positioning(&mut root, &info);

        // `--a` is scoped to `scope_holder`'s subtree, so the outside target
        // must not move.
        let target = root.children[1].node().unwrap();
        let target_box = target.layout_box.iter().next().unwrap();
        assert!((target_box.border_box.y - 100.0).abs() < 0.01);
    }

    #[test]
    fn anchor_top_applies_margin_offset() {
        let mut anchor_style = ContainerStyle::default();
        anchor_style.anchor.anchors = vec!["--a".to_string()];
        let mut target_style = ContainerStyle::default();
        target_style.anchor.position_anchor = Some("--a".to_string());
        target_style.anchor.top = Some(top_spec());

        let anchor_node = block(vec![], model(10.0, 20.0, 100.0, 40.0));
        let mut target_node = block(vec![], model(10.0, 200.0, 50.0, 30.0));
        target_node.style.spacing.margin_top =
            ui_layout::LengthOrAuto::Length(ui_layout::Length::Px(5.0));
        let mut root = block(
            vec![
                LayoutChild::Node(Box::new(anchor_node)),
                LayoutChild::Node(Box::new(target_node)),
            ],
            model(0.0, 0.0, 500.0, 500.0),
        );

        let info = container(
            ContainerStyle::default(),
            vec![
                container(anchor_style, vec![]),
                container(target_style, vec![]),
            ],
        );

        resolve_anchor_positioning(&mut root, &info);

        // Margin box top (border y - 5) must align with the anchor top (20).
        let target = root.children[1].node().unwrap();
        let target_box = target.layout_box.iter().next().unwrap();
        assert!((target_box.border_box.y - 5.0 - 20.0).abs() < 0.01);
    }

    #[test]
    fn anchor_top_wins_when_both_vertical_edges_present() {
        let mut anchor_style = ContainerStyle::default();
        anchor_style.anchor.anchors = vec!["--a".to_string()];
        let mut target_style = ContainerStyle::default();
        target_style.anchor.position_anchor = Some("--a".to_string());
        target_style.anchor.top = Some(top_spec());
        target_style.anchor.bottom = Some(bottom_spec());

        let anchor_node = block(vec![], model(10.0, 20.0, 100.0, 40.0));
        let target_node = block(vec![], model(10.0, 200.0, 50.0, 30.0));
        let mut root = block(
            vec![
                LayoutChild::Node(Box::new(anchor_node)),
                LayoutChild::Node(Box::new(target_node)),
            ],
            model(0.0, 0.0, 500.0, 500.0),
        );

        let info = container(
            ContainerStyle::default(),
            vec![
                container(anchor_style, vec![]),
                container(target_style, vec![]),
            ],
        );

        resolve_anchor_positioning(&mut root, &info);

        // Only the top edge is honored (no resizing), so the result matches the
        // top-only resolution.
        let target = root.children[1].node().unwrap();
        let target_box = target.layout_box.iter().next().unwrap();
        assert!((target_box.border_box.y - 20.0).abs() < 0.01);
    }
}
