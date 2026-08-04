//! Generation of [`DrawCommand`]s from the layout tree (box models, borders,
//! backgrounds and text).

use ui_layout::{BoxModel, LayoutChild, LayoutNode, Rect};

use crate::engine::layouter::text_layouter::TextFlowLayouter;
use crate::engine::layouter::types::{
    Background, BorderRadius, Color, ContainerStyle, CornerRadius, InfoNode, NodeKind,
    TextDecoration, TextStyle,
};
use crate::engine::renderer_model::draw_command::{Brush, DrawCommand, FillRule, Paint};
use crate::engine::renderer_model::geom::AffineTransform;
use crate::engine::renderer_model::path::{
    Path, append_quarter_ellipse, clamp_radii, rect_path, rounded_rect_path,
};
use crate::engine::ui::ContentSize;

/// Per-box-model push state for balanced pop generation.
#[derive(Default, Clone, Copy)]
struct BoxPushState {
    border: bool,
    clip: bool,
    content: bool,
    scroll: bool,
}

// --------------------------------
// Helpers
// --------------------------------

fn push_transform(cmd_buf: &mut Vec<DrawCommand>, dx: f32, dy: f32) -> bool {
    if dx != 0.0 || dy != 0.0 {
        cmd_buf.push(DrawCommand::PushTransform {
            transform: AffineTransform::translate(dx, dy),
        });
        true
    } else {
        false
    }
}

/// Resolve the four outer corner radii to pixels against the border box.
///
/// Horizontal components resolve against the box width, vertical components
/// against the box height (so `%` works per-axis per CSS).
fn resolve_outer_radii(radius: &BorderRadius, box_w: f32, box_h: f32) -> [(f32, f32); 4] {
    let resolve = |c: &CornerRadius| -> (f32, f32) {
        (
            c.x.resolve_with(Some(box_w), 0.0, 0.0)
                .unwrap_or(0.0)
                .max(0.0),
            c.y.resolve_with(Some(box_h), 0.0, 0.0)
                .unwrap_or(0.0)
                .max(0.0),
        )
    };
    [
        resolve(&radius.top_left),
        resolve(&radius.top_right),
        resolve(&radius.bottom_right),
        resolve(&radius.bottom_left),
    ]
}

/// Compute the inner (padding-box) corner radii: the outer radii reduced by
/// the two adjacent border widths, per CSS.
fn inner_radii(outer: [(f32, f32); 4], bl: f32, bt: f32, br: f32, bb: f32) -> [(f32, f32); 4] {
    [
        (outer[0].0 - bl, outer[0].1 - bt),
        (outer[1].0 - br, outer[1].1 - bt),
        (outer[2].0 - br, outer[2].1 - bb),
        (outer[3].0 - bl, outer[3].1 - bb),
    ]
    .map(|(x, y)| (x.max(0.0), y.max(0.0)))
}

/// Build the closed path of the top border edge, including the top-left and
/// top-right corner caps. Coordinates are relative to the border-box origin.
///
/// `outer`/`inner` are the four corner radii in CSS order; the inner arcs are
/// concentric with the outer arcs.
fn top_border_strip(
    w: f32,
    bl: f32,
    bt: f32,
    br: f32,
    outer: [(f32, f32); 4],
    inner: [(f32, f32); 4],
) -> Path {
    let (rtl_x, rtl_y) = outer[0];
    let (rtr_x, rtr_y) = outer[1];
    let (itl_x, itl_y) = inner[0];
    let (itr_x, itr_y) = inner[1];
    let mut path = Path::new();
    path.move_to(0.0, rtl_y);
    append_quarter_ellipse(
        &mut path,
        rtl_x,
        rtl_y,
        rtl_x,
        rtl_y,
        (0.0, rtl_y),
        (rtl_x, 0.0),
    );
    path.line_to(w - rtr_x, 0.0);
    append_quarter_ellipse(
        &mut path,
        w - rtr_x,
        rtr_y,
        rtr_x,
        rtr_y,
        (w - rtr_x, 0.0),
        (w, rtr_y),
    );
    // Draw the top edge line to the start of the inner top‑right corner.
    // Use the right border width (`br`) for the outer edge, then transition to the inner radius.
    path.line_to(w - br, rtr_y);
    // Inner top‑right corner: connect outer edge to inner edge.
    // The start point is at the outer edge (`w - br`, `rtr_y`),
    // and the end point aligns with the inner radius.
    append_quarter_ellipse(
        &mut path,
        w - rtr_x - br,
        rtr_y - bt,
        itr_x,
        itr_y,
        (w - br, rtr_y),
        (w - rtr_x - br, bt),
    );
    path.line_to(itl_x, bt);
    append_quarter_ellipse(
        &mut path,
        rtl_x - bl,
        rtl_y - bt,
        itl_x,
        itl_y,
        (itl_x, bt),
        (bl, rtl_y),
    );
    path.close();
    path
}

