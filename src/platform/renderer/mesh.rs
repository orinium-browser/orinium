//! GPU-agnostic geometry layer.
//!
//! Interprets [`DrawCommand`]s into a vertex mesh and text sections without
//! touching `wgpu`, so the whole command → geometry path is unit-testable
//! without a GPU. [`GpuRenderer`](crate::platform::renderer::gpu::GpuRenderer)
//! consumes the produced mesh and uploads it to GPU buffers.

use std::sync::Arc;

use orinium_text::TextLayout;
use smol_str::SmolStr;

use crate::engine::layouter::types::{
    Color, ColorStop, Gradient, GradientKind, LineHeight, RadialShape, RadialSizeKind,
    TextFlowStyle, TextStyle,
};
use crate::engine::renderer_model::{
    AffineTransform, Brush, DrawCommand, Image, Paint, Path, Rect,
};

/// A single vertex: NDC position plus linear-space color.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

/// A positioned, clipped text layout ready for the glyph pipeline.
#[derive(Clone)]
pub struct TextSection {
    pub screen_position: (f32, f32),
    pub clip_origin: (f32, f32),
    pub bounds: (f32, f32),
    pub font_weight: u16,
    pub layout: Arc<TextLayout>,
}

/// A clipped image rectangle ready for the textured GPU pipeline.
#[derive(Clone)]
pub struct ImageSection {
    pub image: Image,
    pub rect: Rect,
    pub uv: Rect,
    pub opacity: f32,
}

/// One draw in command order. The GPU renders [`Mesh::draw_items`] in order so
/// that later commands composite over earlier ones (e.g. text drawn after a
/// shape is no longer forced on top by buffer-type passes).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DrawItem {
    /// A range of [`Mesh::vertices`] to draw with the shape pipeline.
    Fill {
        vertex_start: u32,
        vertex_count: u32,
    },
    /// An index into [`Mesh::images`].
    Image(usize),
    /// An index into [`Mesh::sections`].
    Text(usize),
}

/// Source of pre-laid-out text, decoupled from the [`MeshBuilder`] so that the
/// geometry layer stays free of font-system / GPU state.
pub trait TextLayoutSource {
    /// Lay out `text` with `style` (already in physical pixel units).
    fn layout_text(
        &mut self,
        text: &str,
        style: &TextStyle,
        flow_style: TextFlowStyle,
    ) -> Option<Arc<TextLayout>>;
}

/// The result of interpreting a command list: CPU-side vertices and text.
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub draw_items: Vec<DrawItem>,
    pub sections: Vec<TextSection>,
    pub images: Vec<ImageSection>,
}

/// An axis-aligned clip region in logical (pre-scale-factor) coordinates.
#[derive(Clone, Copy)]
struct ClipRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

/// Builds a [`Mesh`] from [`DrawCommand`]s, reusing allocations across frames.
pub struct MeshBuilder {
    screen_width: f32,
    screen_height: f32,
    scale_factor: f32,
    enable_text_culling: bool,

    mesh: Mesh,

    transform_stack: Vec<AffineTransform>,
    clip_stack: Vec<ClipRect>,

    // Scratch buffers, reused across frames to avoid allocation churn.
    ring_scratch: Vec<Vec<(f32, f32)>>,
    clip_in: Vec<(f32, f32)>,
    clip_out: Vec<(f32, f32)>,
    tri_indices: Vec<[u32; 3]>,
}

impl MeshBuilder {
    /// Creates a builder for a viewport of `screen_width` × `screen_height`
    /// physical pixels at the given scale factor.
    pub fn new(screen_width: f32, screen_height: f32, scale_factor: f32) -> Self {
        Self {
            screen_width,
            screen_height,
            scale_factor,
            enable_text_culling: true,
            mesh: Mesh {
                vertices: Vec::new(),
                draw_items: Vec::new(),
                sections: Vec::new(),
                images: Vec::new(),
            },
            transform_stack: vec![AffineTransform::identity()],
            clip_stack: vec![ClipRect {
                x: 0.0,
                y: 0.0,
                w: screen_width,
                h: screen_height,
            }],
            ring_scratch: Vec::new(),
            clip_in: Vec::new(),
            clip_out: Vec::new(),
            tri_indices: Vec::new(),
        }
    }

    pub fn set_screen_size(&mut self, width: f32, height: f32) {
        self.screen_width = width;
        self.screen_height = height;
    }

    pub fn set_scale_factor(&mut self, scale_factor: f32) {
        self.scale_factor = scale_factor;
    }

    pub fn set_text_culling(&mut self, enabled: bool) {
        self.enable_text_culling = enabled;
    }

    /// Interprets `commands` and rebuilds the internal mesh. Returns the
    /// freshly built mesh, whose buffers are reused on the next call.
    pub fn build(
        &mut self,
        commands: &[DrawCommand],
        text: &mut Option<&mut dyn TextLayoutSource>,
    ) -> &Mesh {
        self.mesh.vertices.clear();
        self.mesh.draw_items.clear();
        self.mesh.sections.clear();
        self.mesh.images.clear();

        self.transform_stack.clear();
        self.transform_stack.push(AffineTransform::identity());
        self.clip_stack.clear();
        self.clip_stack.push(ClipRect {
            x: 0.0,
            y: 0.0,
            w: self.screen_width,
            h: self.screen_height,
        });

        for command in commands {
            match command {
                DrawCommand::PushTransform { transform } => {
                    let parent = *self.transform_stack.last().unwrap();
                    self.transform_stack.push(parent.then(transform));
                }
                DrawCommand::PopTransform => {
                    if self.transform_stack.len() > 1 {
                        self.transform_stack.pop();
                    }
                }
                DrawCommand::PushClip { path, rule: _ } => {
                    let t = *self.transform_stack.last().unwrap();
                    let parent = self.clip_stack.last().copied();
                    self.clip_stack.push(push_clip(&t, path, parent));
                }
                DrawCommand::PopClip => {
                    if self.clip_stack.len() > 1 {
                        self.clip_stack.pop();
                    }
                }
                DrawCommand::Fill {
                    path,
                    paint,
                    rule: _,
                } => {
                    let t = *self.transform_stack.last().unwrap();
                    let clip = *self.clip_stack.last().unwrap();
                    let vertices_before = self.mesh.vertices.len() as u32;
                    let images_before = self.mesh.images.len();
                    self.emit_fill(&t, &clip, path, paint);
                    if self.mesh.images.len() > images_before {
                        self.mesh.draw_items.push(DrawItem::Image(images_before));
                    } else {
                        let vertex_count = self.mesh.vertices.len() as u32 - vertices_before;
                        if vertex_count > 0 {
                            self.mesh.draw_items.push(DrawItem::Fill {
                                vertex_start: vertices_before,
                                vertex_count,
                            });
                        }
                    }
                }
                DrawCommand::DrawText {
                    x,
                    y,
                    text: tstr,
                    style,
                    flow_style,
                } => {
                    let t = *self.transform_stack.last().unwrap();
                    let clip = *self.clip_stack.last().unwrap();
                    let sections_before = self.mesh.sections.len();
                    self.emit_text(text, *x, *y, tstr, style, flow_style, &t, &clip);
                    for section in sections_before..self.mesh.sections.len() {
                        self.mesh.draw_items.push(DrawItem::Text(section));
                    }
                }
                DrawCommand::SystemUi { .. } => {
                    // TODO: implement composite/render system UI element
                }
            }
        }

        &self.mesh
    }

