//! Draw command definition for rendering, which represents drawing instructions.

// Path definitions moved to path.rs
use crate::engine::layouter::text_layouter::TextFlowLayouter;
use crate::engine::layouter::types::{
    Background, BorderRadius, Color, ContainerStyle, CornerRadius, Gradient, InfoNode, NodeKind,
    TextDecoration, TextStyle,
};
use crate::engine::renderer_model::path::{
    Path, append_quarter_ellipse, clamp_radii, rect_path, rounded_rect_path,
};
use crate::engine::ui::custom_bridge::get_custom_inline_result;
use smol_str::SmolStr;
use ui_layout::{LayoutChild, LayoutNode};

/// An affine transformation matrix (2D), in row-major convention with an
/// implicit translation.
#[derive(Debug, Clone, Copy)]
pub struct AffineTransform {
    pub m11: f32,
    pub m12: f32,
    pub m21: f32,
    pub m22: f32,
    pub dx: f32,
    pub dy: f32,
}

impl AffineTransform {
    /// The identity transform.
    pub const fn identity() -> Self {
        AffineTransform {
            m11: 1.0,
            m12: 0.0,
            m21: 0.0,
            m22: 1.0,
            dx: 0.0,
            dy: 0.0,
        }
    }

    /// A pure translation transform.
    pub fn translate(dx: f32, dy: f32) -> Self {
        AffineTransform {
            m11: 1.0,
            m12: 0.0,
            m21: 0.0,
            m22: 1.0,
            dx,
            dy,
        }
    }

    /// A pure scale transform.
    pub const fn scale(sx: f32, sy: f32) -> Self {
        AffineTransform {
            m11: sx,
            m12: 0.0,
            m21: 0.0,
            m22: sy,
            dx: 0.0,
            dy: 0.0,
        }
    }

    /// A rotation transform by `angle` radians.
    pub fn rotate(angle: f32) -> Self {
        let c = angle.cos();
        let s = angle.sin();
        AffineTransform {
            m11: c,
            m12: -s,
            m21: s,
            m22: c,
            dx: 0.0,
            dy: 0.0,
        }
    }

    /// Apply this transform to a point.
    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            x * self.m11 + y * self.m12 + self.dx,
            x * self.m21 + y * self.m22 + self.dy,
        )
    }

    /// Compose: `self` after `rhs` (rhs is applied first, then self).
    pub fn then(&self, rhs: &AffineTransform) -> Self {
        AffineTransform {
            m11: self.m11 * rhs.m11 + self.m12 * rhs.m21,
            m12: self.m11 * rhs.m12 + self.m12 * rhs.m22,
            m21: self.m21 * rhs.m11 + self.m22 * rhs.m21,
            m22: self.m21 * rhs.m12 + self.m22 * rhs.m22,
            dx: self.m11 * rhs.dx + self.m12 * rhs.dy + self.dx,
            dy: self.m21 * rhs.dx + self.m22 * rhs.dy + self.dy,
        }
    }

    /// Returns the inverse transform, or `None` when the matrix is singular.
    pub fn inverse(&self) -> Option<Self> {
        let det = self.m11 * self.m22 - self.m12 * self.m21;
        if det.abs() < f32::EPSILON {
            return None;
        }
        let inv_det = 1.0 / det;
        Some(AffineTransform {
            m11: self.m22 * inv_det,
            m12: -self.m12 * inv_det,
            m21: -self.m21 * inv_det,
            m22: self.m11 * inv_det,
            dx: (self.m21 * self.dy - self.m22 * self.dx) * inv_det,
            dy: (self.m12 * self.dx - self.m11 * self.dy) * inv_det,
        })
    }
}

/// Fill rule for path filling.
///
/// The GPU rasterizer currently ignores this value and always fills the path
/// polygon directly; only simple, convex subpaths are rendered correctly.
#[derive(Debug, Clone, Copy)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

// --------------------------------
// Path helpers
// --------------------------------
// Shape helpers are provided by path.rs

/// An axis-aligned rectangle.
#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    /// Creates a new rectangle.
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Returns `true` if the rectangle contains the given point.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }

    /// Returns the intersection of two rectangles, or `None` if they do not
    /// overlap.
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = (self.x + self.width).min(other.x + other.width);
        let y2 = (self.y + self.height).min(other.y + other.height);
        if x2 > x1 && y2 > y1 {
            Some(Rect::new(x1, y1, x2 - x1, y2 - y1))
        } else {
            None
        }
    }
}

/// A fill source: a solid color or a gradient.
#[derive(Debug, Clone)]
pub enum Brush {
    Solid(Color),
    Gradient(Gradient),
}

/// How a path is painted: the brush plus an opacity multiplier.
#[derive(Debug, Clone)]
pub struct Paint {
    pub brush: Brush,
    pub opacity: f32,
}