/// Build the closed path of the bottom border edge, including the bottom-left
/// and bottom-right corner caps.
fn bottom_border_strip(
    w: f32,
    h: f32,
    bl: f32,
    bb: f32,
    br: f32,
    outer: [(f32, f32); 4],
    inner: [(f32, f32); 4],
) -> Path {
    let (rbl_x, rbl_y) = outer[3];
    let (rbr_x, rbr_y) = outer[2];
    let (ibl_x, ibl_y) = inner[3];
    let (ibr_x, ibr_y) = inner[2];
    let mut path = Path::new();
    path.move_to(w, h - rbr_y);
    append_quarter_ellipse(
        &mut path,
        w - rbr_x,
        h - rbr_y,
        rbr_x,
        rbr_y,
        (w, h - rbr_y),
        (w - rbr_x, h),
    );
    path.line_to(rbl_x, h);
    append_quarter_ellipse(
        &mut path,
        rbl_x,
        h - rbl_y,
        rbl_x,
        rbl_y,
        (rbl_x, h),
        (0.0, h - rbl_y),
    );
    path.line_to(bl, h - rbl_y);
    append_quarter_ellipse(
        &mut path,
        rbl_x,
        h - rbl_y,
        ibl_x,
        ibl_y,
        (bl, h - rbl_y),
        (rbl_x, h - bb),
    );
    path.line_to(w - rbr_x, h - bb);
    append_quarter_ellipse(
        &mut path,
        w - rbr_x,
        h - rbr_y,
        ibr_x,
        ibr_y,
        (w - rbr_x, h - bb),
        (w - br, h - rbr_y),
    );
    path.close();
    path
}

