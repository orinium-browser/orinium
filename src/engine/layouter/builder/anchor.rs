//! CSS Anchor Positioning resolution.

use std::collections::HashMap;

use crate::engine::css::values::CssValue;
use crate::engine::layouter::builder::layout_fix::shift_layout_box;
use crate::engine::layouter::types::{AnchorSide, AnchorSpec, ContainerStyle, InfoNode, NodeKind};

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

    // Take the first comma-separated argument list (top-level arguments)
    let mut groups = args_groups;
    if groups.is_empty() {
        return None;
    }
    let first_group = groups.remove(0);

    let mut element: Option<String> = None;
    let mut side: Option<AnchorSide> = None;
    let mut fallback: Option<ui_layout::LengthOrAuto> = None;

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
                        // Could be a dashed ident
                        if lower.starts_with("--") {
                            element = Some(k.to_string());
                        }
                    }
                }
            }
            CssValue::Length(val, unit) => {
                // try to map unit to LengthOrAuto via standard means? but we only need basic
                // fallback is stored as ui_layout::LengthOrAuto; import via type alias not directly
                // but we can construct via known variants
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

fn box_border_rect(layout_box: &LayoutBox) -> Option<Rect> {
    match layout_box {
        LayoutBox::BlockBox(m) => Some(m.border_box),
        LayoutBox::InlineBox(i) => Some(i.box_model.border_box),
        LayoutBox::None => None,
    }
}

fn custom_border_rect(child: &ui_layout::CustomChild) -> Option<Rect> {
    child.result().map(|r| r.box_model.border_box)
}

fn resolve_anchor_rect<'a>(
    name: &str,
    anchors: &'a HashMap<String, Rect>,
    _container_style: &ContainerStyle,
    _position_anchor: Option<&str>,
) -> Option<&'a Rect> {
    anchors.get(name)
}

fn anchor_side_coord(rect: &Rect, side: AnchorSide, axis: Axis) -> f32 {
    match axis {
        Axis::Vertical => match side {
            AnchorSide::Top | AnchorSide::Start => rect.y,
            AnchorSide::Bottom | AnchorSide::End => rect.bottom(),
            AnchorSide::Center => rect.y + rect.height * 0.5,
            AnchorSide::Percent(p) => rect.y + rect.height * (p / 100.0),
            _ => rect.y,
        },
        Axis::Horizontal => match side {
            AnchorSide::Left | AnchorSide::Start => rect.x,
            AnchorSide::Right | AnchorSide::End => rect.right(),
            AnchorSide::Center => rect.x + rect.width * 0.5,
            AnchorSide::Percent(p) => rect.x + rect.width * (p / 100.0),
            _ => rect.x,
        },
    }
}

#[derive(Clone, Copy)]
enum Axis {
    Vertical,
    Horizontal,
}

struct Collector<'a> {
    anchors: HashMap<String, Rect>,
    parent_origin: Point,
    _info: &'a InfoNode,
}

fn collect_anchors(
    layout: &LayoutNode,
    info: &InfoNode,
    parent_origin: Point,
    anchors: &mut HashMap<String, Rect>,
) {
    let border = box_border_rect(&layout.layout_box);
    let origin = if let Some(b) = border {
        Point {
            x: parent_origin.x + b.x,
            y: parent_origin.y + b.y,
        }
    } else {
        parent_origin
    };

    // Register anchor names from this node's container style
    if let NodeKind::Container { style, .. } = &info.kind {
        for name in &style.anchor.anchors {
            let key = name.clone();
            if let Some(b) = border {
                let page_rect = Rect {
                    x: parent_origin.x + b.x,
                    y: parent_origin.y + b.y,
                    width: b.width,
                    height: b.height,
                };
                anchors.insert(key, page_rect);
            }
        }
    } else if let NodeKind::Custom { style, .. } = &info.kind {
        for name in &style.anchor.anchors {
            let key = name.clone();
            if let Some(b) = border {
                let page_rect = Rect {
                    x: parent_origin.x + b.x,
                    y: parent_origin.y + b.y,
                    width: b.width,
                    height: b.height,
                };
                anchors.insert(key, page_rect);
            }
        }
    } else if let NodeKind::Svg { style, .. } = &info.kind {
        for name in &style.anchor.anchors {
            let key = name.clone();
            if let Some(b) = border {
                let page_rect = Rect {
                    x: parent_origin.x + b.x,
                    y: parent_origin.y + b.y,
                    width: b.width,
                    height: b.height,
                };
                anchors.insert(key, page_rect);
            }
        }
    }

    for (child_idx, child) in layout.children.iter().enumerate() {
        match child {
            LayoutChild::Node(n) => {
                if let Some(info_child) = info.children.get(child_idx) {
                    collect_anchors(n, info_child, origin, anchors);
                }
            }
            LayoutChild::Custom(c) => {
                if let Some(info_child) = info.children.get(child_idx) {
                    // custom node: register if has anchors
                    if let NodeKind::Custom { style, .. } = &info_child.kind {
                        for name in &style.anchor.anchors {
                            if let Some(b) = custom_border_rect(c) {
                                let page_rect = Rect {
                                    x: parent_origin.x + b.x, // custom uses its own box; but better use origin? maybe parent content + b.x? approximated
                                    y: parent_origin.y + b.y,
                                    width: b.width,
                                    height: b.height,
                                };
                                anchors.insert(name.clone(), page_rect);
                            }
                        }
                    }
                    // no deeper children in info for custom? usually leaf
                }
            }
            LayoutChild::Fragment(_) => {}
        }
    }
}