/// A drawing instruction for the GPU renderer.
#[derive(Debug, Clone)]
pub enum DrawCommand {
    /// Fill a path.
    ///
    /// The `rule` field is reserved; the GPU rasterizer currently ignores it
    /// and only renders simple, roughly convex polygon subpaths. The `opacity`
    /// is applied to solid color fills.
    Fill {
        path: Path,
        paint: Paint,
        rule: FillRule,
    },
    /// Draw a text run at `(x, y)`.
    DrawText {
        x: f32,
        y: f32,
        text: SmolStr,
        style: TextStyle,
    },
    /// Push a clip region given by a path.
    ///
    /// Non-rectangular paths are approximated by their bounding box.
    PushClip {
        path: Path,
        rule: FillRule,
    },
    PopClip,
    /// Push a coordinate transform.
    PushTransform {
        transform: AffineTransform,
    },
    PopTransform,

    /// Delegate rendering to a platform-native system UI element.
    ///
    /// The renderer composites or renders the element identified by
    /// [`SystemUiKind`] at the given rectangle within the current
    /// coordinate space.
    SystemUi {
        kind: SystemUiKind,
        rect: Rect,
    },
}

/// Discriminator for [`DrawCommand::SystemUi`].
/// Stub
#[derive(Debug, Clone)]
pub enum SystemUiKind {
    /// Composite an external surface (WebView, iframe, …).
    WebView { surface_id: usize },
    /// Render a platform-native input widget.
    Input {
        value: SmolStr,
        placeholder: SmolStr,
    },
}

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
            c.x.resolve_with(Some(box_w), 0.0, 0.0).unwrap_or(0.0).max(0.0),
            c.y.resolve_with(Some(box_h), 0.0, 0.0).unwrap_or(0.0).max(0.0),
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
    append_quarter_ellipse(&mut path, rtl_x, rtl_y, rtl_x, rtl_y, (0.0, rtl_y), (rtl_x, 0.0));
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
    path.line_to(w - br, rtr_y);
    append_quarter_ellipse(
        &mut path,
        w - rtr_x,
        rtr_y,
        itr_x,
        itr_y,
        (w - br, rtr_y),
        (w - rtr_x, bt),
    );
    path.line_to(itl_x, bt);
    append_quarter_ellipse(
        &mut path,
        rtl_x,
        rtl_y,
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
    inner = clamp_radii(inner, padding_box.width.max(0.0), padding_box.height.max(0.0));

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
            push_fill(cmd_buf, rect_path(0.0, h - bw_bottom, w, bw_bottom), bc.bottom);
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
            rect_path(w - bw_right, outer[1].1, bw_right, h - outer[1].1 - outer[2].1),
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
    inner = clamp_radii(inner, padding_box.width.max(0.0), padding_box.height.max(0.0));

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
                paint: Paint {
                    brush: Brush::Gradient(g.clone()),
                    opacity: 1.0,
                },
                rule: FillRule::NonZero,
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

    let state = BoxPushState {
        clip,
        content,
        scroll,
        ..state
    };

    state
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
            node,
            text_style,
            ..
        } => {
            let effective_style = node.background_color().map(|c| ContainerStyle {
                background: Background::Color(c),
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

            node.draw(cmd_buf, text_style);
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
                layout_id,
                ..
            } => {
                if let Some(LayoutChild::Node(node_layout)) = layout_iter.next() {
                    // Block custom element: recurse into the child layout node.
                    generate_draw_commands(cmd_buf, node_layout, child_info);
                } else if layout_id.is_some() {
                    // Inline custom element: consume the Object and draw directly.
                    layout_iter.next();

                    let effective_style = node.background_color().map(|c| ContainerStyle {
                        background: Background::Color(c),
                        ..style.clone()
                    });
                    let style_ref = effective_style.as_ref().unwrap_or(style);

                    if let Some(result) = get_custom_inline_result(layout_id.unwrap()) {
                        for span in &result.spans {
                            let cx = span.line_pos.0 - text_origin.0;
                            let cy = span.line_pos.1 - text_origin.1;
                            let cw = result.width;
                            let ch = result.height;

                            let pb_x = cx - result.padding_left;
                            let pb_y = cy - result.padding_top;
                            let pb_w = cw + result.padding_left + result.padding_right;
                            let pb_h = ch + result.padding_top + result.padding_bottom;

                            let bb_x = pb_x - result.border_left;
                            let bb_y = pb_y - result.border_top;
                            let bb_w = pb_w + result.border_left + result.border_right;
                            let bb_h = pb_h + result.border_top + result.border_bottom;

                            let rect = ui_layout::BoxModel {
                                border_box: ui_layout::Rect {
                                    x: bb_x,
                                    y: bb_y,
                                    width: bb_w,
                                    height: bb_h,
                                },
                                padding_box: ui_layout::Rect {
                                    x: pb_x,
                                    y: pb_y,
                                    width: pb_w,
                                    height: pb_h,
                                },
                                content_box: ui_layout::Rect {
                                    x: cx,
                                    y: cy,
                                    width: cw,
                                    height: ch,
                                },
                                children_box: ui_layout::Rect {
                                    x: cx,
                                    y: cy,
                                    width: cw,
                                    height: ch,
                                },
                            };
                            push_box_model(cmd_buf, &rect, style_ref, 0.0, 0.0, true);
                            node.draw(cmd_buf, text_style);
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