/// Draw the four border edges inside the current coordinate system.
/// Coordinates are relative to the border-box origin.
fn draw_border(
    cmd_buf: &mut Vec<DrawCommand>,
    border_box: &ui_layout::Rect,
    padding_box: &ui_layout::Rect,
    style: &ContainerStyle,
) {
    let bw_top = (padding_box.y - border_box.y).max(0.0);
    let bw_bottom =
        (border_box.y + border_box.height - (padding_box.y + padding_box.height)).max(0.0);
    let bw_left = (padding_box.x - border_box.x).max(0.0);
    let bw_right = (border_box.x + border_box.width - (padding_box.x + padding_box.width)).max(0.0);

    let w = border_box.width;
    let h = border_box.height;
    let mut outer = resolve_outer_radii(&style.border_radius, w, h);
    outer = clamp_radii(outer, w, h);
    let mut inner = inner_radii(outer, bw_left, bw_top, bw_right, bw_bottom);
    inner = clamp_radii(
        inner,
        padding_box.width.max(0.0),
        padding_box.height.max(0.0),
    );

    let bc = &style.border_color;
    let push_fill = |cmd_buf: &mut Vec<DrawCommand>, path: Path, color: Color| {
        cmd_buf.push(DrawCommand::Fill {
            path,
            rule: FillRule::NonZero,
            paint: Paint {
                brush: Brush::Solid(color),
                opacity: 1.0,
            },
        });
    };

    let has_radius = outer.iter().any(|(rx, ry)| *rx > 0.0 || *ry > 0.0);
    if !has_radius {
        if bw_top > 0.0 {
            push_fill(cmd_buf, rect_path(0.0, 0.0, w, bw_top), bc.top);
        }
        if bw_bottom > 0.0 {
            push_fill(
                cmd_buf,
                rect_path(0.0, h - bw_bottom, w, bw_bottom),
                bc.bottom,
            );
        }
        if bw_left > 0.0 {
            push_fill(
                cmd_buf,
                rect_path(0.0, bw_top, bw_left, h - bw_top - bw_bottom),
                bc.left,
            );
        }
        if bw_right > 0.0 {
            push_fill(
                cmd_buf,
                rect_path(w - bw_right, bw_top, bw_right, h - bw_top - bw_bottom),
                bc.right,
            );
        }
        return;
    }

    if bw_top > 0.0 {
        push_fill(
            cmd_buf,
            top_border_strip(w, bw_left, bw_top, bw_right, outer, inner),
            bc.top,
        );
    }
    if bw_bottom > 0.0 {
        push_fill(
            cmd_buf,
            bottom_border_strip(w, h, bw_left, bw_bottom, bw_right, outer, inner),
            bc.bottom,
        );
    }
    if bw_left > 0.0 {
        push_fill(
            cmd_buf,
            rect_path(0.0, outer[0].1, bw_left, h - outer[0].1 - outer[3].1),
            bc.left,
        );
    }
    if bw_right > 0.0 {
        push_fill(
            cmd_buf,
            rect_path(
                w - bw_right,
                outer[1].1,
                bw_right,
                h - outer[1].1 - outer[2].1,
            ),
            bc.right,
        );
    }
}

/// Draw the background inside the padding box (rounded when a border radius
/// is present).
/// Coordinates are relative to the border-box origin.
fn draw_background(
    cmd_buf: &mut Vec<DrawCommand>,
    border_box: &ui_layout::Rect,
    padding_box: &ui_layout::Rect,
    style: &ContainerStyle,
) {
    let x = padding_box.x - border_box.x;
    let y = padding_box.y - border_box.y;
    let bw_top = (padding_box.y - border_box.y).max(0.0);
    let bw_bottom =
        (border_box.y + border_box.height - (padding_box.y + padding_box.height)).max(0.0);
    let bw_left = (padding_box.x - border_box.x).max(0.0);
    let bw_right = (border_box.x + border_box.width - (padding_box.x + padding_box.width)).max(0.0);

    let mut outer = resolve_outer_radii(&style.border_radius, border_box.width, border_box.height);
    outer = clamp_radii(outer, border_box.width, border_box.height);
    let mut inner = inner_radii(outer, bw_left, bw_top, bw_right, bw_bottom);
    inner = clamp_radii(
        inner,
        padding_box.width.max(0.0),
        padding_box.height.max(0.0),
    );
    // Build the rounded background path
    let path = rounded_rect_path(
        x,
        y,
        padding_box.width,
        padding_box.height,
        inner[0],
        inner[1],
        inner[2],
        inner[3],
    );
    match &style.background {
        Background::Color(c) if c.3 > 0 => {
            cmd_buf.push(DrawCommand::Fill {
                path: path,
                rule: FillRule::NonZero,
                paint: Paint {
                    brush: Brush::Solid(*c),
                    opacity: 1.0,
                },
            });
        }
        Background::Gradient(g) => {
            cmd_buf.push(DrawCommand::Fill {
                path: path,
                rule: FillRule::NonZero,
                paint: Paint {
                    brush: Brush::Gradient(g.clone()),
                    opacity: 1.0,
                },
            });
        }
        _ => {}
    }
}

