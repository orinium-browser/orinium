//! Generation of [`DrawCommand`]s from the layout tree (box models, borders,
//! backgrounds and text).

use ui_layout::{BoxModel, EdgeOption, LayoutChild, LayoutNode, Position, Rect};

use crate::engine::layouter::text_layouter::TextFlowLayouter;
use crate::engine::layouter::types::{
    Background, BackgroundDimension, BackgroundOffset, BackgroundPositionAxis, BackgroundRepeat,
    BackgroundSize, BorderRadius, Color, ContainerStyle, CornerRadius, InfoNode, NodeKind,
    TextDecoration, TextFlowStyle, TextStyle, Visibility,
};
use crate::engine::renderer_model::draw_command::{Brush, DrawCommand, FillRule, Paint};
use crate::engine::renderer_model::geom::AffineTransform;
use crate::engine::renderer_model::path::{
    Path, append_quarter_ellipse, clamp_radii, offset_path, rect_path, rounded_rect_path,
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

/// State of the nearest scrollport, used to resolve `position: sticky` offsets.
///
/// All coordinates are expressed in the current node's parent-content space
/// (the same space its `border_box` coordinates live in).
#[derive(Default, Clone, Copy)]
struct StickyViewport {
    /// Top-left corner of the scrollport's visible region, scroll offset already
    /// applied. The renderer's scroll transform is `translate(scroll_x,
    /// -scroll_y)`, so in the scrollport's content space this is
    /// `(padding.left - scroll_x, padding.top + scroll_y)`; the value is rebased
    /// into each descendant's content space.
    top_left: (f32, f32),
    /// Visible (padding-box) size of the nearest scrollport.
    size: (f32, f32),
}

/// Whether the node scrolls its own content and thus establishes a scrollport
/// for its subtree. The document root always does, because the UI layer scrolls
/// it directly without necessarily setting its scroll flags.
fn is_scrollport(kind: &NodeKind) -> bool {
    match kind {
        NodeKind::Container {
            scroll_x, scroll_y, ..
        }
        | NodeKind::Custom {
            scroll_x, scroll_y, ..
        } => *scroll_x || *scroll_y,
        _ => false,
    }
}

/// Compute the sticky translate for a box relative to its nearest scrollport.
///
/// `box_rect` is the sticky box's border box (its natural position, in
/// parent-content space) and `containing` is the parent content-box size — the
/// containing block the box may not leave. Per CSS-POSITION-3 §3.4 the box is
/// shifted inward just enough to keep each specified edge inside the
/// scrollport's "sticky view rectangle" (the visible region, whose top-left is
/// `viewport.top_left`), while staying within the containing block.
fn sticky_offset(
    edges: &EdgeOption,
    box_rect: &Rect,
    viewport: StickyViewport,
    containing: (f32, f32),
) -> (f32, f32) {
    let x = box_rect.x;
    let y = box_rect.y;
    let w = box_rect.width;
    let h = box_rect.height;

    let mut dy: f32 = 0.0;
    let port_top = viewport.top_left.1;
    let port_bottom = viewport.top_left.1 + viewport.size.1;
    if let Some(top) = edges.top {
        dy = dy.max((port_top + top) - y);
    }
    if let Some(bottom) = edges.bottom {
        // CSS-POSITION-3 §3.4: when the sticky view rectangle is shorter than
        // the box, the end-edge inset is ignored and the box sticks to its
        // start edge instead.
        let has_room = match edges.top {
            Some(top) => (port_bottom - bottom) - (port_top + top) >= h,
            None => true,
        };
        if has_room {
            dy = dy.min((port_bottom - bottom) - (y + h));
        }
    }
    // Containing-block constraint (parent content box). The bounds are
    // zero-clamped so an already-overflowing box is never forced to move
    // (matching Chromium/Firefox "clamp sticky offset bounds by zero").
    let dy_lo = (-y).min(0.0);
    let dy_hi = (containing.1 - y - h).max(0.0);
    dy = dy.max(dy_lo).min(dy_hi);

    let mut dx: f32 = 0.0;
    let port_left = viewport.top_left.0;
    let port_right = viewport.top_left.0 + viewport.size.0;
    if let Some(left) = edges.left {
        dx = dx.max((port_left + left) - x);
    }
    if let Some(right) = edges.right {
        let has_room = match edges.left {
            Some(left) => (port_right - right) - (port_left + left) >= w,
            None => true,
        };
        if has_room {
            dx = dx.min((port_right - right) - (x + w));
        }
    }
    let dx_lo = (-x).min(0.0);
    let dx_hi = (containing.0 - x - w).max(0.0);
    dx = dx.max(dx_lo).min(dx_hi);

    (dx, dy)
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
/// Coordinates are relative to the border-box origin when `ox`/`oy` are zero
/// (the transform case); otherwise they are added to place the border in
/// absolute space (the inline case, where no transform is pushed).
fn draw_border(
    cmd_buf: &mut Vec<DrawCommand>,
    border_box: &ui_layout::Rect,
    padding_box: &ui_layout::Rect,
    style: &ContainerStyle,
    ox: f32,
    oy: f32,
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
            push_fill(cmd_buf, rect_path(ox, oy, w, bw_top), bc.top);
        }
        if bw_bottom > 0.0 {
            push_fill(
                cmd_buf,
                rect_path(ox, oy + h - bw_bottom, w, bw_bottom),
                bc.bottom,
            );
        }
        if bw_left > 0.0 {
            push_fill(
                cmd_buf,
                rect_path(ox, oy + bw_top, bw_left, h - bw_top - bw_bottom),
                bc.left,
            );
        }
        if bw_right > 0.0 {
            push_fill(
                cmd_buf,
                rect_path(
                    ox + w - bw_right,
                    oy + bw_top,
                    bw_right,
                    h - bw_top - bw_bottom,
                ),
                bc.right,
            );
        }
        return;
    }

    if bw_top > 0.0 {
        push_fill(
            cmd_buf,
            offset_path(
                &top_border_strip(w, bw_left, bw_top, bw_right, outer, inner),
                ox,
                oy,
            ),
            bc.top,
        );
    }
    if bw_bottom > 0.0 {
        push_fill(
            cmd_buf,
            offset_path(
                &bottom_border_strip(w, h, bw_left, bw_bottom, bw_right, outer, inner),
                ox,
                oy,
            ),
            bc.bottom,
        );
    }
    if bw_left > 0.0 {
        push_fill(
            cmd_buf,
            rect_path(ox, oy + outer[0].1, bw_left, h - outer[0].1 - outer[3].1),
            bc.left,
        );
    }
    if bw_right > 0.0 {
        push_fill(
            cmd_buf,
            rect_path(
                ox + w - bw_right,
                oy + outer[1].1,
                bw_right,
                h - outer[1].1 - outer[2].1,
            ),
            bc.right,
        );
    }
}