    fn emit_fill(&mut self, t: &AffineTransform, clip: &ClipRect, path: &Path, paint: &Paint) {
        let sf = self.scale_factor;

        let rings = path.subpaths();
        if rings.is_empty() {
            return;
        }

        // Transform every subpath into physical (scaled) coordinates and
        // compute the combined bounding box for quick rejection.
        self.ring_scratch.clear();
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for ring in &rings {
            let mut transformed = Vec::with_capacity(ring.len());
            let mut prev: Option<(f32, f32)> = None;
            for (px, py) in ring {
                let (tx, ty) = t.apply(*px, *py);
                let (sx, sy) = (tx * sf, ty * sf);
                if prev == Some((sx, sy)) {
                    continue;
                }
                prev = Some((sx, sy));
                min_x = min_x.min(sx);
                min_y = min_y.min(sy);
                max_x = max_x.max(sx);
                max_y = max_y.max(sy);
                transformed.push((sx, sy));
            }
            if transformed.len() >= 3 {
                self.ring_scratch.push(transformed);
            }
        }
        if self.ring_scratch.is_empty() {
            return;
        }

        let clip_l = clip.x * sf;
        let clip_t = clip.y * sf;
        let clip_r = (clip.x + clip.w) * sf;
        let clip_b = (clip.y + clip.h) * sf;

        // Quick reject by bounding box.
        if max_x <= clip_l || min_x >= clip_r || max_y <= clip_t || min_y >= clip_b {
            return;
        }

        match &paint.brush {
            Brush::Solid(color) => {
                let mut color_arr = color.to_linear_f32_array();
                color_arr[3] *= paint.opacity;
                for ring in &self.ring_scratch {
                    let count = clip_ring_to_rect(
                        ring,
                        clip_l,
                        clip_t,
                        clip_r,
                        clip_b,
                        &mut self.clip_in,
                        &mut self.clip_out,
                    );
                    if count == 0 {
                        continue;
                    }
                    emit_polygon_fill_with_colors(
                        &mut self.mesh.vertices,
                        &self.clip_out[..count],
                        &mut self.tri_indices,
                        self.screen_width,
                        self.screen_height,
                        |_, _| color_arr,
                    );
                }
            }
            Brush::Gradient(gradient) => {
                let x1 = min_x.max(clip_l);
                let y1 = min_y.max(clip_t);
                let x2 = max_x.min(clip_r);
                let y2 = max_y.min(clip_b);
                if x2 <= x1 || y2 <= y1 {
                    return;
                }

                // The gradient is sampled over the full (unclipped) extent so
                // that clipped regions keep the correct color distribution.
                let logical_corners = [
                    (min_x, min_y),
                    (min_x, max_y),
                    (max_x, min_y),
                    (max_x, max_y),
                ];

                match &gradient.kind {
                    GradientKind::Linear { .. } => {
                        // A linear gradient is affine, so sampling it at the
                        // triangulated path's vertices is exact.
                        let (dir_x, dir_y, min_p, max_p) =
                            linear_gradient_extent(gradient, &logical_corners);
                        let range = max_p - min_p;
                        for ring in &self.ring_scratch {
                            let count = clip_ring_to_rect(
                                ring,
                                clip_l,
                                clip_t,
                                clip_r,
                                clip_b,
                                &mut self.clip_in,
                                &mut self.clip_out,
                            );
                            if count == 0 {
                                continue;
                            }
                            emit_polygon_fill_with_colors(
                                &mut self.mesh.vertices,
                                &self.clip_out[..count],
                                &mut self.tri_indices,
                                self.screen_width,
                                self.screen_height,
                                |sx, sy| {
                                    let p = sx * dir_x + sy * dir_y;
                                    let t = if range > 0.0 {
                                        (p - min_p) / range
                                    } else {
                                        0.0
                                    };
                                    let mut c = sample_gradient_stops(&gradient.stops, t)
                                        .to_linear_f32_array();
                                    c[3] *= paint.opacity;
                                    c
                                },
                            );
                        }
                    }
                    GradientKind::Radial { .. } => {
                        let (cx, cy, rx, ry) =
                            compute_radial_params(&gradient.kind, &logical_corners);
                        // Clip the path against a grid of cells covering the
                        // visible area. Interior grid vertices carry the
                        // gradient's interior colors, while cells clipped to the
                        // path outline preserve the shape's silhouette.
                        let divisions =
                            ((x2 - x1).max(y2 - y1) / 8.0).ceil().clamp(8.0, 64.0) as usize;
                        let step_x = (x2 - x1) / divisions as f32;
                        let step_y = (y2 - y1) / divisions as f32;
                        for gy in 0..divisions {
                            let cell_t = y1 + gy as f32 * step_y;
                            let cell_b = cell_t + step_y;
                            for gx in 0..divisions {
                                let cell_l = x1 + gx as f32 * step_x;
                                let cell_r = cell_l + step_x;
                                for ring in &self.ring_scratch {
                                    let count = clip_ring_to_rect(
                                        ring,
                                        cell_l,
                                        cell_t,
                                        cell_r,
                                        cell_b,
                                        &mut self.clip_in,
                                        &mut self.clip_out,
                                    );
                                    if count == 0 {
                                        continue;
                                    }
                                    emit_polygon_fill_with_colors(
                                        &mut self.mesh.vertices,
                                        &self.clip_out[..count],
                                        &mut self.tri_indices,
                                        self.screen_width,
                                        self.screen_height,
                                        |sx, sy| {
                                            let t = color_at_point(cx, cy, rx, ry, sx, sy);
                                            let mut c = sample_gradient_stops(&gradient.stops, t)
                                                .to_linear_f32_array();
                                            c[3] *= paint.opacity;
                                            c
                                        },
                                    );
                                }
                            }
                        }
                    }
                    GradientKind::Conic { angle, position } => {
                        let (cx, cy) = (
                            min_x + position.0 * (max_x - min_x),
                            min_y + position.1 * (max_y - min_y),
                        );
                        // Like the radial case, sample per grid cell because a
                        // conic gradient is not affine.
                        let divisions =
                            ((x2 - x1).max(y2 - y1) / 8.0).ceil().clamp(8.0, 64.0) as usize;
                        let step_x = (x2 - x1) / divisions as f32;
                        let step_y = (y2 - y1) / divisions as f32;
                        for gy in 0..divisions {
                            let cell_t = y1 + gy as f32 * step_y;
                            let cell_b = cell_t + step_y;
                            for gx in 0..divisions {
                                let cell_l = x1 + gx as f32 * step_x;
                                let cell_r = cell_l + step_x;
                                for ring in &self.ring_scratch {
                                    let count = clip_ring_to_rect(
                                        ring,
                                        cell_l,
                                        cell_t,
                                        cell_r,
                                        cell_b,
                                        &mut self.clip_in,
                                        &mut self.clip_out,
                                    );
                                    if count == 0 {
                                        continue;
                                    }
                                    emit_polygon_fill_with_colors(
                                        &mut self.mesh.vertices,
                                        &self.clip_out[..count],
                                        &mut self.tri_indices,
                                        self.screen_width,
                                        self.screen_height,
                                        |sx, sy| {
                                            // CSS conic angles start at the top
                                            // and increase clockwise (y grows
                                            // downward on screen).
                                            let deg = (sy - cy).atan2(sx - cx).to_degrees();
                                            let t =
                                                ((deg + 90.0 - angle).rem_euclid(360.0)) / 360.0;
                                            let mut c = sample_gradient_stops(&gradient.stops, t)
                                                .to_linear_f32_array();
                                            c[3] *= paint.opacity;
                                            c
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
            }
            Brush::Image(image) => {
                let x1 = min_x.max(clip_l);
                let y1 = min_y.max(clip_t);
                let x2 = max_x.min(clip_r);
                let y2 = max_y.min(clip_b);
                if x2 <= x1 || y2 <= y1 {
                    return;
                }

                let width = max_x - min_x;
                let height = max_y - min_y;
                self.mesh.images.push(ImageSection {
                    image: image.clone(),
                    rect: Rect::new(x1, y1, x2 - x1, y2 - y1),
                    uv: Rect::new(
                        (x1 - min_x) / width,
                        (y1 - min_y) / height,
                        (x2 - x1) / width,
                        (y2 - y1) / height,
                    ),
                    opacity: paint.opacity,
                });
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_text(
        &mut self,
        text: &mut Option<&mut dyn TextLayoutSource>,
        x: f32,
        y: f32,
        text_str: &SmolStr,
        style: &TextStyle,
        flow_style: &TextFlowStyle,
        t: &AffineTransform,
        clip: &ClipRect,
    ) {
        let sf = self.scale_factor;
        let (tdx, tdy) = (t.dx, t.dy);
        let tw = clip.w;
        let th = clip.h;
        let font_size = flow_style.font_size;

        // Line pitch in logical pixels, matching the text renderer's
        // line-height resolution (`text.rs::create_buffer_for_text_inner`).
        let line_height_ratio = match flow_style.line_height {
            LineHeight::Normal => 1.2,
            LineHeight::Number(n) => n.max(0.0),
            LineHeight::Px(px) => px / font_size.max(1e-3),
        };
        let pitch = font_size * line_height_ratio;
        // Margin so glyph ink that spills past the em box (ascenders,
        // descenders) is never culled while still partially visible.
        let line_cull_h = pitch * 1.2;

        let clip_l = clip.x * sf;
        let clip_t = clip.y * sf;
        let clip_r = (clip.x + clip.w) * sf;
        let clip_b = (clip.y + clip.h) * sf;

        // Culling is done per line: split the input text up front so that a
        // mostly off-screen blob only lays out the lines intersecting the
        // clip, and a tall/long text no longer disappears mid-scroll when the
        // whole-block estimate (previously the clip size) would reject it.
        let sx1 = (x + tdx) * sf;
        let Some(tr) = text.as_mut() else {
            return;
        };
        for (line_index, line) in text_str.split('\n').enumerate() {
            let sy1 = (y + line_index as f32 * pitch + tdy) * sf;
            let sy2 = sy1 + line_cull_h * sf;

            if self.enable_text_culling {
                let est_w = (font_size * sf) * (line.len().max(1) as f32) * 0.5;
                if sy2 <= clip_t || sy1 >= clip_b || sx1 + est_w <= clip_l || sx1 >= clip_r {
                    continue;
                }
            }

            if line.is_empty() {
                continue;
            }

            let mut scaled = *flow_style;
            scaled.font_size = ((font_size * sf) * 64.0).round() / 64.0;
            let Some(layout) = tr.layout_text(line, style, scaled) else {
                return;
            };

            self.mesh.sections.push(TextSection {
                screen_position: (sx1, sy1),
                clip_origin: (clip.x * sf, clip.y * sf),
                bounds: (tw * sf, th * sf),
                font_weight: style.font_weight.0,
                layout,
            });
        }
    }
}

/// Compute the clip region for a `PushClip`, intersecting the transformed
/// bounding box of `path` with the parent clip.
fn push_clip(t: &AffineTransform, path: &Path, parent: Option<ClipRect>) -> ClipRect {
    let b = path.bounding_box().unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0));
    let corners = [
        (b.x, b.y),
        (b.x + b.width, b.y),
        (b.x, b.y + b.height),
        (b.x + b.width, b.y + b.height),
    ];
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for (cx, cy) in corners {
        let (tx, ty) = t.apply(cx, cy);
        min_x = min_x.min(tx);
        min_y = min_y.min(ty);
        max_x = max_x.max(tx);
        max_y = max_y.max(ty);
    }
    let new_clip = ClipRect {
        x: min_x,
        y: min_y,
        w: (max_x - min_x).max(0.0),
        h: (max_y - min_y).max(0.0),
    };

    let Some(parent) = parent else {
        return new_clip;
    };
    let x1 = new_clip.x.max(parent.x);
    let y1 = new_clip.y.max(parent.y);
    let x2 = (new_clip.x + new_clip.w).min(parent.x + parent.w);
    let y2 = (new_clip.y + new_clip.h).min(parent.y + parent.h);
    ClipRect {
        x: x1,
        y: y1,
        w: (x2 - x1).max(0.0),
        h: (y2 - y1).max(0.0),
    }
}

/// Sutherland–Hodgman polygon clipping against one axis-aligned edge of the
/// clip rectangle, appending the result to `out`.
///
/// `edge` selects the clip edge in physical (scaled) coordinates:
/// `0` = left, `1` = right, `2` = top, `3` = bottom.
fn clip_against_edge(
    poly: &[(f32, f32)],
    edge: u8,
    clip_l: f32,
    clip_t: f32,
    clip_r: f32,
    clip_b: f32,
    out: &mut Vec<(f32, f32)>,
) {
    if poly.is_empty() {
        return;
    }
    let len = poly.len();
    for i in 0..len {
        let (sx, sy) = poly[i];
        let (ex, ey) = poly[(i + 1) % len];
        let inside = |x: f32, y: f32| -> bool {
            match edge {
                0 => x >= clip_l,
                1 => x <= clip_r,
                2 => y >= clip_t,
                3 => y <= clip_b,
                _ => true,
            }
        };
        let s_in = inside(sx, sy);
        let e_in = inside(ex, ey);

        // Intersection of the segment (s..e) with the selected clip edge.
        let intersect = || -> (f32, f32) {
            match edge {
                0 | 1 => {
                    let x_edge = if edge == 0 { clip_l } else { clip_r };
                    let dx = ex - sx;
                    if dx.abs() < f32::EPSILON {
                        (x_edge, sy)
                    } else {
                        let t = (x_edge - sx) / dx;
                        (x_edge, sy + t * (ey - sy))
                    }
                }
                2 | 3 => {
                    let y_edge = if edge == 2 { clip_t } else { clip_b };
                    let dy = ey - sy;
                    if dy.abs() < f32::EPSILON {
                        (sx, y_edge)
                    } else {
                        let t = (y_edge - sy) / dy;
                        (sx + t * (ex - sx), y_edge)
                    }
                }
                _ => (ex, ey),
            }
        };

        if s_in && e_in {
            out.push((ex, ey));
        } else if s_in && !e_in {
            out.push(intersect());
        } else if !s_in && e_in {
            out.push(intersect());
            out.push((ex, ey));
        }
    }
}

/// Clip `points` (in physical screen coordinates) against an axis-aligned
/// rectangle, ping-ponging between two scratch buffers. The clipped ring ends
/// up in `out`; returns its vertex count (0 if fully clipped away).
fn clip_ring_to_rect(
    points: &[(f32, f32)],
    l: f32,
    t: f32,
    r: f32,
    b: f32,
    scratch: &mut Vec<(f32, f32)>,
    out: &mut Vec<(f32, f32)>,
) -> usize {
    if points.len() < 3 {
        return 0;
    }

    out.clear();
    out.extend_from_slice(points);
    for edge in 0..4u8 {
        scratch.clear();
        clip_against_edge(out, edge, l, t, r, b, scratch);
        std::mem::swap(out, scratch);
        if out.len() < 3 {
            return 0;
        }
    }
    out.len()
}

/// Triangulate `ring` (already clipped to the viewport) with ear clipping and
/// emit its vertices, computing each vertex's color from `color_at` at its
/// physical (scaled) screen position.
fn emit_polygon_fill_with_colors(
    vertices: &mut Vec<Vertex>,
    ring: &[(f32, f32)],
    tri_indices: &mut Vec<[u32; 3]>,
    screen_width: f32,
    screen_height: f32,
    mut color_at: impl FnMut(f32, f32) -> [f32; 4],
) {
    if ring.len() < 3 {
        return;
    }

    tri_indices.clear();
    triangulate(ring, tri_indices);

    let ndc = |v: f32, max: f32| (v / max) * 2.0 - 1.0;
    vertices.reserve(tri_indices.len() * 3);
    for &[i0, i1, i2] in tri_indices.iter() {
        let p0 = ring[i0 as usize];
        let p1 = ring[i1 as usize];
        let p2 = ring[i2 as usize];
        vertices.push(Vertex {
            position: [ndc(p0.0, screen_width), -ndc(p0.1, screen_height), 0.0],
            color: color_at(p0.0, p0.1),
        });
        vertices.push(Vertex {
            position: [ndc(p1.0, screen_width), -ndc(p1.1, screen_height), 0.0],
            color: color_at(p1.0, p1.1),
        });
        vertices.push(Vertex {
            position: [ndc(p2.0, screen_width), -ndc(p2.1, screen_height), 0.0],
            color: color_at(p2.0, p2.1),
        });
    }
}

// --------------------------------
// Ear-clipping triangulation
// --------------------------------

fn cross(a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> f32 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

/// Signed area of a closed ring. Positive winding is counter-clockwise in a
/// y-down coordinate system.
fn polygon_area(poly: &[(f32, f32)]) -> f32 {
    let mut area = 0.0;
    for i in 0..poly.len() {
        let (x0, y0) = poly[i];
        let (x1, y1) = poly[(i + 1) % poly.len()];
        area += x0 * y1 - x1 * y0;
    }
    area * 0.5
}

/// Cross-product sign of a point relative to an oriented edge.
fn edge_sign(p1: (f32, f32), p2: (f32, f32), p3: (f32, f32)) -> f32 {
    (p1.0 - p3.0) * (p2.1 - p3.1) - (p2.0 - p3.0) * (p1.1 - p3.1)
}

fn point_in_triangle(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let d1 = edge_sign(p, a, b);
    let d2 = edge_sign(p, b, c);
    let d3 = edge_sign(p, c, a);
    let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_neg && has_pos)
}

/// Returns `true` if the vertex `i1` of `poly` is an ear tip: strictly convex
/// and no other vertex lies inside the triangle `(i0, i1, i2)`.
fn is_ear(poly: &[(f32, f32)], i0: u32, i1: u32, i2: u32, area_sign: f32) -> bool {
    let a = poly[i0 as usize];
    let b = poly[i1 as usize];
    let c = poly[i2 as usize];

    let cr = cross(a, b, c);
    if cr == 0.0 {
        return false;
    }
    // Normalize winding so that "convex" means the same sign as the ring.
    if area_sign >= 0.0 {
        if cr < 0.0 {
            return false;
        }
    } else if cr > 0.0 {
        return false;
    }

    for (idx, p) in poly.iter().enumerate() {
        if idx == i0 as usize || idx == i1 as usize || idx == i2 as usize {
            continue;
        }
        if point_in_triangle(*p, a, b, c) {
            return false;
        }
    }
    true
}

/// Triangulate a simple (possibly concave, not self-intersecting) ring into a
/// triangle list by iterative ear clipping. Degenerate input falls back to a
/// fan so that callers always make progress.
///
/// Emitted triangle indices always refer to the *top-level* `poly`, because
/// clipping works on a ring of absolute indices rather than on copied slices.
fn triangulate(poly: &[(f32, f32)], out: &mut Vec<[u32; 3]>) {
    let n = poly.len();
    if n < 3 {
        return;
    }

    let area_sign = polygon_area(poly);
    let mut ring: Vec<u32> = (0..n as u32).collect();
    let mut count = n;

    let mut guard = 0;
    while count > 3 {
        guard += 1;
        if guard > n * n {
            break;
        }
        let mut clipped = false;
        let mut i = 0;
        while i < count {
            let ip = ring[(i + count - 1) % count];
            let cur = ring[i];
            let ni = ring[(i + 1) % count];
            if is_ear(poly, ip, cur, ni, area_sign) {
                out.push([ip, cur, ni]);
                ring.remove(i);
                count -= 1;
                clipped = true;
                break;
            }
            i += 1;
        }
        if !clipped {
            break;
        }
    }

    if count == 3 {
        out.push([ring[0], ring[1], ring[2]]);
    } else {
        // No ear found (degenerate / collinear input): fall back to a fan
        // over whatever ring remains.
        for i in 1..(count - 1) {
            out.push([ring[0], ring[i], ring[i + 1]]);
        }
    }
}

// --------------------------------
// Gradients
// --------------------------------

/// Projection parameters of a linear gradient over the given (unclipped)
/// corners: the gradient direction `(dir_x, dir_y)` and the min/max projection
/// onto it. Returns zeros for non-linear gradients.
fn linear_gradient_extent(gradient: &Gradient, corners: &[(f32, f32); 4]) -> (f32, f32, f32, f32) {
    let GradientKind::Linear { angle } = &gradient.kind else {
        return (0.0, 0.0, 0.0, 0.0);
    };
    let rad = angle.to_radians();
    let dir_x = rad.sin();
    let dir_y = -rad.cos();
    let mut min_p = f32::INFINITY;
    let mut max_p = f32::NEG_INFINITY;
    for (cx, cy) in corners {
        let p = cx * dir_x + cy * dir_y;
        min_p = min_p.min(p);
        max_p = max_p.max(p);
    }
    (dir_x, dir_y, min_p, max_p)
}

/// Compute the 4 corner colors for a linear gradient rectangle.
///
/// `extent_corners` define the full gradient extent (min/max projection).
/// `sample_corners` are the actual corners to compute colors for (usually the clipped rect).
/// corners layout: [TL, BL, TR, BR] in physical (pre-NDC) screen coordinates.
#[cfg(test)]
fn compute_gradient_corner_colors_extent(
    gradient: &Gradient,
    extent_corners: &[(f32, f32); 4],
    sample_corners: &[(f32, f32); 4],
) -> [Color; 4] {
    let GradientKind::Linear { angle } = &gradient.kind else {
        return [Color(0, 0, 0, 0); 4];
    };
    let rad = angle.to_radians();
    let dir_x = rad.sin();
    let dir_y = -rad.cos();

    // Compute gradient extent from the full (unclipped) rectangle
    let extent_projs: Vec<f32> = extent_corners
        .iter()
        .map(|(cx, cy)| cx * dir_x + cy * dir_y)
        .collect();
    let min_p = extent_projs.iter().cloned().fold(f32::INFINITY, f32::min);
    let max_p = extent_projs
        .iter()
        .cloned()
        .fold(f32::NEG_INFINITY, f32::max);
    let range = max_p - min_p;

    // Sample the gradient at each of the visible (clipped) corners
    let mut colors = [Color(0, 0, 0, 0); 4];
    for (i, (cx, cy)) in sample_corners.iter().enumerate() {
        let p = cx * dir_x + cy * dir_y;
        let t = if range > 0.0 {
            (p - min_p) / range
        } else {
            0.0
        };
        colors[i] = sample_gradient_stops(&gradient.stops, t);
    }
    colors
}

/// Returns the required ellipse radius `rx` (in the x direction) such that
/// an ellipse centered with aspect ratio `w/h` passes through a point at
/// offset `(dx, dy)` from its center.
///
/// The derivation:
///   (dx/rx)² + (dy/ry)² = 1   and   ry = rx * h/w
///   ⇒  rx² = dx² + dy² * w²/h²
fn ellipse_rx_for_corner(dx: f32, dy: f32, w: f32, h: f32) -> f32 {
    if h <= 0.0 || w <= 0.0 {
        return dx;
    }
    (dx * dx + dy * dy * (w / h) * (w / h)).sqrt()
}

fn compute_radial_params(kind: &GradientKind, corners: &[(f32, f32); 4]) -> (f32, f32, f32, f32) {
    let GradientKind::Radial {
        shape,
        size,
        position,
    } = kind
    else {
        return (0.0, 0.0, 1.0, 1.0);
    };

    let min_x = corners.iter().map(|c| c.0).fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|c| c.0)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_y = corners.iter().map(|c| c.1).fold(f32::INFINITY, f32::min);
    let max_y = corners
        .iter()
        .map(|c| c.1)
        .fold(f32::NEG_INFINITY, f32::max);
    let w = max_x - min_x;
    let h = max_y - min_y;
    let cx = min_x + w * position.0;
    let cy = min_y + h * position.1;

    log::trace!(
        "compute_radial_params: corners=[({:.1},{:.1}),({:.1},{:.1}),({:.1},{:.1}),({:.1},{:.1})] \
         box=({:.1},{:.1}) center=({:.1},{:.1}) shape={:?} size={:?}",
        corners[0].0,
        corners[0].1,
        corners[1].0,
        corners[1].1,
        corners[2].0,
        corners[2].1,
        corners[3].0,
        corners[3].1,
        w,
        h,
        cx,
        cy,
        shape,
        size,
    );

    let (rx, ry) = match (shape, size) {
        (RadialShape::Circle, RadialSizeKind::ClosestSide) => {
            let d = (cx - min_x)
                .min(max_x - cx)
                .min((cy - min_y).min(max_y - cy));
            (d, d)
        }
        (RadialShape::Circle, RadialSizeKind::FarthestSide) => {
            let d = (cx - min_x)
                .max(max_x - cx)
                .max((cy - min_y).max(max_y - cy));
            (d, d)
        }
        (RadialShape::Circle, RadialSizeKind::ClosestCorner) => {
            let d = corners
                .iter()
                .map(|(px, py)| ((px - cx).powi(2) + (py - cy).powi(2)).sqrt())
                .fold(f32::INFINITY, f32::min);
            (d, d)
        }
        (RadialShape::Circle, RadialSizeKind::FarthestCorner) => {
            let d = corners
                .iter()
                .map(|(px, py)| ((px - cx).powi(2) + (py - cy).powi(2)).sqrt())
                .fold(0.0f32, f32::max);
            (d, d)
        }
        (RadialShape::Ellipse, RadialSizeKind::ClosestSide) => {
            let rx = (cx - min_x).min(max_x - cx);
            let ry = (cy - min_y).min(max_y - cy);
            (rx, ry)
        }
        (RadialShape::Ellipse, RadialSizeKind::FarthestSide) => {
            let rx = (cx - min_x).max(max_x - cx);
            let ry = (cy - min_y).max(max_y - cy);
            (rx, ry)
        }
        (RadialShape::Ellipse, RadialSizeKind::ClosestCorner) => {
            let mut best_rx = f32::INFINITY;
            let mut best_ry = f32::INFINITY;
            for (px, py) in corners.iter() {
                let dx = (px - cx).abs();
                let dy = (py - cy).abs();
                let rx = ellipse_rx_for_corner(dx, dy, w, h);
                let ry = rx * h / w;
                if rx < best_rx {
                    best_rx = rx;
                    best_ry = ry;
                }
            }
            (best_rx, best_ry)
        }
        (RadialShape::Ellipse, RadialSizeKind::FarthestCorner) => {
            let mut best_rx = 0.0f32;
            let mut best_ry = 0.0f32;
            for (px, py) in corners.iter() {
                let dx = (px - cx).abs();
                let dy = (py - cy).abs();
                let rx = ellipse_rx_for_corner(dx, dy, w, h);
                let ry = rx * h / w;
                if rx > best_rx {
                    best_rx = rx;
                    best_ry = ry;
                }
            }
            (best_rx, best_ry)
        }
        (_, RadialSizeKind::Explicit { rx, ry }) => (rx * w, ry * h),
    };

    let rx = rx.max(0.001);
    let ry = ry.max(0.001);

    log::trace!(
        "compute_radial_params: result cx={:.1} cy={:.1} rx={:.1} ry={:.1}",
        cx,
        cy,
        rx,
        ry,
    );

    (cx, cy, rx, ry)
}

fn color_at_point(cx: f32, cy: f32, rx: f32, ry: f32, px: f32, py: f32) -> f32 {
    let dx = px - cx;
    let dy = py - cy;
    ((dx / rx).powi(2) + (dy / ry).powi(2))
        .sqrt()
        .clamp(0.0, 1.0)
}

/// Sample a gradient's color stops at normalized position `t` (0.0–1.0).
fn sample_gradient_stops(stops: &[ColorStop], t: f32) -> Color {
    if stops.is_empty() {
        return Color(0, 0, 0, 0);
    }
    if stops.len() == 1 {
        return stops[0].color;
    }

    let t = t.clamp(0.0, 1.0);
    let last = stops.len() - 1;

    for i in 0..last {
        let pos_i = stops[i]
            .position
            .unwrap_or(if i == 0 { 0.0 } else { i as f32 / last as f32 });
        let pos_j = stops[i + 1].position.unwrap_or(if i + 1 == last {
            1.0
        } else {
            (i + 1) as f32 / last as f32
        });

        if t >= pos_i && t <= pos_j {
            let local = if pos_j > pos_i {
                (t - pos_i) / (pos_j - pos_i)
            } else {
                0.0
            };
            return lerp_color(stops[i].color, stops[i + 1].color, local);
        }
    }

    if t <= stops[0].position.unwrap_or(0.0) {
        return stops[0].color;
    }
    stops[last].color
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    // Interpolate in linear RGB space for correct color mixing
    let al = a.to_linear_f32_array();
    let bl = b.to_linear_f32_array();
    Color::from_linear_f32_array([
        al[0] + (bl[0] - al[0]) * t,
        al[1] + (bl[1] - al[1]) * t,
        al[2] + (bl[2] - al[2]) * t,
        al[3] + (bl[3] - al[3]) * t,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::renderer_model::{ellipse_path, rect_path, rounded_rect_path};

    // A text source stub that never lays out text (geometry-only tests).
    struct NoText;

    impl TextLayoutSource for NoText {
        fn layout_text(
            &mut self,
            _text: &str,
            _style: &TextStyle,
            _flow_style: TextFlowStyle,
        ) -> Option<Arc<TextLayout>> {
            None
        }
    }

    // ── triangulate ───────────────────────────────────────────────────

    fn area_sum(vertices: &[Vertex]) -> f32 {
        // Reconstruct screen-space triangles and sum their signed areas.
        let mut sum = 0.0f32;
        for tri in vertices.chunks_exact(3) {
            let p0 = (
                (tri[0].position[0] + 1.0) / 2.0,
                -(tri[0].position[1] - 1.0) / 2.0,
            );
            let p1 = (
                (tri[1].position[0] + 1.0) / 2.0,
                -(tri[1].position[1] - 1.0) / 2.0,
            );
            let p2 = (
                (tri[2].position[0] + 1.0) / 2.0,
                -(tri[2].position[1] - 1.0) / 2.0,
            );
            sum += (p1.0 - p0.0) * (p2.1 - p0.1) - (p2.0 - p0.0) * (p1.1 - p0.1);
        }
        sum.abs() * 0.5
    }

    #[test]
    fn test_triangulate_square() {
        let poly = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let mut out = Vec::new();
        triangulate(&poly, &mut out);
        assert_eq!(out.len(), 2);
        // The two triangles must cover exactly the square.
        let covered = area_sum_tri(&poly, &out);
        assert!((covered - 100.0).abs() < 1e-3, "covered={covered}");
    }

    fn area_sum_tri(poly: &[(f32, f32)], tris: &[[u32; 3]]) -> f32 {
        let mut sum = 0.0;
        for &[i0, i1, i2] in tris {
            let a = poly[i0 as usize];
            let b = poly[i1 as usize];
            let c = poly[i2 as usize];
            sum += (b.0 - a.0) * (c.1 - a.1) - (c.0 - a.0) * (b.1 - a.1);
        }
        sum.abs() * 0.5
    }

    #[test]
    fn test_triangulate_concave() {
        // Concave "arrowhead" shape; a fan from v0 would spill outside.
        let poly = [
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (5.0, 5.0),
            (0.0, 10.0),
        ];
        let mut out = Vec::new();
        triangulate(&poly, &mut out);
        assert!(!out.is_empty());
        let poly_area = polygon_area(&poly).abs();
        let covered = area_sum_tri(&poly, &out);
        assert!(
            (covered - poly_area).abs() < 1e-3,
            "covered={covered} expected={poly_area}"
        );
    }

    // ── clip_against_edge / clip_ring_to_rect / emit_polygon_fill_with_colors ──

    #[test]
    fn test_clip_against_edge_left_keeps_points_inside() {
        let poly = [(10.0, 0.0), (20.0, 0.0), (20.0, 10.0), (10.0, 10.0)];
        let mut clipped = Vec::new();
        clip_against_edge(&poly, 0, 5.0, 0.0, 100.0, 100.0, &mut clipped);
        assert_eq!(clipped.len(), poly.len());
        for (x, _) in &clipped {
            assert!(*x >= 5.0);
        }
    }

    #[test]
    fn test_emit_polygon_fill_with_colors_fully_inside() {
        let mut vertices = Vec::new();
        let points = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
        let mut clip_in = Vec::new();
        let mut clip_out = Vec::new();
        let mut tris = Vec::new();
        let count = clip_ring_to_rect(&points, 0.0, 0.0, 200.0, 200.0, &mut clip_in, &mut clip_out);
        assert_eq!(count, 4);
        emit_polygon_fill_with_colors(
            &mut vertices,
            &clip_out[..count],
            &mut tris,
            200.0,
            200.0,
            |_, _| [1.0; 4],
        );
        assert_eq!(vertices.len(), 6);
    }

    #[test]
    fn test_emit_polygon_fill_with_colors_fully_outside() {
        let mut vertices = Vec::new();
        let points = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let mut clip_in = Vec::new();
        let mut clip_out = Vec::new();
        let mut tris = Vec::new();
        let count = clip_ring_to_rect(&points, 20.0, 20.0, 30.0, 30.0, &mut clip_in, &mut clip_out);
        assert_eq!(count, 0);
        emit_polygon_fill_with_colors(&mut vertices, &clip_out, &mut tris, 200.0, 200.0, |_, _| {
            [1.0; 4]
        });
        assert!(vertices.is_empty());
    }

    #[test]
    fn test_emit_polygon_fill_with_colors_partially_inside() {
        let mut vertices = Vec::new();
        let points = [(0.0, 0.0), (50.0, 0.0), (50.0, 50.0), (0.0, 50.0)];
        let mut clip_in = Vec::new();
        let mut clip_out = Vec::new();
        let mut tris = Vec::new();
        let count = clip_ring_to_rect(&points, 10.0, 10.0, 40.0, 40.0, &mut clip_in, &mut clip_out);
        assert!(count >= 4);
        emit_polygon_fill_with_colors(
            &mut vertices,
            &clip_out[..count],
            &mut tris,
            100.0,
            100.0,
            |_, _| [1.0; 4],
        );
        assert!(!vertices.is_empty());
        assert_eq!(vertices.len() % 3, 0);
        for v in &vertices {
            let sx = (v.position[0] + 1.0) / 2.0 * 100.0;
            let sy = -(v.position[1] - 1.0) / 2.0 * 100.0;
            assert!((10.0 - 1e-3..=40.0 + 1e-3).contains(&sx), "sx={sx}");
            assert!((10.0 - 1e-3..=40.0 + 1e-3).contains(&sy), "sy={sy}");
        }
    }

    // ── ellipse_rx_for_corner ─────────────────────────────────────────

    #[test]
    fn test_ellipse_rx_for_corner_centered() {
        let rx = ellipse_rx_for_corner(100.0, 50.0, 200.0, 100.0);
        let expected = (20000.0f32).sqrt();
        assert!(
            (rx - expected).abs() < 0.01,
            "rx={} expected={}",
            rx,
            expected
        );
    }

    #[test]
    fn test_ellipse_rx_for_corner_square() {
        let rx = ellipse_rx_for_corner(100.0, 100.0, 100.0, 100.0);
        let expected = (20000.0f32).sqrt();
        assert!(
            (rx - expected).abs() < 0.01,
            "rx={} expected={}",
            rx,
            expected
        );
    }

    #[test]
    fn test_ellipse_rx_for_corner_zero_box() {
        assert_eq!(ellipse_rx_for_corner(50.0, 30.0, 100.0, 0.0), 50.0);
        assert_eq!(ellipse_rx_for_corner(50.0, 30.0, 0.0, 100.0), 50.0);
    }

    // ── color_at_point ────────────────────────────────────────────────

    #[test]
    fn test_color_at_point_center() {
        let d = color_at_point(100.0, 100.0, 50.0, 50.0, 100.0, 100.0);
        assert!((d - 0.0).abs() < 1e-6, "d={}", d);
    }

    #[test]
    fn test_color_at_point_on_edge() {
        let d = color_at_point(100.0, 100.0, 50.0, 50.0, 150.0, 100.0);
        assert!((d - 1.0).abs() < 1e-6, "d={}", d);
    }

    #[test]
    fn test_color_at_point_outside() {
        let d = color_at_point(100.0, 100.0, 50.0, 50.0, 200.0, 200.0);
        assert!((d - 1.0).abs() < 1e-6, "d={}", d);
    }

    #[test]
    fn test_color_at_point_ellipse() {
        let d = color_at_point(0.0, 0.0, 100.0, 50.0, 50.0, 25.0);
        assert!((d - (0.5f32).sqrt()).abs() < 1e-6, "d={}", d);
    }

    // ── compute_radial_params ─────────────────────────────────────────

    fn make_radial(shape: RadialShape, size: RadialSizeKind, position: (f32, f32)) -> GradientKind {
        GradientKind::Radial {
            shape,
            size,
            position,
        }
    }

    fn corners_from_rect(x: f32, y: f32, w: f32, h: f32) -> [(f32, f32); 4] {
        [(x, y), (x, y + h), (x + w, y), (x + w, y + h)]
    }

    #[test]
    fn test_radial_circle_closest_side_centered() {
        let kind = make_radial(RadialShape::Circle, RadialSizeKind::ClosestSide, (0.5, 0.5));
        let (_cx, _cy, rx, ry) =
            compute_radial_params(&kind, &corners_from_rect(0.0, 0.0, 200.0, 100.0));
        assert!((rx - 50.0).abs() < 0.01, "rx={}", rx);
        assert!((ry - 50.0).abs() < 0.01, "ry={}", ry);
    }

    #[test]
    fn test_radial_circle_farthest_corner_centered() {
        let kind = make_radial(
            RadialShape::Circle,
            RadialSizeKind::FarthestCorner,
            (0.5, 0.5),
        );
        let (_cx, _cy, rx, ry) =
            compute_radial_params(&kind, &corners_from_rect(0.0, 0.0, 200.0, 100.0));
        let expected = (100.0f32 * 100.0 + 50.0 * 50.0).sqrt();
        assert!(
            (rx - expected).abs() < 0.01,
            "rx={} expected={}",
            rx,
            expected
        );
        assert!((ry - expected).abs() < 0.01, "ry={}", ry);
    }

    #[test]
    fn test_radial_ellipse_closest_side_centered() {
        let kind = make_radial(
            RadialShape::Ellipse,
            RadialSizeKind::ClosestSide,
            (0.5, 0.5),
        );
        let (_cx, _cy, rx, ry) =
            compute_radial_params(&kind, &corners_from_rect(0.0, 0.0, 200.0, 100.0));
        assert!((rx - 100.0).abs() < 0.01, "rx={}", rx);
        assert!((ry - 50.0).abs() < 0.01, "ry={}", ry);
    }

    #[test]
    fn test_radial_ellipse_farthest_side_centered() {
        let kind = make_radial(
            RadialShape::Ellipse,
            RadialSizeKind::FarthestSide,
            (0.5, 0.5),
        );
        let (_cx, _cy, rx, ry) =
            compute_radial_params(&kind, &corners_from_rect(0.0, 0.0, 200.0, 100.0));
        assert!((rx - 100.0).abs() < 0.01, "rx={}", rx);
        assert!((ry - 50.0).abs() < 0.01, "ry={}", ry);
    }

    #[test]
    fn test_radial_ellipse_closest_corner_centered() {
        let kind = make_radial(
            RadialShape::Ellipse,
            RadialSizeKind::ClosestCorner,
            (0.5, 0.5),
        );
        let (_cx, _cy, rx, ry) =
            compute_radial_params(&kind, &corners_from_rect(0.0, 0.0, 200.0, 100.0));
        let expected_rx = (20000.0f32).sqrt();
        let expected_ry = expected_rx * 100.0 / 200.0;
        assert!(
            (rx - expected_rx).abs() < 0.01,
            "rx={} expected={}",
            rx,
            expected_rx
        );
        assert!(
            (ry - expected_ry).abs() < 0.01,
            "ry={} expected={}",
            ry,
            expected_ry
        );
    }

    #[test]
    fn test_radial_ellipse_farthest_corner_centered() {
        let kind = make_radial(
            RadialShape::Ellipse,
            RadialSizeKind::FarthestCorner,
            (0.5, 0.5),
        );
        let (_cx, _cy, rx, ry) =
            compute_radial_params(&kind, &corners_from_rect(0.0, 0.0, 200.0, 100.0));
        let expected_rx = (20000.0f32).sqrt();
        let expected_ry = expected_rx * 100.0 / 200.0;
        assert!(
            (rx - expected_rx).abs() < 0.01,
            "rx={} expected={}",
            rx,
            expected_rx
        );
        assert!(
            (ry - expected_ry).abs() < 0.01,
            "ry={} expected={}",
            ry,
            expected_ry
        );
    }

    #[test]
    fn test_radial_ellipse_closest_corner_offset() {
        let kind = make_radial(
            RadialShape::Ellipse,
            RadialSizeKind::ClosestCorner,
            (0.2, 0.3),
        );
        let (_cx, _cy, rx, ry) =
            compute_radial_params(&kind, &corners_from_rect(0.0, 0.0, 200.0, 100.0));
        let expected_rx = (5200.0f32).sqrt();
        let expected_ry = expected_rx * 100.0 / 200.0;
        assert!(
            (rx - expected_rx).abs() < 0.1,
            "rx={} expected={}",
            rx,
            expected_rx
        );
        assert!(
            (ry - expected_ry).abs() < 0.1,
            "ry={} expected={}",
            ry,
            expected_ry
        );
    }

    #[test]
    fn test_radial_ellipse_farthest_corner_offset() {
        let kind = make_radial(
            RadialShape::Ellipse,
            RadialSizeKind::FarthestCorner,
            (0.2, 0.3),
        );
        let (_cx, _cy, rx, ry) =
            compute_radial_params(&kind, &corners_from_rect(0.0, 0.0, 200.0, 100.0));
        let expected_rx = (45200.0f32).sqrt();
        let expected_ry = expected_rx * 100.0 / 200.0;
        assert!(
            (rx - expected_rx).abs() < 0.1,
            "rx={} expected={}",
            rx,
            expected_rx
        );
        assert!(
            (ry - expected_ry).abs() < 0.1,
            "ry={} expected={}",
            ry,
            expected_ry
        );
    }

    #[test]
    fn test_radial_ellipse_explicit_percentage_resolves_against_box_axes() {
        // 30% of width (200) and 50% of height (100) => rx=60, ry=50.
        let kind = make_radial(
            RadialShape::Ellipse,
            RadialSizeKind::Explicit { rx: 0.3, ry: 0.5 },
            (0.5, 0.5),
        );
        let (_cx, _cy, rx, ry) =
            compute_radial_params(&kind, &corners_from_rect(0.0, 0.0, 200.0, 100.0));
        assert!((rx - 60.0).abs() < 0.01, "rx={}", rx);
        assert!((ry - 50.0).abs() < 0.01, "ry={}", ry);
    }

    // ── sample_gradient_stops / lerp_color ────────────────────────────

    #[test]
    fn test_lerp_color_srgb_vs_linear() {
        let red = Color(255, 0, 0, 255);
        let blue = Color(0, 0, 255, 255);
        let mixed = lerp_color(red, blue, 0.5);
        assert!(
            mixed.0 > 128,
            "R should be >128 in linear lerp, got {}",
            mixed.0
        );
        assert!(
            mixed.2 > 128,
            "B should be >128 in linear lerp, got {}",
            mixed.2
        );
    }

    #[test]
    fn test_sample_gradient_single_stop() {
        let stops = vec![ColorStop {
            color: Color(255, 0, 0, 255),
            position: None,
        }];
        let c = sample_gradient_stops(&stops, 0.5);
        assert_eq!(c, Color(255, 0, 0, 255));
    }

    #[test]
    fn test_sample_gradient_two_stops() {
        let stops = vec![
            ColorStop {
                color: Color(255, 0, 0, 255),
                position: None,
            },
            ColorStop {
                color: Color(0, 0, 255, 255),
                position: None,
            },
        ];
        let c0 = sample_gradient_stops(&stops, 0.0);
        assert_eq!(c0, Color(255, 0, 0, 255));
        let c1 = sample_gradient_stops(&stops, 1.0);
        assert_eq!(c1, Color(0, 0, 255, 255));
        let mid = sample_gradient_stops(&stops, 0.5);
        assert!(mid.0 > 0 && mid.0 < 255);
        assert!(mid.2 > 0 && mid.2 < 255);
    }

    // ── compute_gradient_corner_colors_extent ─────────────────────────

    #[test]
    fn test_linear_gradient_extent_clipped() {
        let gradient = Gradient {
            kind: GradientKind::Linear { angle: 90.0 },
            stops: vec![
                ColorStop {
                    color: Color(255, 0, 0, 255),
                    position: None,
                },
                ColorStop {
                    color: Color(0, 0, 255, 255),
                    position: None,
                },
            ],
        };
        let full = [(0.0, 0.0), (0.0, 100.0), (200.0, 0.0), (200.0, 100.0)];
        let visible = [(120.0, 0.0), (120.0, 100.0), (200.0, 0.0), (200.0, 100.0)];

        let colors = compute_gradient_corner_colors_extent(&gradient, &full, &visible);
        for (i, c) in colors.iter().enumerate() {
            assert!(
                c.2 > c.0,
                "Corner {} should be more blue (got r={}, b={})",
                i,
                c.0,
                c.2
            );
        }
        assert!(colors[0].0 > colors[2].0, "TL should have more red than TR");
    }

    #[test]
    fn test_linear_gradient_extent_unclipped() {
        let gradient = Gradient {
            kind: GradientKind::Linear { angle: 90.0 },
            stops: vec![
                ColorStop {
                    color: Color(255, 0, 0, 255),
                    position: None,
                },
                ColorStop {
                    color: Color(0, 0, 255, 255),
                    position: None,
                },
            ],
        };
        let corners = [(0.0, 0.0), (0.0, 100.0), (200.0, 0.0), (200.0, 100.0)];
        let colors = compute_gradient_corner_colors_extent(&gradient, &corners, &corners);
        assert_eq!(colors[0], Color(255, 0, 0, 255));
        assert_eq!(colors[1], Color(255, 0, 0, 255));
        assert_eq!(colors[2], Color(0, 0, 255, 255));
        assert_eq!(colors[3], Color(0, 0, 255, 255));
    }

    // ── MeshBuilder end-to-end ────────────────────────────────────────

    fn solid_fill(path: Path, color: Color) -> DrawCommand {
        DrawCommand::Fill {
            path,
            paint: Paint {
                brush: Brush::Solid(color),
                opacity: 1.0,
            },
            rule: crate::engine::renderer_model::FillRule::NonZero,
        }
    }

    fn gradient_fill(path: Path, gradient: Gradient) -> DrawCommand {
        DrawCommand::Fill {
            path,
            paint: Paint {
                brush: Brush::Gradient(gradient),
                opacity: 1.0,
            },
            rule: crate::engine::renderer_model::FillRule::NonZero,
        }
    }

    #[test]
    fn test_image_fill_preserves_uv_when_clipped() {
        let mut builder = MeshBuilder::new(100.0, 100.0, 1.0);
        let mut text: Option<&mut dyn TextLayoutSource> = Some(&mut NoText);
        let image = Image::from_rgba(2, 2, vec![255; 16]).unwrap();
        let commands = [
            DrawCommand::PushClip {
                path: rect_path(25.0, 25.0, 50.0, 50.0),
                rule: crate::engine::renderer_model::FillRule::NonZero,
            },
            DrawCommand::Fill {
                path: rect_path(0.0, 0.0, 100.0, 100.0),
                paint: Paint {
                    brush: Brush::Image(image),
                    opacity: 0.5,
                },
                rule: crate::engine::renderer_model::FillRule::NonZero,
            },
            DrawCommand::PopClip,
        ];

        let mesh = builder.build(&commands, &mut text);
        assert_eq!(mesh.images.len(), 1);
        let section = &mesh.images[0];
        assert_eq!(section.rect.x, 25.0);
        assert_eq!(section.rect.y, 25.0);
        assert_eq!(section.rect.width, 50.0);
        assert_eq!(section.rect.height, 50.0);
        assert_eq!(section.uv.x, 0.25);
        assert_eq!(section.uv.y, 0.25);
        assert_eq!(section.uv.width, 0.5);
        assert_eq!(section.uv.height, 0.5);
        assert_eq!(section.opacity, 0.5);
    }

    #[test]
    fn test_draw_items_preserve_command_order() {
        // Regression: the GPU used to split geometry into shape/image/text
        // passes, always compositing text on top. `draw_items` must record the
        // command order so a single pass can respect interleaving.
        let mut builder = MeshBuilder::new(100.0, 100.0, 1.0);
        let mut text: Option<&mut dyn TextLayoutSource> = Some(&mut NoText);
        let image = Image::from_rgba(2, 2, vec![255; 16]).unwrap();
        let commands = [
            solid_fill(rect_path(0.0, 0.0, 50.0, 50.0), Color(255, 0, 0, 255)),
            solid_fill(rect_path(0.0, 0.0, 25.0, 25.0), Color(0, 255, 0, 255)),
            DrawCommand::Fill {
                path: rect_path(0.0, 0.0, 100.0, 100.0),
                paint: Paint {
                    brush: Brush::Image(image),
                    opacity: 1.0,
                },
                rule: crate::engine::renderer_model::FillRule::NonZero,
            },
            DrawCommand::DrawText {
                x: 0.0,
                y: 0.0,
                text: "hello".into(),
                style: TextStyle::default(),
                flow_style: TextFlowStyle {
                    font_size: 12.0,
                    ..TextFlowStyle::default()
                },
            },
        ];

        let mesh = builder.build(&commands, &mut text);

        // NoText never lays out text, so the DrawText command contributes no
        // section and must not appear in the items.
        assert_eq!(mesh.sections.len(), 0);
        assert_eq!(mesh.images.len(), 1);
        assert_eq!(mesh.draw_items.len(), 3);

        let mut vertices_seen = 0u32;
        match mesh.draw_items[0] {
            DrawItem::Fill {
                vertex_start,
                vertex_count,
            } => {
                assert_eq!(vertex_start, 0);
                vertices_seen += vertex_count;
            }
            ref other => panic!("expected Fill, got {other:?}"),
        }
        match mesh.draw_items[1] {
            DrawItem::Fill {
                vertex_start,
                vertex_count,
            } => {
                assert_eq!(vertex_start, vertices_seen);
                vertices_seen += vertex_count;
            }
            ref other => panic!("expected Fill, got {other:?}"),
        }
        assert_eq!(mesh.draw_items[2], DrawItem::Image(0));
        assert_eq!(vertices_seen as usize, mesh.vertices.len());

        // The same geometry must be covered exactly once: the item vertex
        // ranges partition the vertex buffer.
        let covered: usize = mesh
            .draw_items
            .iter()
            .filter_map(|item| match item {
                DrawItem::Fill { vertex_count, .. } => Some(*vertex_count as usize),
                _ => None,
            })
            .sum();
        assert_eq!(covered, mesh.vertices.len());
    }

    #[test]
    fn test_gradient_fill_respects_circle_shape() {
        // Regression: gradient fills used to rasterize the path's bounding
        // box, turning a circle into a square. No emitted vertex may lie
        // outside the circle.
        let mut builder = MeshBuilder::new(800.0, 600.0, 1.0);
        let mut text: Option<&mut dyn TextLayoutSource> = Some(&mut NoText);
        let gradient = Gradient {
            kind: GradientKind::Radial {
                shape: RadialShape::Circle,
                size: RadialSizeKind::FarthestCorner,
                position: (0.5, 0.5),
            },
            stops: vec![
                ColorStop {
                    color: Color(255, 255, 0, 255),
                    position: Some(0.0),
                },
                ColorStop {
                    color: Color(0, 255, 0, 255),
                    position: Some(1.0),
                },
            ],
        };
        builder.build(
            &[gradient_fill(
                ellipse_path(200.0, 200.0, 100.0, 100.0),
                gradient,
            )],
            &mut text,
        );
        assert!(!builder.mesh.vertices.is_empty());
        // The cubic Bézier circle approximation deviates from the ideal circle
        // by a fraction of a pixel, so allow a small margin. The old
        // bounding-box behavior would emit corners at radius ~141px, far
        // beyond this tolerance.
        let max_r = 100.0 + 2.0;
        for v in builder.mesh.vertices {
            let sx = (v.position[0] + 1.0) / 2.0 * 800.0;
            let sy = -(v.position[1] - 1.0) / 2.0 * 600.0;
            let d2 = (sx - 200.0).powi(2) + (sy - 200.0).powi(2);
            assert!(d2 <= max_r * max_r, "vertex ({sx},{sy}) outside circle");
        }
    }

    #[test]
    fn test_gradient_fill_respects_rounded_rect() {
        // Regression: a linear gradient on a rounded square used to render the
        // full bounding box, filling the rounded corner cut-outs.
        let mut builder = MeshBuilder::new(800.0, 600.0, 1.0);
        let mut text: Option<&mut dyn TextLayoutSource> = Some(&mut NoText);
        let gradient = Gradient {
            kind: GradientKind::Linear { angle: 90.0 },
            stops: vec![
                ColorStop {
                    color: Color(255, 0, 0, 255),
                    position: Some(0.0),
                },
                ColorStop {
                    color: Color(0, 0, 255, 255),
                    position: Some(1.0),
                },
            ],
        };
        builder.build(
            &[gradient_fill(
                rounded_rect_path(
                    100.0,
                    100.0,
                    200.0,
                    200.0,
                    (50.0, 50.0),
                    (50.0, 50.0),
                    (50.0, 50.0),
                    (50.0, 50.0),
                ),
                gradient,
            )],
            &mut text,
        );
        assert!(!builder.mesh.vertices.is_empty());
        for v in builder.mesh.vertices {
            let sx = (v.position[0] + 1.0) / 2.0 * 800.0;
            let sy = -(v.position[1] - 1.0) / 2.0 * 600.0;
            assert!(
                (100.0 - 1e-3..=300.0 + 1e-3).contains(&sx)
                    && (100.0 - 1e-3..=300.0 + 1e-3).contains(&sy),
                "vertex ({sx},{sy}) outside rounded rect bounds"
            );
            // The four corner cut-outs (quarter arcs of radius 50 around the
            // corner centers) must stay empty.
            let corners = [
                (150.0, 150.0, -1.0, -1.0),
                (250.0, 150.0, 1.0, -1.0),
                (250.0, 250.0, 1.0, 1.0),
                (150.0, 250.0, -1.0, 1.0),
            ];
            for (cx, cy, sxg, syg) in corners {
                let dx = (sx - cx) * sxg;
                let dy = (sy - cy) * syg;
                if dx > 0.0 && dy > 0.0 {
                    let d2 = dx * dx + dy * dy;
                    let slack = 50.0 + 0.5;
                    assert!(
                        d2 <= slack * slack,
                        "vertex ({sx},{sy}) lies in corner cut-out near ({cx},{cy})"
                    );
                }
            }
        }
    }

    #[test]
    fn test_gradient_linear_edge_colors() {
        // A 90° linear gradient goes left→right; the rect's left and right
        // edges must carry the first and last stop colors.
        let mut builder = MeshBuilder::new(400.0, 200.0, 1.0);
        let mut text: Option<&mut dyn TextLayoutSource> = Some(&mut NoText);
        let gradient = Gradient {
            kind: GradientKind::Linear { angle: 90.0 },
            stops: vec![
                ColorStop {
                    color: Color(255, 0, 0, 255),
                    position: Some(0.0),
                },
                ColorStop {
                    color: Color(0, 0, 255, 255),
                    position: Some(1.0),
                },
            ],
        };
        builder.build(
            &[gradient_fill(rect_path(0.0, 0.0, 200.0, 100.0), gradient)],
            &mut text,
        );
        assert!(!builder.mesh.vertices.is_empty());
        let mut saw_left = false;
        let mut saw_right = false;
        for v in builder.mesh.vertices {
            let sx = (v.position[0] + 1.0) / 2.0 * 400.0;
            if sx < 0.5 {
                saw_left = true;
                assert!(v.color[0] > 0.8, "left edge not red: {:?}", v.color);
                assert!(v.color[2] < 0.2, "left edge has blue: {:?}", v.color);
            }
            if sx > 199.5 {
                saw_right = true;
                assert!(v.color[0] < 0.2, "right edge has red: {:?}", v.color);
                assert!(v.color[2] > 0.8, "right edge not blue: {:?}", v.color);
            }
        }
        assert!(saw_left && saw_right, "expected vertices at both edges");
    }

    #[test]
    fn test_builder_fills_rect() {
        let mut builder = MeshBuilder::new(800.0, 600.0, 1.0);
        let mut text: Option<&mut dyn TextLayoutSource> = Some(&mut NoText);
        builder.build(
            &[solid_fill(
                rect_path(0.0, 0.0, 100.0, 100.0),
                Color(255, 0, 0, 255),
            )],
            &mut text,
        );
        assert_eq!(builder.mesh.vertices.len(), 6);
        // All vertices within the rect bounds in screen space.
        for v in builder.mesh.vertices {
            let sx = (v.position[0] + 1.0) / 2.0 * 800.0;
            let sy = -(v.position[1] - 1.0) / 2.0 * 600.0;
            assert!((0.0 - 1e-3..=100.0 + 1e-3).contains(&sx), "sx={sx}");
            assert!((0.0 - 1e-3..=100.0 + 1e-3).contains(&sy), "sy={sy}");
        }
    }

    #[test]
    fn test_builder_transform_translates_geometry() {
        let mut builder = MeshBuilder::new(800.0, 600.0, 1.0);
        let mut text: Option<&mut dyn TextLayoutSource> = Some(&mut NoText);
        builder.build(
            &[
                DrawCommand::PushTransform {
                    transform: AffineTransform::translate(50.0, 30.0),
                },
                solid_fill(rect_path(0.0, 0.0, 10.0, 10.0), Color(255, 0, 0, 255)),
                DrawCommand::PopTransform,
            ],
            &mut text,
        );
        assert_eq!(builder.mesh.vertices.len(), 6);
        for v in builder.mesh.vertices {
            let sx = (v.position[0] + 1.0) / 2.0 * 800.0;
            let sy = -(v.position[1] - 1.0) / 2.0 * 600.0;
            assert!((50.0..=60.0).contains(&sx), "sx={sx}");
            assert!((30.0..=40.0).contains(&sy), "sy={sy}");
        }
    }

    #[test]
    fn test_builder_clip_intersects_geometry() {
        let mut builder = MeshBuilder::new(800.0, 600.0, 1.0);
        let mut text: Option<&mut dyn TextLayoutSource> = Some(&mut NoText);
        builder.build(
            &[
                DrawCommand::PushClip {
                    path: rect_path(10.0, 10.0, 40.0, 40.0),
                    rule: crate::engine::renderer_model::FillRule::NonZero,
                },
                solid_fill(rect_path(0.0, 0.0, 100.0, 100.0), Color(255, 0, 0, 255)),
                DrawCommand::PopClip,
            ],
            &mut text,
        );
        assert!(!builder.mesh.vertices.is_empty());
        for v in builder.mesh.vertices {
            let sx = (v.position[0] + 1.0) / 2.0 * 800.0;
            let sy = -(v.position[1] - 1.0) / 2.0 * 600.0;
            assert!((10.0 - 1e-3..=50.0 + 1e-3).contains(&sx), "sx={sx}");
            assert!((10.0 - 1e-3..=50.0 + 1e-3).contains(&sy), "sy={sy}");
        }
    }

    #[test]
    fn test_builder_culls_offscreen_fill() {
        let mut builder = MeshBuilder::new(800.0, 600.0, 1.0);
        let mut text: Option<&mut dyn TextLayoutSource> = Some(&mut NoText);
        builder.build(
            &[solid_fill(
                rect_path(1000.0, 1000.0, 100.0, 100.0),
                Color(255, 0, 0, 255),
            )],
            &mut text,
        );
        assert!(builder.mesh.vertices.is_empty());
    }

    #[test]
    fn test_builder_concave_path_area() {
        // Concave path: total emitted area must match the path's area.
        let mut builder = MeshBuilder::new(800.0, 600.0, 1.0);
        let mut text: Option<&mut dyn TextLayoutSource> = Some(&mut NoText);
        let mut path = Path::new();
        path.move_to(100.0, 100.0);
        path.line_to(200.0, 100.0);
        path.line_to(200.0, 200.0);
        path.line_to(150.0, 150.0);
        path.line_to(100.0, 200.0);
        path.close();
        builder.build(&[solid_fill(path, Color(255, 0, 0, 255))], &mut text);
        assert!(!builder.mesh.vertices.is_empty());
        assert_eq!(builder.mesh.vertices.len() % 3, 0);
        // Concave "arrowhead": area = 100*100 - 2*(50*50/2) = 7500 physical px,
        // but `area_sum` works in normalized NDC space, so compare against
        // 7500 / (800 * 600).
        let covered = area_sum(&builder.mesh.vertices) * (800.0 * 600.0);
        assert!(
            (covered - 7500.0).abs() < 1.0,
            "covered={covered} expected=7500"
        );
    }

    #[test]
    fn test_builder_scale_factor() {
        let mut builder = MeshBuilder::new(1600.0, 1200.0, 2.0);
        let mut text: Option<&mut dyn TextLayoutSource> = Some(&mut NoText);
        builder.build(
            &[solid_fill(
                rect_path(0.0, 0.0, 50.0, 50.0),
                Color(255, 0, 0, 255),
            )],
            &mut text,
        );
        assert_eq!(builder.mesh.vertices.len(), 6);
        for v in builder.mesh.vertices {
            let sx = (v.position[0] + 1.0) / 2.0 * 1600.0;
            let sy = -(v.position[1] - 1.0) / 2.0 * 1200.0;
            assert!((0.0 - 1e-3..=100.0 + 1e-3).contains(&sx), "sx={sx}");
            assert!((0.0 - 1e-3..=100.0 + 1e-3).contains(&sy), "sy={sy}");
        }
    }
}