/// Push all draw commands for a single box model, returning the pop state.
fn push_box_model(
    cmd_buf: &mut Vec<DrawCommand>,
    box_model: &ui_layout::BoxModel,
    style: &crate::engine::layouter::types::ContainerStyle,
    scroll_offset_x: f32,
    scroll_offset_y: f32,
    is_inline: bool,
) -> BoxPushState {
    let border_box = box_model.border_box;
    let padding_box = box_model.padding_box;
    let content_box = box_model.content_box;

    let dx = content_box.x - border_box.x;
    let dy = content_box.y - border_box.y;

    let state = BoxPushState {
        border: push_transform(cmd_buf, border_box.x, border_box.y),
        clip: false,
        content: false,
        scroll: false,
    };

    draw_border(cmd_buf, &border_box, &padding_box, style);

    draw_background(cmd_buf, &border_box, &padding_box, style);

    let clip = !is_inline && padding_box.width > 0.0 && padding_box.height > 0.0;
    if clip {
        cmd_buf.push(DrawCommand::PushClip {
            path: rect_path(
                padding_box.x - border_box.x,
                padding_box.y - border_box.y,
                padding_box.width,
                padding_box.height,
            ),
            rule: FillRule::NonZero,
        });
    }

    let content = push_transform(cmd_buf, dx, dy);
    let scroll = push_transform(cmd_buf, scroll_offset_x, -scroll_offset_y);

    BoxPushState {
        clip,
        content,
        scroll,
        ..state
    }
}

/// Pop commands for a single box model (reverse order of pushes).
fn pop_box_model(cmd_buf: &mut Vec<DrawCommand>, state: BoxPushState) {
    if state.scroll {
        cmd_buf.push(DrawCommand::PopTransform);
    }
    if state.content {
        cmd_buf.push(DrawCommand::PopTransform);
    }
    if state.clip {
        cmd_buf.push(DrawCommand::PopClip);
    }
    if state.border {
        cmd_buf.push(DrawCommand::PopTransform);
    }
}

/// Draw text spans for a single text node.
///
/// `content_origin` is the content-box origin of the enclosing box.
/// For block containers this is `(0, 0)` (the flow cursor already starts at
/// the content area), but for inline containers the flow layouter positions
/// text in the parent's coordinate space while `push_box_model` also pushes
/// the border-box offset, so we must subtract the content-box origin to
/// avoid double-counting.
fn draw_text(
    cmd_buf: &mut Vec<DrawCommand>,
    style: &TextStyle,
    text_id: usize,
    content_origin: (f32, f32),
) {
    if let Some(result) = TextFlowLayouter::get_result(text_id) {
        for (i, line_text) in result.line_texts.iter().enumerate() {
            let span = &result.spans[i];
            let x = span.line_pos.0 - content_origin.0;
            let y = span.line_pos.1 - content_origin.1;

            cmd_buf.push(DrawCommand::DrawText {
                x,
                y,
                text: line_text.as_str().into(),
                style: style.clone(),
            });

            let font_size = style.font_size;
            let line_thickness = (font_size * 0.08).max(1.0);
            let line_y_adj = if line_text.is_empty() {
                y
            } else {
                y + font_size
            };
            let (line_y, draw) = match style.text_decoration {
                TextDecoration::None => (0.0, false),
                TextDecoration::Underline => (line_y_adj, true),
                TextDecoration::LineThrough => (y + font_size * 0.5, true),
                TextDecoration::Overline => (y, true),
            };

            if draw {
                cmd_buf.push(DrawCommand::Fill {
                    path: rect_path(
                        x,
                        line_y,
                        span.x_range.end - span.x_range.start,
                        line_thickness,
                    ),
                    rule: FillRule::NonZero,
                    paint: Paint {
                        brush: Brush::Solid(style.text_decoration_color.unwrap_or(style.color)),
                        opacity: 1.0,
                    },
                });
            }
        }
    }
}

// --------------------------------
// Public entry point
// --------------------------------