/// Draw the background inside the padding box (rounded when a border radius
/// is present).
/// Coordinates are relative to the border-box origin when `ox`/`oy` are zero;
/// otherwise they are added to place the background in absolute space.
fn draw_background(
    cmd_buf: &mut Vec<DrawCommand>,
    border_box: &ui_layout::Rect,
    padding_box: &ui_layout::Rect,
    style: &ContainerStyle,
    ox: f32,
    oy: f32,
) {
    let x = padding_box.x - border_box.x + ox;
    let y = padding_box.y - border_box.y + oy;
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
                path,
                rule: FillRule::NonZero,
                paint: Paint {
                    brush: Brush::Solid(*c),
                    opacity: 1.0,
                },
            });
        }
        Background::Gradient(g) => {
            cmd_buf.push(DrawCommand::Fill {
                path,
                rule: FillRule::NonZero,
                paint: Paint {
                    brush: Brush::Gradient(g.clone()),
                    opacity: 1.0,
                },
            });
        }
        Background::Image { image, color, .. } => {
            if color.3 > 0 {
                cmd_buf.push(DrawCommand::Fill {
                    path: path.clone(),
                    rule: FillRule::NonZero,
                    paint: Paint {
                        brush: Brush::Solid(*color),
                        opacity: 1.0,
                    },
                });
            }
            if let Some(image) = image {
                let (image_width, image_height) = resolve_background_image_size(
                    image.width() as f32,
                    image.height() as f32,
                    padding_box.width,
                    padding_box.height,
                    style.background_size,
                );
                if image_width > 0.0 && image_height > 0.0 {
                    let image_x = resolve_background_axis(
                        padding_box.width,
                        image_width,
                        style.background_position.x,
                    );
                    let image_y = resolve_background_axis(
                        padding_box.height,
                        image_height,
                        style.background_position.y,
                    );
                    let repeat_x = matches!(
                        style.background_repeat,
                        BackgroundRepeat::Repeat | BackgroundRepeat::RepeatX
                    );
                    let repeat_y = matches!(
                        style.background_repeat,
                        BackgroundRepeat::Repeat | BackgroundRepeat::RepeatY
                    );
                    let xs = background_tile_positions(
                        image_x,
                        image_width,
                        padding_box.width,
                        repeat_x,
                    );
                    let ys = background_tile_positions(
                        image_y,
                        image_height,
                        padding_box.height,
                        repeat_y,
                    );
                    cmd_buf.push(DrawCommand::PushClip {
                        path,
                        rule: FillRule::NonZero,
                    });
                    for tile_y in ys {
                        for tile_x in &xs {
                            cmd_buf.push(DrawCommand::Fill {
                                path: rect_path(x + *tile_x, y + tile_y, image_width, image_height),
                                rule: FillRule::NonZero,
                                paint: Paint {
                                    brush: Brush::Image(image.clone()),
                                    opacity: 1.0,
                                },
                            });
                        }
                    }
                    cmd_buf.push(DrawCommand::PopClip);
                }
            }
        }
        _ => {}
    }
}

fn resolve_background_dimension(dimension: BackgroundDimension, area: f32) -> Option<f32> {
    match dimension {
        BackgroundDimension::Auto => None,
        BackgroundDimension::Length(value) => Some(value.max(0.0)),
        BackgroundDimension::Percent(value) => Some((area * value).max(0.0)),
    }
}

fn resolve_background_image_size(
    intrinsic_width: f32,
    intrinsic_height: f32,
    area_width: f32,
    area_height: f32,
    size: BackgroundSize,
) -> (f32, f32) {
    if intrinsic_width <= 0.0 || intrinsic_height <= 0.0 {
        return (0.0, 0.0);
    }
    let ratio = intrinsic_width / intrinsic_height;
    match size {
        BackgroundSize::Auto => (intrinsic_width, intrinsic_height),
        BackgroundSize::Contain | BackgroundSize::Cover => {
            let width_scale = area_width / intrinsic_width;
            let height_scale = area_height / intrinsic_height;
            let scale = if matches!(size, BackgroundSize::Contain) {
                width_scale.min(height_scale)
            } else {
                width_scale.max(height_scale)
            };
            (intrinsic_width * scale, intrinsic_height * scale)
        }
        BackgroundSize::Explicit { width, height } => {
            let width = resolve_background_dimension(width, area_width);
            let height = resolve_background_dimension(height, area_height);
            match (width, height) {
                (Some(width), Some(height)) => (width, height),
                (Some(width), None) => (width, width / ratio),
                (None, Some(height)) => (height * ratio, height),
                (None, None) => (intrinsic_width, intrinsic_height),
            }
        }
    }
}

fn resolve_background_axis(area: f32, image: f32, position: BackgroundPositionAxis) -> f32 {
    let available = area - image;
    let length_offset = |offset| match offset {
        BackgroundOffset::Zero => 0.0,
        BackgroundOffset::Length(value) => value,
        BackgroundOffset::Percent(value) => area * value,
    };
    match position {
        BackgroundPositionAxis::Start(BackgroundOffset::Percent(value)) => available * value,
        BackgroundPositionAxis::Start(offset) => length_offset(offset),
        BackgroundPositionAxis::Center(offset) => available * 0.5 + length_offset(offset),
        BackgroundPositionAxis::End(offset) => available - length_offset(offset),
    }
}

fn background_tile_positions(base: f32, tile: f32, area: f32, repeat: bool) -> Vec<f32> {
    if !repeat || tile <= 0.0 {
        return vec![base];
    }
    let mut start = base;
    while start > 0.0 {
        start -= tile;
    }
    while start + tile <= 0.0 {
        start += tile;
    }
    let mut positions = Vec::new();
    let mut position = start;
    while position < area && positions.len() < 512 {
        positions.push(position);
        position += tile;
    }
    positions
}