fn resolve_node(
    layout: &mut LayoutNode,
    info: &InfoNode,
    parent_origin: Point,
    anchors: &HashMap<String, Rect>,
) {
    let border = box_border_rect(&layout.layout_box);
    let origin = if let Some(b) = border {
        Point {
            x: parent_origin.x + b.x,
            y: parent_origin.y + b.y,
        }
    } else {
        parent_origin
    };

    // Resolve insets for this node if it has anchor specs
    let mut dx: f32 = 0.0;
    let mut dy: f32 = 0.0;

    let container_style_ref = match &info.kind {
        NodeKind::Container { style, .. } => Some(style.clone()),
        NodeKind::Custom { style, .. } => Some(style.clone()),
        NodeKind::Svg { style, .. } => Some(style.clone()),
        _ => None,
    };

    if let Some(cs) = &container_style_ref {
        // top
        if let Some(spec) = &cs.anchor.top {
            if let Some(rect) = find_anchor_rect(spec, cs, anchors) {
                let target_y = anchor_side_coord(rect, spec.side, Axis::Vertical);
                if let Some(b) = border {
                    dy += target_y - (parent_origin.y + b.y);
                }
            }
        }
        if let Some(spec) = &cs.anchor.bottom {
            if let Some(rect) = find_anchor_rect(spec, cs, anchors) {
                let target_y = anchor_side_coord(rect, spec.side, Axis::Vertical);
                if let Some(b) = border {
                    dy += target_y - (parent_origin.y + b.bottom());
                }
            }
        }
        if let Some(spec) = &cs.anchor.left {
            if let Some(rect) = find_anchor_rect(spec, cs, anchors) {
                let target_x = anchor_side_coord(rect, spec.side, Axis::Horizontal);
                if let Some(b) = border {
                    dx += target_x - (parent_origin.x + b.x);
                }
            }
        }
        if let Some(spec) = &cs.anchor.right {
            if let Some(rect) = find_anchor_rect(spec, cs, anchors) {
                let target_x = anchor_side_coord(rect, spec.side, Axis::Horizontal);
                if let Some(b) = border {
                    dx += target_x - (parent_origin.x + b.right());
                }
            }
        }
    }

    if dx.abs() > 0.001 || dy.abs() > 0.001 {
        shift_layout_box(&mut layout.layout_box, dx, dy);
    }

    for (child_idx, child) in layout.children.iter_mut().enumerate() {
        match child {
            LayoutChild::Node(n) => {
                if let Some(info_child) = info.children.get(child_idx) {
                    resolve_node(n, info_child, origin, anchors);
                }
            }
            LayoutChild::Custom(_c) => {
                // custom nodes positioned by own box; shifting parent origin logic already applied to their stored rects? easier to skip deep resolve for custom content
            }
            LayoutChild::Fragment(_) => {}
        }
    }
}

fn find_anchor_rect<'a>(
    spec: &AnchorSpec,
    container_style: &ContainerStyle,
    anchors: &'a HashMap<String, Rect>,
) -> Option<&'a Rect> {
    // Try explicit element name first
    if let Some(ref name) = spec.element {
        if let Some(r) = anchors.get(name) {
            return Some(r);
        }
    }
    // Try position-anchor
    if let Some(ref name) = container_style.anchor.position_anchor {
        if let Some(r) = anchors.get(name) {
            return Some(r);
        }
    }
    None
}

/// Resolve CSS anchor positioning for all positioned elements.
/// Walks the tree after layout to collect anchor rects (by anchor-name),
/// then shifts elements that use `anchor()` inset functions.
pub fn resolve_anchor_positioning(layout: &mut LayoutNode, info: &InfoNode) {
    let mut anchors: HashMap<String, Rect> = HashMap::new();
    let origin = Point { x: 0.0, y: 0.0 };
    collect_anchors(layout, info, origin, &mut anchors);
    resolve_node(layout, info, origin, &anchors);
}