/// LayoutNode + InfoNode → DrawCommand
pub fn generate_draw_commands(
    cmd_buf: &mut Vec<DrawCommand>,
    layout: &LayoutNode,
    info: &InfoNode,
) {
    let mut box_states: Vec<BoxPushState> = Vec::new();

    let is_inline = matches!(layout.layout_box, ui_layout::LayoutBox::InlineBox(_));

    match &info.kind {
        NodeKind::Text { .. } | NodeKind::LineBreak => unreachable!(),

        NodeKind::Container {
            scroll_offset_x,
            scroll_offset_y,
            style,
            ..
        } => {
            for box_model in &layout.layout_box {
                box_states.push(push_box_model(
                    cmd_buf,
                    &box_model,
                    style,
                    *scroll_offset_x,
                    *scroll_offset_y,
                    is_inline,
                ));
            }
        }

        NodeKind::Custom {
            scroll_offset_x,
            scroll_offset_y,
            style,
            layout_style,
            node,
            text_style,
            ..
        } => {
            let effective_style = node.background().map(|background| ContainerStyle {
                background,
                ..style.clone()
            });
            let style_ref = effective_style.as_ref().unwrap_or(style);

            for box_model in &layout.layout_box {
                box_states.push(push_box_model(
                    cmd_buf,
                    &box_model,
                    style_ref,
                    *scroll_offset_x,
                    *scroll_offset_y,
                    is_inline,
                ));
            }

            let size = layout.layout_box.iter().next().map_or_else(
                || node.intrinsic_size(),
                |box_model| ContentSize {
                    width: box_model.content_box.width,
                    height: box_model.content_box.height,
                },
            );
            node.draw_sized(cmd_buf, text_style, layout_style, size);
        }
    }

    // For inline containers the text flow layouter positions text in the
    // parent's coordinate space, but push_box_model already pushes the
    // border-box offset.  Subtract the content-box origin so text
    // coordinates become relative to the pushed coordinate system.
    let text_origin = if is_inline {
        layout
            .layout_box
            .iter()
            .next()
            .map_or((0.0, 0.0), |bm| (bm.content_box.x, bm.content_box.y))
    } else {
        (0.0, 0.0)
    };

    let mut layout_iter = layout.children.iter();

    for child_info in &info.children {
        match &child_info.kind {
            NodeKind::Text { text_id, style, .. } => {
                draw_text(cmd_buf, style, *text_id, text_origin);
                layout_iter.next();
            }
            NodeKind::LineBreak => {
                layout_iter.next();
            }
            NodeKind::Container { .. } => {
                if let Some(LayoutChild::Node(node)) = layout_iter.next() {
                    generate_draw_commands(cmd_buf, node, child_info);
                }
            }
            NodeKind::Custom {
                node,
                text_style,
                style,
                layout_style,
                ..
            } => {
                match layout_iter.next() {
                    // Block custom element: recurse into the child layout node.
                    Some(LayoutChild::Node(node_layout)) => {
                        generate_draw_commands(cmd_buf, node_layout, child_info);
                    }
                    // Inline custom element: consume the Object and draw it
                    // from the layout result stored on the tree child.
                    Some(LayoutChild::Custom(custom_child)) => {
                        if let Some(result) = custom_child.result() {
                            let effective_style =
                                node.background().map(|background| ContainerStyle {
                                    background,
                                    ..style.clone()
                                });
                            let style_ref = effective_style.as_ref().unwrap_or(style);

                            let bm = &result.box_model;
                            let rect = BoxModel {
                                border_box: Rect {
                                    x: bm.border_box.x - text_origin.0,
                                    y: bm.border_box.y - text_origin.1,
                                    width: bm.border_box.width,
                                    height: bm.border_box.height,
                                },
                                padding_box: Rect {
                                    x: bm.padding_box.x - text_origin.0,
                                    y: bm.padding_box.y - text_origin.1,
                                    width: bm.padding_box.width,
                                    height: bm.padding_box.height,
                                },
                                content_box: Rect {
                                    x: bm.content_box.x - text_origin.0,
                                    y: bm.content_box.y - text_origin.1,
                                    width: bm.content_box.width,
                                    height: bm.content_box.height,
                                },
                                children_box: Rect {
                                    x: bm.children_box.x - text_origin.0,
                                    y: bm.children_box.y - text_origin.1,
                                    width: bm.children_box.width,
                                    height: bm.children_box.height,
                                },
                            };
                            push_box_model(cmd_buf, &rect, style_ref, 0.0, 0.0, true);
                            node.draw_sized(
                                cmd_buf,
                                text_style,
                                layout_style,
                                ContentSize {
                                    width: rect.content_box.width,
                                    height: rect.content_box.height,
                                },
                            );
                            pop_box_model(
                                cmd_buf,
                                BoxPushState {
                                    border: true,
                                    clip: false,
                                    content: false,
                                    scroll: false,
                                },
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    if matches!(
        info.kind,
        NodeKind::Container { .. } | NodeKind::Custom { .. }
    ) {
        for state in box_states.iter().rev() {
            pop_box_model(cmd_buf, *state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::renderer_model::geom::AffineTransform;

    fn count_balanced(commands: &[DrawCommand]) -> bool {
        let mut transform_depth = 0usize;
        let mut clip_depth = 0usize;
        for cmd in commands {
            match cmd {
                DrawCommand::PushTransform { .. } => transform_depth += 1,
                DrawCommand::PopTransform => transform_depth -= 1,
                DrawCommand::PushClip { .. } => clip_depth += 1,
                DrawCommand::PopClip => clip_depth -= 1,
                _ => {}
            }
        }
        transform_depth == 0 && clip_depth == 0
    }

    #[test]
    fn test_push_pop_transform_balanced() {
        let mut buf = Vec::new();
        assert!(push_transform(&mut buf, 5.0, 5.0));
        assert!(matches!(buf.pop(), Some(DrawCommand::PushTransform { .. })));
        assert!(!push_transform(&mut buf, 0.0, 0.0));
    }

    #[test]
    fn test_radii_resolution_and_clamp() {
        let outer = resolve_outer_radii(&BorderRadius::default(), 100.0, 50.0);
        assert_eq!(outer, [(0.0, 0.0); 4]);
    }

    fn ui_rect(x: f32, y: f32, w: f32, h: f32) -> ui_layout::Rect {
        ui_layout::Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn test_single_box_model_is_balanced() {
        let box_model = ui_layout::BoxModel {
            border_box: ui_rect(10.0, 20.0, 120.0, 60.0),
            padding_box: ui_rect(12.0, 22.0, 116.0, 56.0),
            content_box: ui_rect(12.0, 22.0, 116.0, 56.0),
            children_box: ui_rect(12.0, 22.0, 116.0, 56.0),
        };
        let style = ContainerStyle::default();
        let mut buf = Vec::new();
        let state = push_box_model(&mut buf, &box_model, &style, 0.0, 0.0, false);
        // Scroll/content transforms are no-ops here (zero offsets); border
        // transform + clip + content are pushed while the box is open.
        assert!(buf.len() >= 2);
        // Opening pushes are not yet balanced: a clip and transforms are pending.
        assert!(!count_balanced(&buf));
        pop_box_model(&mut buf, state);
        assert!(count_balanced(&buf));
    }

    #[test]
    fn test_nested_box_models_balanced() {
        let mk_box = |x: f32, y: f32, w: f32, h: f32| ui_layout::BoxModel {
            border_box: ui_rect(x, y, w, h),
            padding_box: ui_rect(x + 2.0, y + 2.0, w - 4.0, h - 4.0),
            content_box: ui_rect(x + 2.0, y + 2.0, w - 4.0, h - 4.0),
            children_box: ui_rect(x + 2.0, y + 2.0, w - 4.0, h - 4.0),
        };
        let style = ContainerStyle::default();
        let mut buf = Vec::new();
        let outer = push_box_model(
            &mut buf,
            &mk_box(0.0, 0.0, 100.0, 100.0),
            &style,
            3.0,
            4.0,
            false,
        );
        let inner = push_box_model(
            &mut buf,
            &mk_box(10.0, 10.0, 50.0, 50.0),
            &style,
            0.0,
            0.0,
            false,
        );
        // Sanity: inner push generated commands (border + background + clip).
        assert!(!buf.is_empty());
        pop_box_model(&mut buf, inner);
        pop_box_model(&mut buf, outer);
        assert!(count_balanced(&buf));
    }

    #[test]
    fn test_affine_transform_reexport() {
        let t = AffineTransform::translate(1.0, 2.0);
        assert_eq!(t.apply(0.0, 0.0), (1.0, 2.0));
    }
}