/// Push all draw commands for a single box model, returning the pop state.
fn push_box_model(
    cmd_buf: &mut Vec<DrawCommand>,
    box_model: &ui_layout::BoxModel,
    style: &crate::engine::layouter::types::ContainerStyle,
    scroll_offset_x: f32,
    scroll_offset_y: f32,
    is_inline: bool,
    clips_overflow: bool,
    draw_bg: bool,
) -> BoxPushState {
    let border_box = box_model.border_box;
    let padding_box = box_model.padding_box;
    let content_box = box_model.content_box;

    let dx = content_box.x - border_box.x;
    let dy = content_box.y - border_box.y;

    // Inline containers lay their text out in the parent's coordinate space,
    // and every line span yields its own box model, so no transform may be
    // pushed here: otherwise the accumulated border/content offsets of all
    // line boxes would displace the inline content.
    let border = !is_inline && push_transform(cmd_buf, border_box.x, border_box.y);

    // When no transform is pushed (inline), draw commands must use absolute
    // coordinates; otherwise they are already relative to the border-box
    // origin thanks to the transform above.
    let (ox, oy) = if is_inline {
        (border_box.x, border_box.y)
    } else {
        (0.0, 0.0)
    };

    draw_border(cmd_buf, &border_box, &padding_box, style, ox, oy);

    if draw_bg {
        draw_background(cmd_buf, &border_box, &padding_box, style, ox, oy);
    }

    let clip = !is_inline && clips_overflow && padding_box.width > 0.0 && padding_box.height > 0.0;
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

    let content = !is_inline && push_transform(cmd_buf, dx, dy);
    let scroll = !is_inline && push_transform(cmd_buf, -scroll_offset_x, -scroll_offset_y);

    BoxPushState {
        border,
        clip,
        content,
        scroll,
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
fn draw_text(
    cmd_buf: &mut Vec<DrawCommand>,
    style: &TextStyle,
    flow_style: TextFlowStyle,
    text_id: usize,
) {
    if let Some(result) = TextFlowLayouter::get_result(text_id) {
        for (i, line_text) in result.line_texts.iter().enumerate() {
            let span = &result.spans[i];
            let x = span.line_pos.0;
            let y = span.line_pos.1;

            cmd_buf.push(DrawCommand::DrawText {
                x,
                y,
                text: line_text.as_str().into(),
                style: style.clone(),
                flow_style,
            });

            let font_size = flow_style.font_size;
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
///
/// `viewport` is the visible (window) size of the page area; it establishes the
/// root scrollport that page-level `position: sticky` boxes stick to.
pub fn generate_draw_commands(
    cmd_buf: &mut Vec<DrawCommand>,
    layout: &LayoutNode,
    info: &InfoNode,
    viewport: (f32, f32),
) {
    let (scroll_x, scroll_y) = info.kind.scroll_offsets();
    let root_viewport = StickyViewport {
        top_left: (-scroll_x, scroll_y),
        size: viewport,
    };
    let containing = layout
        .layout_box
        .iter()
        .next()
        .map_or((0.0, 0.0), |b| (b.content_box.width, b.content_box.height));
    let origin = layout
        .layout_box
        .iter()
        .next()
        .map_or((0.0, 0.0), |b| (b.content_box.x, b.content_box.y));
    let mut popups: Vec<(Vec<DrawCommand>, (f32, f32))> = Vec::new();
    generate_draw_commands_inner(
        cmd_buf,
        layout,
        info,
        (0.0, 0.0),
        root_viewport,
        containing,
        origin,
        &mut popups,
        true,
    );
    // Top-layer popups render after every other box, outside all ancestor
    // clips and transforms.
    for (commands, (tx, ty)) in popups {
        if tx != 0.0 || ty != 0.0 {
            cmd_buf.push(DrawCommand::PushTransform {
                transform: AffineTransform::translate(tx, ty),
            });
        }
        cmd_buf.extend(commands);
        if tx != 0.0 || ty != 0.0 {
            cmd_buf.push(DrawCommand::PopTransform);
        }
    }
}

/// Content-box origin of a child layout node in page space, derived from the
/// parent's page-space origin. Children are laid out relative to the parent's
/// content box, so the child's origin is the parent's origin plus the child's
/// content-box offset.
fn child_origin(child: &LayoutNode, parent_origin: (f32, f32)) -> (f32, f32) {
    child.layout_box.iter().next().map_or(parent_origin, |b| {
        (
            parent_origin.0 + b.content_box.x,
            parent_origin.1 + b.content_box.y,
        )
    })
}

/// Recursive draw-command generation.
///
/// `accumulated_scroll` is the sum of the scroll offsets of every scrollable
/// ancestor, expressed in content space (`(x, y)`), i.e. the displacement the
/// current subtree inherits from ancestor scrolling. A `position: fixed` box
/// is positioned relative to the viewport, so the inherited displacement is
/// cancelled by pushing the inverse transform before its own box models and
/// resetting the accumulated scroll for its descendants.
///
/// `viewport` is the nearest scrollport as seen from this node's parent-content
/// space (used to resolve `position: sticky`), `containing` is this node's
/// parent content-box size (the containing block sticky boxes must stay in),
/// `origin` is this node's content-box origin in page space (unscrolled), and
/// `is_root` marks the document root, which always establishes the page
/// scrollport.
///
/// Open popups owned by custom nodes are collected (with the page-space
/// translation of their content-box origin) instead of being drawn inline, so
/// the caller can emit them above all page content.
fn generate_draw_commands_inner(
    cmd_buf: &mut Vec<DrawCommand>,
    layout: &LayoutNode,
    info: &InfoNode,
    accumulated_scroll: (f32, f32),
    viewport: StickyViewport,
    containing: (f32, f32),
    origin: (f32, f32),
    popups: &mut Vec<(Vec<DrawCommand>, (f32, f32))>,
    is_root: bool,
) {
    // Check visibility before pushing position-dependent transforms.
    // Hidden elements return early, so any transform pushed above this point
    // would otherwise leak into subsequent siblings.
    match &info.kind {
        NodeKind::Container { style, .. } | NodeKind::Custom { style, .. } => {
            if matches!(style.visibility, Visibility::Hidden | Visibility::Collapse) {
                return;
            }
        }
        _ => {}
    }

    let mut box_states: Vec<BoxPushState> = Vec::new();

    let is_fixed = layout.style.position.kind == Position::Fixed;
    let is_sticky = layout.style.position.kind == Position::Sticky;
    let is_inline = matches!(layout.layout_box, ui_layout::LayoutBox::InlineBox(_));

    // Cancel the inherited scroll displacement for fixed-position boxes.
    let cancel_scroll = is_fixed && (accumulated_scroll.0 != 0.0 || accumulated_scroll.1 != 0.0);
    if cancel_scroll {
        cmd_buf.push(DrawCommand::PushTransform {
            transform: AffineTransform::translate(-accumulated_scroll.0, accumulated_scroll.1),
        });
    }

    // Shift sticky boxes so each specified inset stays within the visible area
    // of the nearest scrollport. Fixed boxes are positioned relative to the
    // viewport, so sticky offsets never apply to them.
    let sticky_pushed = if is_sticky && !is_fixed {
        match layout.layout_box.iter().next() {
            Some(bm) => {
                let (dx, dy) = bm
                    .sticky_edges
                    .map(|edges| sticky_offset(&edges, &bm.border_box, viewport, containing))
                    .unwrap_or((0.0, 0.0));
                push_transform(cmd_buf, dx, dy)
            }
            None => false,
        }
    } else {
        false
    };

    match &info.kind {
        NodeKind::Text { .. } | NodeKind::LineBreak => unreachable!(),

        NodeKind::Container {
            scroll_x,
            scroll_y,
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
                    *scroll_x || *scroll_y,
                    true,
                ));
            }
        }

        NodeKind::Custom {
            scroll_x,
            scroll_y,
            scroll_offset_x,
            scroll_offset_y,
            style,
            layout_style,
            node,
            text_style,
            text_flow_style,
            ..
        } => {
            for box_model in &layout.layout_box {
                box_states.push(push_box_model(
                    cmd_buf,
                    &box_model,
                    style,
                    *scroll_offset_x,
                    *scroll_offset_y,
                    false,
                    *scroll_x || *scroll_y,
                    false,
                ));
            }

            let size = layout.layout_box.iter().next().map_or_else(
                || node.intrinsic_size(),
                |box_model| ContentSize {
                    width: box_model.content_box.width,
                    height: box_model.content_box.height,
                },
            );
            node.draw_sized(cmd_buf, text_style, text_flow_style, layout_style, size);
            // Collect open popups so they render above every other box,
            // outside all ancestor clips and transforms.
            if let Some(popup) = node.popup(text_style, text_flow_style) {
                let own_scroll = info.kind.scroll_offsets();
                // Fixed boxes are positioned relative to the viewport, so the
                // inherited scroll displacement does not move their popup.
                let inherited_scroll = if is_fixed {
                    (0.0, 0.0)
                } else {
                    accumulated_scroll
                };
                let (mut tx, mut ty) = (
                    origin.0 - inherited_scroll.0 - own_scroll.0,
                    origin.1 - inherited_scroll.1 - own_scroll.1,
                );
                // Sticky boxes shift their whole subtree (and thus their
                // popup) by the sticky offset.
                if is_sticky
                    && !is_fixed
                    && let Some(bm) = layout.layout_box.iter().next()
                    && let Some(edges) = bm.sticky_edges
                {
                    let (dx, dy) = sticky_offset(&edges, &bm.border_box, viewport, containing);
                    tx += dx;
                    ty += dy;
                }
                popups.push((popup.commands, (tx, ty)));
            }
        }
    }

    // Scroll offsets of this node itself; they scroll the node's own content.
    let own_scroll = info.kind.scroll_offsets();
    // A fixed box resets the inherited displacement (already cancelled above),
    // so its descendants only inherit its own scroll offset.
    let child_scroll = if is_fixed {
        own_scroll
    } else {
        (
            accumulated_scroll.0 + own_scroll.0,
            accumulated_scroll.1 + own_scroll.1,
        )
    };

    // Sticky viewport state for this node's subtree, rebased into each child's
    // parent-content space. A node that scrolls its own content (or the root,
    // which the UI layer scrolls directly) becomes the scrollport for its
    // descendants; otherwise the inherited visible region is shifted by the
    // node's content-box offset.
    let child_viewport = if is_root || is_scrollport(&info.kind) {
        let bm = layout.layout_box.iter().next();
        let (scroll_x, scroll_y) = own_scroll;
        StickyViewport {
            top_left: bm.as_ref().map_or((0.0, 0.0), |b| {
                (
                    b.padding_box.x - b.content_box.x - scroll_x,
                    b.padding_box.y - b.content_box.y + scroll_y,
                )
            }),
            size: if is_root {
                viewport.size
            } else {
                bm.map_or(viewport.size, |b| {
                    (b.padding_box.width, b.padding_box.height)
                })
            },
        }
    } else {
        let bm = layout.layout_box.iter().next();
        StickyViewport {
            top_left: bm.map_or(viewport.top_left, |b| {
                (
                    viewport.top_left.0 - b.content_box.x,
                    viewport.top_left.1 - b.content_box.y,
                )
            }),
            size: viewport.size,
        }
    };
    let child_containing = layout
        .layout_box
        .iter()
        .next()
        .map_or(containing, |b| (b.content_box.width, b.content_box.height));

    let mut layout_iter = layout.children.iter();
    let mut positive_stacking_children: Vec<(i32, usize, Vec<DrawCommand>)> = Vec::new();

    for (child_order, child_info) in info.children.iter().enumerate() {
        match &child_info.kind {
            NodeKind::Text {
                text_id,
                style,
                flow_style,
                ..
            } => {
                draw_text(cmd_buf, style, *flow_style, *text_id);
                layout_iter.next();
            }
            NodeKind::LineBreak => {
                layout_iter.next();
            }
            NodeKind::Container { .. } => {
                if let Some(LayoutChild::Node(node)) = layout_iter.next() {
                    let child_origin = child_origin(node, origin);
                    let z_index = child_info.kind.z_index();
                    if z_index > 0 {
                        let mut child_commands = Vec::new();
                        generate_draw_commands_inner(
                            &mut child_commands,
                            node,
                            child_info,
                            child_scroll,
                            child_viewport,
                            child_containing,
                            child_origin,
                            popups,
                            false,
                        );
                        positive_stacking_children.push((z_index, child_order, child_commands));
                    } else {
                        generate_draw_commands_inner(
                            cmd_buf,
                            node,
                            child_info,
                            child_scroll,
                            child_viewport,
                            child_containing,
                            child_origin,
                            popups,
                            false,
                        );
                    }
                }
            }
            NodeKind::Custom {
                node,
                text_style,
                text_flow_style,
                style,
                layout_style,
                ..
            } => {
                match layout_iter.next() {
                    // Block custom element: recurse into the child layout node.
                    Some(LayoutChild::Node(node_layout)) => {
                        let child_origin = child_origin(node_layout, origin);
                        generate_draw_commands_inner(
                            cmd_buf,
                            node_layout,
                            child_info,
                            child_scroll,
                            child_viewport,
                            child_containing,
                            child_origin,
                            popups,
                            false,
                        );
                    }
                    // Inline custom element: consume the Object and draw it
                    // from the layout result stored on the tree child.
                    Some(LayoutChild::Custom(custom_child)) => {
                        if let Some(result) = custom_child.result() {
                            let bm = &result.box_model;
                            let rect = BoxModel {
                                sticky_edges: bm.sticky_edges,
                                border_box: bm.border_box.clone(),
                                padding_box: bm.padding_box.clone(),
                                content_box: bm.content_box.clone(),
                                children_box: bm.children_box.clone(),
                            };
                            let state = push_box_model(
                                cmd_buf, &rect, style, 0.0, 0.0, false, false, false,
                            );
                            node.draw_sized(
                                cmd_buf,
                                text_style,
                                text_flow_style,
                                layout_style,
                                ContentSize {
                                    width: rect.content_box.width,
                                    height: rect.content_box.height,
                                },
                            );
                            pop_box_model(cmd_buf, state);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    positive_stacking_children.sort_by_key(|(z_index, child_order, _)| (*z_index, *child_order));
    for (_, _, commands) in positive_stacking_children {
        cmd_buf.extend(commands);
    }

    if matches!(
        info.kind,
        NodeKind::Container { .. } | NodeKind::Custom { .. }
    ) {
        for state in box_states.iter().rev() {
            pop_box_model(cmd_buf, *state);
        }
    }

    if sticky_pushed {
        cmd_buf.push(DrawCommand::PopTransform);
    }

    if cancel_scroll {
        cmd_buf.push(DrawCommand::PopTransform);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::layouter::types::ContainerRole;
    use crate::engine::renderer_model::geom::AffineTransform;
    use crate::engine::ui::custom_node::{CustomNode, Popup};
    use std::sync::Arc;
    use ui_layout::Style;

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

    #[test]
    fn scratch_background_image_geometry_is_resolved() {
        let (width, height) = resolve_background_image_size(
            1200.0,
            600.0,
            800.0,
            400.0,
            BackgroundSize::Explicit {
                width: BackgroundDimension::Length(624.0),
                height: BackgroundDimension::Length(325.0),
            },
        );
        assert_eq!((width, height), (624.0, 325.0));
        assert_eq!(
            resolve_background_axis(
                800.0,
                width,
                BackgroundPositionAxis::End(BackgroundOffset::Zero),
            ),
            176.0
        );
        assert_eq!(
            resolve_background_axis(
                400.0,
                height,
                BackgroundPositionAxis::Center(BackgroundOffset::Zero),
            ),
            37.5
        );
    }

    #[test]
    fn background_contain_and_cover_preserve_aspect_ratio() {
        assert_eq!(
            resolve_background_image_size(200.0, 100.0, 300.0, 300.0, BackgroundSize::Contain,),
            (300.0, 150.0)
        );
        assert_eq!(
            resolve_background_image_size(200.0, 100.0, 300.0, 300.0, BackgroundSize::Cover),
            (600.0, 300.0)
        );
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
            sticky_edges: None,
            border_box: ui_rect(10.0, 20.0, 120.0, 60.0),
            padding_box: ui_rect(12.0, 22.0, 116.0, 56.0),
            content_box: ui_rect(12.0, 22.0, 116.0, 56.0),
            children_box: ui_rect(12.0, 22.0, 116.0, 56.0),
        };
        let style = ContainerStyle::default();
        let mut buf = Vec::new();
        let state = push_box_model(&mut buf, &box_model, &style, 0.0, 0.0, false, true, true);
        // Scroll/content transforms are no-ops here (zero offsets); border
        // transform + clip + content are pushed while the box is open.
        assert!(buf.len() >= 2);
        // Opening pushes are not yet balanced: a clip and transforms are pending.
        assert!(!count_balanced(&buf));
        pop_box_model(&mut buf, state);
        assert!(count_balanced(&buf));
    }

    #[test]
    fn visible_overflow_does_not_clip_block_contents() {
        let box_model = ui_layout::BoxModel {
            sticky_edges: None,
            border_box: ui_rect(0.0, 0.0, 100.0, 50.0),
            padding_box: ui_rect(0.0, 0.0, 100.0, 50.0),
            content_box: ui_rect(0.0, 0.0, 100.0, 50.0),
            children_box: ui_rect(0.0, 0.0, 120.0, 60.0),
        };
        let mut commands = Vec::new();
        let state = push_box_model(
            &mut commands,
            &box_model,
            &ContainerStyle::default(),
            0.0,
            0.0,
            false,
            false,
            true,
        );
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, DrawCommand::PushClip { .. }))
        );
        pop_box_model(&mut commands, state);
        assert!(count_balanced(&commands));
    }

    #[test]
    fn test_nested_box_models_balanced() {
        let mk_box = |x: f32, y: f32, w: f32, h: f32| ui_layout::BoxModel {
            sticky_edges: None,
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
            true,
            true,
        );
        let inner = push_box_model(
            &mut buf,
            &mk_box(10.0, 10.0, 50.0, 50.0),
            &style,
            0.0,
            0.0,
            false,
            true,
            true,
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

    /// Build an InfoNode subtree matching `generate_draw_commands`' expectation:
    /// `kind` (with `style`), `children` (each with `kind`).
    fn mk_info_node(kind: NodeKind, children: Vec<InfoNode>) -> InfoNode {
        InfoNode {
            kind,
            children,
            dom_id: None,
        }
    }

    /// Collect the sequence of scroll-related transforms in `commands` as
    /// `(translate_x, translate_y)` tuples, in order.
    fn scroll_translates(commands: &[DrawCommand]) -> Vec<(f32, f32)> {
        let mut tx: Vec<f32> = Vec::new();
        let mut ty: Vec<f32> = Vec::new();
        let mut out = Vec::new();
        for cmd in commands {
            match cmd {
                DrawCommand::PushTransform { transform } => {
                    tx.push(transform.apply(0.0, 0.0).0);
                    ty.push(transform.apply(0.0, 0.0).1);
                    out.push((transform.apply(0.0, 0.0).0, transform.apply(0.0, 0.0).1));
                }
                DrawCommand::PopTransform => {
                    if let (Some(x), Some(y)) = (tx.pop(), ty.pop()) {
                        out.push((-x, -y));
                    }
                }
                _ => {}
            }
        }
        out
    }

    #[test]
    fn fixed_node_cancels_inherited_scroll_offset() {
        let ui_rect = |x: f32, y: f32, w: f32, h: f32| ui_layout::Rect {
            x,
            y,
            width: w,
            height: h,
        };
        let mk_box = |x: f32, y: f32, w: f32, h: f32| ui_layout::BoxModel {
            sticky_edges: None,
            border_box: ui_rect(x, y, w, h),
            padding_box: ui_rect(x, y, w, h),
            content_box: ui_rect(x, y, w, h),
            children_box: ui_rect(x, y, w, h),
        };

        // A scrollable ancestor (scrolled by 50px) containing a fixed child.
        let scroller_style = Style::default();
        let mut scroller = LayoutNode::new(scroller_style);
        scroller.layout_box = ui_layout::LayoutBox::BlockBox(mk_box(0.0, 0.0, 100.0, 100.0));

        let mut fixed_style = Style::default();
        fixed_style.position.kind = Position::Fixed;
        let fixed = LayoutNode::new(fixed_style);
        scroller.children = vec![LayoutChild::Node(Box::new(fixed))];

        let scroller_info = mk_info_node(
            NodeKind::Container {
                scroll_x: true,
                scroll_y: true,
                scroll_offset_x: 0.0,
                scroll_offset_y: 50.0,
                style: ContainerStyle::default(),
                role: ContainerRole::Normal,
            },
            vec![mk_info_node(
                NodeKind::Container {
                    scroll_x: false,
                    scroll_y: false,
                    scroll_offset_x: 0.0,
                    scroll_offset_y: 0.0,
                    style: ContainerStyle::default(),
                    role: ContainerRole::Normal,
                },
                Vec::new(),
            )],
        );

        let mut commands = Vec::new();
        generate_draw_commands(&mut commands, &scroller, &scroller_info, (100.0, 100.0));
        assert!(count_balanced(&commands));

        // The fixed child must push a transform cancelling the ancestor's
        // 50px scroll: `(-0, +50)` cancels the inherited `(0, -50)`.
        let translates = scroll_translates(&commands);
        assert!(
            translates.contains(&(0.0, 50.0)),
            "expected a scroll-cancelling transform, got {translates:?}"
        );
    }

    #[test]
    fn positive_z_index_child_paints_after_later_normal_sibling() {
        let box_model = |x: f32| ui_layout::BoxModel {
            sticky_edges: None,
            border_box: ui_rect(x, 0.0, 50.0, 50.0),
            padding_box: ui_rect(x, 0.0, 50.0, 50.0),
            content_box: ui_rect(x, 0.0, 50.0, 50.0),
            children_box: ui_rect(x, 0.0, 50.0, 50.0),
        };
        let mut root = LayoutNode::new(Style::default());
        root.layout_box = ui_layout::LayoutBox::BlockBox(box_model(0.0));
        let mut front = LayoutNode::new(Style::default());
        front.layout_box = ui_layout::LayoutBox::BlockBox(box_model(0.0));
        let mut normal = LayoutNode::new(Style::default());
        normal.layout_box = ui_layout::LayoutBox::BlockBox(box_model(0.0));
        root.children = vec![
            LayoutChild::Node(Box::new(front)),
            LayoutChild::Node(Box::new(normal)),
        ];

        let child_info = |color, z_index| {
            mk_info_node(
                NodeKind::Container {
                    scroll_x: false,
                    scroll_y: false,
                    scroll_offset_x: 0.0,
                    scroll_offset_y: 0.0,
                    style: ContainerStyle {
                        background: Background::Color(color),
                        z_index,
                        ..ContainerStyle::default()
                    },
                    role: ContainerRole::Normal,
                },
                Vec::new(),
            )
        };
        let root_info = mk_info_node(
            NodeKind::Container {
                scroll_x: false,
                scroll_y: false,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                style: ContainerStyle::default(),
                role: ContainerRole::Normal,
            },
            vec![
                child_info(Color(255, 0, 0, 255), Some(10)),
                child_info(Color(0, 0, 255, 255), None),
            ],
        );

        let mut commands = Vec::new();
        generate_draw_commands(&mut commands, &root, &root_info, (100.0, 100.0));
        let colors: Vec<_> = commands
            .iter()
            .filter_map(|command| match command {
                DrawCommand::Fill {
                    paint:
                        Paint {
                            brush: Brush::Solid(color),
                            ..
                        },
                    ..
                } => Some(*color),
                _ => None,
            })
            .collect();
        assert_eq!(colors, vec![Color(0, 0, 255, 255), Color(255, 0, 0, 255)]);
        assert!(count_balanced(&commands));
    }

    fn sticky_viewport(scroll: (f32, f32)) -> StickyViewport {
        StickyViewport {
            top_left: (-scroll.0, scroll.1),
            size: (800.0, 500.0),
        }
    }

    fn sticky_edges(top: Option<f32>, bottom: Option<f32>) -> EdgeOption {
        EdgeOption {
            left: None,
            top,
            right: None,
            bottom,
        }
    }

    #[test]
    fn test_sticky_offset_top_sticks_to_viewport_top() {
        // Natural y=600, viewport scrolled 600px: the box's top edge is pinned
        // at the viewport top + 10.
        let (dx, dy) = sticky_offset(
            &sticky_edges(Some(10.0), None),
            &ui_rect(0.0, 600.0, 100.0, 50.0),
            sticky_viewport((0.0, 600.0)),
            (800.0, 2000.0),
        );
        assert_eq!((dx, dy), (0.0, 10.0));
    }

    #[test]
    fn test_sticky_offset_bottom_pushes_up() {
        // Natural bottom (1000) is below the viewport bottom minus the inset
        // (490): the box is pulled up by 510.
        let (dx, dy) = sticky_offset(
            &sticky_edges(None, Some(10.0)),
            &ui_rect(0.0, 900.0, 100.0, 100.0),
            sticky_viewport((0.0, 0.0)),
            (800.0, 2000.0),
        );
        assert_eq!((dx, dy), (0.0, -510.0));
    }

    #[test]
    fn test_sticky_offset_no_movement_when_in_view() {
        let (dx, dy) = sticky_offset(
            &sticky_edges(Some(10.0), None),
            &ui_rect(0.0, 50.0, 100.0, 50.0),
            sticky_viewport((0.0, 0.0)),
            (800.0, 2000.0),
        );
        assert_eq!((dx, dy), (0.0, 0.0));
    }

    #[test]
    fn test_sticky_offset_containing_block_hi_clamp() {
        // A box taller than the visible area near the container end must not
        // be pushed past the containing block bottom (natural bottom is
        // already at 1300, the container end).
        let (dx, dy) = sticky_offset(
            &sticky_edges(Some(10.0), None),
            &ui_rect(0.0, 900.0, 100.0, 400.0),
            sticky_viewport((0.0, 900.0)),
            (800.0, 1300.0),
        );
        assert_eq!((dx, dy), (0.0, 0.0));
    }

    #[test]
    fn test_sticky_offset_inset_fit_ignores_end_edge_without_room() {
        // Both insets set but the sticky view rectangle is shorter than the
        // box: the bottom inset is ignored, the box sticks to its top edge.
        let (dx, dy) = sticky_offset(
            &sticky_edges(Some(10.0), Some(10.0)),
            &ui_rect(0.0, 100.0, 100.0, 500.0),
            sticky_viewport((0.0, 0.0)),
            (800.0, 2000.0),
        );
        assert_eq!((dx, dy), (0.0, 0.0));
    }

    #[test]
    fn test_sticky_offset_horizontal_right_pushes_left() {
        // Box spans [850, 950], off-screen right of the 800-wide viewport; a
        // right inset of 10 pulls it left so its right edge sits at 790.
        let edges = EdgeOption {
            left: None,
            top: None,
            right: Some(10.0),
            bottom: None,
        };
        let (dx, dy) = sticky_offset(
            &edges,
            &ui_rect(850.0, 0.0, 100.0, 50.0),
            sticky_viewport((0.0, 0.0)),
            (2000.0, 2000.0),
        );
        assert_eq!((dx, dy), (-160.0, 0.0));
    }

    #[test]
    fn sticky_node_pushes_sticky_offset_transform() {
        // Root acts as the page scrollport (scrolled 600px). A sticky child at
        // natural y=600 with top:10 must push a translate(0, 10).
        let ui_rect = |x: f32, y: f32, w: f32, h: f32| ui_layout::Rect {
            x,
            y,
            width: w,
            height: h,
        };
        let mk_box = |x: f32, y: f32, w: f32, h: f32| ui_layout::BoxModel {
            sticky_edges: None,
            border_box: ui_rect(x, y, w, h),
            padding_box: ui_rect(x, y, w, h),
            content_box: ui_rect(x, y, w, h),
            children_box: ui_rect(x, y, w, h),
        };

        let root_style = Style::default();
        let mut root = LayoutNode::new(root_style);
        root.layout_box = ui_layout::LayoutBox::BlockBox(mk_box(0.0, 0.0, 800.0, 2000.0));

        let mut sticky_style = Style::default();
        sticky_style.position.kind = Position::Sticky;
        let mut sticky = LayoutNode::new(sticky_style);
        let mut sticky_bm = mk_box(0.0, 600.0, 800.0, 100.0);
        sticky_bm.sticky_edges = Some(sticky_edges(Some(10.0), None));
        sticky.layout_box = ui_layout::LayoutBox::BlockBox(sticky_bm);
        root.children = vec![LayoutChild::Node(Box::new(sticky))];

        let root_info = mk_info_node(
            NodeKind::Container {
                scroll_x: false,
                scroll_y: false,
                scroll_offset_x: 0.0,
                scroll_offset_y: 600.0,
                style: ContainerStyle::default(),
                role: ContainerRole::Normal,
            },
            vec![mk_info_node(
                NodeKind::Container {
                    scroll_x: false,
                    scroll_y: false,
                    scroll_offset_x: 0.0,
                    scroll_offset_y: 0.0,
                    style: ContainerStyle::default(),
                    role: ContainerRole::Normal,
                },
                Vec::new(),
            )],
        );

        let mut commands = Vec::new();
        generate_draw_commands(&mut commands, &root, &root_info, (800.0, 500.0));
        assert!(count_balanced(&commands));

        let translates = scroll_translates(&commands);
        assert!(
            translates.contains(&(0.0, 10.0)),
            "expected a sticky top offset, got {translates:?}"
        );
    }

    #[test]
    fn sticky_node_bottom_pushes_up() {
        // Same setup but bottom:10, unscrolled, natural y=900: pulled up by 510.
        let ui_rect = |x: f32, y: f32, w: f32, h: f32| ui_layout::Rect {
            x,
            y,
            width: w,
            height: h,
        };
        let mk_box = |x: f32, y: f32, w: f32, h: f32| ui_layout::BoxModel {
            sticky_edges: None,
            border_box: ui_rect(x, y, w, h),
            padding_box: ui_rect(x, y, w, h),
            content_box: ui_rect(x, y, w, h),
            children_box: ui_rect(x, y, w, h),
        };

        let root_style = Style::default();
        let mut root = LayoutNode::new(root_style);
        root.layout_box = ui_layout::LayoutBox::BlockBox(mk_box(0.0, 0.0, 800.0, 2000.0));

        let mut sticky_style = Style::default();
        sticky_style.position.kind = Position::Sticky;
        let mut sticky = LayoutNode::new(sticky_style);
        let mut sticky_bm = mk_box(0.0, 900.0, 800.0, 100.0);
        sticky_bm.sticky_edges = Some(sticky_edges(None, Some(10.0)));
        sticky.layout_box = ui_layout::LayoutBox::BlockBox(sticky_bm);
        root.children = vec![LayoutChild::Node(Box::new(sticky))];

        let root_info = mk_info_node(
            NodeKind::Container {
                scroll_x: false,
                scroll_y: false,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                style: ContainerStyle::default(),
                role: ContainerRole::Normal,
            },
            vec![mk_info_node(
                NodeKind::Container {
                    scroll_x: false,
                    scroll_y: false,
                    scroll_offset_x: 0.0,
                    scroll_offset_y: 0.0,
                    style: ContainerStyle::default(),
                    role: ContainerRole::Normal,
                },
                Vec::new(),
            )],
        );

        let mut commands = Vec::new();
        generate_draw_commands(&mut commands, &root, &root_info, (800.0, 500.0));
        assert!(count_balanced(&commands));

        let translates = scroll_translates(&commands);
        assert!(
            translates.contains(&(0.0, -510.0)),
            "expected a sticky bottom offset, got {translates:?}"
        );
    }

    /// A custom node that reports an open popup with a single fill command.
    #[derive(Debug)]
    struct PopupNode {
        open: bool,
        box_height: f32,
        popup_height: f32,
    }

    impl CustomNode for PopupNode {
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
                height: self.box_height,
            }
        }

        fn popup(
            &self,
            _text_style: &TextStyle,
            _text_flow_style: &TextFlowStyle,
        ) -> Option<Popup> {
            self.open.then(|| Popup {
                rect: crate::engine::renderer_model::Rect {
                    x: 0.0,
                    y: self.box_height,
                    width: 120.0,
                    height: self.popup_height,
                },
                commands: vec![DrawCommand::Fill {
                    path: rect_path(0.0, self.box_height, 120.0, self.popup_height),
                    rule: FillRule::NonZero,
                    paint: Paint {
                        brush: Brush::Solid(Color(255, 0, 0, 255)),
                        opacity: 1.0,
                    },
                }],
            })
        }
    }

    #[test]
    fn popup_is_emitted_as_top_layer_at_node_position() {
        // A select-like node with an open popup nested inside a scrollable
        // container: node content origin (10+5, 20+0) = (15, 20), inherited
        // scroll offset 50 → popup translate (15, 20-50) = (15, -30).
        let node: Arc<dyn CustomNode> = Arc::new(PopupNode {
            open: true,
            box_height: 28.0,
            popup_height: 84.0,
        });

        let mk_box = |x: f32, y: f32, w: f32, h: f32| ui_layout::BoxModel {
            sticky_edges: None,
            border_box: ui_rect(x, y, w, h),
            padding_box: ui_rect(x, y, w, h),
            content_box: ui_rect(x, y, w, h),
            children_box: ui_rect(x, y, w, h),
        };

        let mut root = LayoutNode::new(Style::default());
        root.layout_box = ui_layout::LayoutBox::BlockBox(mk_box(0.0, 0.0, 200.0, 200.0));

        let mut scroller = LayoutNode::new(Style::default());
        scroller.layout_box = ui_layout::LayoutBox::BlockBox(mk_box(10.0, 20.0, 160.0, 100.0));

        let mut custom = LayoutNode::new(Style::default());
        custom.layout_box = ui_layout::LayoutBox::BlockBox(mk_box(5.0, 0.0, 120.0, 28.0));
        scroller.children = vec![LayoutChild::Node(Box::new(custom))];
        root.children = vec![LayoutChild::Node(Box::new(scroller))];

        let root_info = mk_info_node(
            NodeKind::Container {
                scroll_x: false,
                scroll_y: false,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                style: ContainerStyle::default(),
                role: ContainerRole::Normal,
            },
            vec![mk_info_node(
                NodeKind::Container {
                    scroll_x: false,
                    scroll_y: true,
                    scroll_offset_x: 0.0,
                    scroll_offset_y: 50.0,
                    style: ContainerStyle::default(),
                    role: ContainerRole::Normal,
                },
                vec![mk_info_node(
                    NodeKind::Custom {
                        node,
                        scroll_x: false,
                        scroll_y: false,
                        scroll_offset_x: 0.0,
                        scroll_offset_y: 0.0,
                        style: ContainerStyle::default(),
                        layout_style: Style::default(),
                        text_style: TextStyle::default(),
                        text_flow_style: TextFlowStyle::default(),
                    },
                    Vec::new(),
                )],
            )],
        );

        let mut commands = Vec::new();
        generate_draw_commands(&mut commands, &root, &root_info, (200.0, 200.0));
        assert!(count_balanced(&commands));

        // The default styles paint nothing, so the only fill in the buffer is
        // the popup, emitted after every clip/transform.
        let fills = commands
            .iter()
            .filter(|cmd| matches!(cmd, DrawCommand::Fill { .. }))
            .count();
        assert_eq!(fills, 1, "popup must be the only painted box");

        match &commands[commands.len() - 3..] {
            [
                DrawCommand::PushTransform { transform },
                DrawCommand::Fill { .. },
                DrawCommand::PopTransform,
            ] => {
                assert_eq!(transform.apply(0.0, 0.0), (15.0, -30.0));
            }
            _ => panic!("expected trailing popup PushTransform/Fill/PopTransform"),
        }
    }

    #[test]
    fn closed_popup_is_not_emitted() {
        let node: Arc<dyn CustomNode> = Arc::new(PopupNode {
            open: false,
            box_height: 28.0,
            popup_height: 84.0,
        });

        let mk_box = |x: f32, y: f32, w: f32, h: f32| ui_layout::BoxModel {
            sticky_edges: None,
            border_box: ui_rect(x, y, w, h),
            padding_box: ui_rect(x, y, w, h),
            content_box: ui_rect(x, y, w, h),
            children_box: ui_rect(x, y, w, h),
        };

        let mut root = LayoutNode::new(Style::default());
        root.layout_box = ui_layout::LayoutBox::BlockBox(mk_box(0.0, 0.0, 200.0, 200.0));
        let mut custom = LayoutNode::new(Style::default());
        custom.layout_box = ui_layout::LayoutBox::BlockBox(mk_box(0.0, 0.0, 120.0, 28.0));
        root.children = vec![LayoutChild::Node(Box::new(custom))];

        let root_info = mk_info_node(
            NodeKind::Container {
                scroll_x: false,
                scroll_y: false,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                style: ContainerStyle::default(),
                role: ContainerRole::Normal,
            },
            vec![mk_info_node(
                NodeKind::Custom {
                    node,
                    scroll_x: false,
                    scroll_y: false,
                    scroll_offset_x: 0.0,
                    scroll_offset_y: 0.0,
                    style: ContainerStyle::default(),
                    layout_style: Style::default(),
                    text_style: TextStyle::default(),
                    text_flow_style: TextFlowStyle::default(),
                },
                Vec::new(),
            )],
        );

        let mut commands = Vec::new();
        generate_draw_commands(&mut commands, &root, &root_info, (200.0, 200.0));
        assert!(count_balanced(&commands));
        assert_eq!(
            commands
                .iter()
                .filter(|cmd| matches!(cmd, DrawCommand::Fill { .. }))
                .count(),
            0
        );
    }
}
