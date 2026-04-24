use super::{ComponentEvent, DrawCommandEmitter};
use crate::engine::layouter::types::{Color, FontWeight, TextStyle};
use crate::engine::renderer_model::DrawCommand;
use ui_layout::LayoutNode;

/// Button
#[derive(Debug)]
pub struct Button {
    pub id: String,
    pub label: String,
    pub layout: LayoutNode,
    pub hovered: bool,
    pub active: bool,
    pub focused: bool,
}

impl Button {
    pub fn new(id: impl Into<String>, label: impl Into<String>, layout: LayoutNode) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            layout,
            hovered: false,
            active: false,
            focused: false,
        }
    }

    pub fn hit_test(&self, x: f32, y: f32) -> bool {
        self.layout.layout_boxes.iter().any(|box_model| {
            let rect = box_model.padding_box;
            x >= rect.x && x <= rect.x + rect.width && y >= rect.y && y <= rect.y + rect.height
        })
    }

    pub fn on_event(&mut self, event: ComponentEvent) -> bool {
        match event {
            ComponentEvent::PointerDown { x, y } => {
                log::info!("Button {} handled PointerDown at ({}, {})", self.id, x, y);
                self.active = true;
                true
            }
            ComponentEvent::PointerUp { x, y } => {
                log::info!("Button {} handled PointerUp at ({}, {})", self.id, x, y);
                self.active = false;
                true
            }
        }
    }
}

impl DrawCommandEmitter for Button {
    fn draw_commands(&self) -> Vec<DrawCommand> {
        let text_style = TextStyle {
            font_size: 16.0,
            font_weight: FontWeight::BOLD,
            color: Color(255, 255, 255, 255),
            ..Default::default()
        };

        self.layout
            .layout_boxes
            .iter()
            .flat_map(|box_model| {
                let rect = box_model.padding_box;
                let border_color = if self.focused {
                    Color(20, 20, 20, 255)
                } else {
                    Color(200, 200, 200, 255)
                };
                let fill_color = if self.active {
                    // slightly lighter active color
                    Color(180, 180, 180, 255)
                } else if self.hovered {
                    Color(240, 240, 240, 255)
                } else {
                    Color(255, 255, 255, 255)
                };

                // radius and border thickness
                let r = 6.0_f32; // smoother radius
                let border_thickness = 1.0_f32;

                let rounded_rect =
                    |w: f32, h: f32, radius: f32, segments_per_corner: usize| -> Vec<(f32, f32)> {
                        use std::f32::consts::PI;
                        let seg = segments_per_corner.max(1) as f32;
                        let mut pts: Vec<(f32, f32)> = Vec::new();

                        // Top-left corner: arc from 180deg to 270deg
                        let tl_cx = radius;
                        let tl_cy = radius;
                        for i in 0..segments_per_corner {
                            let t = i as f32 / seg;
                            let angle = PI + t * (PI / 2.0);
                            pts.push((tl_cx + radius * angle.cos(), tl_cy + radius * angle.sin()));
                        }
                        // Top edge between corners
                        pts.push((w - radius, 0.0));

                        // Top-right corner: arc from 270deg to 360deg
                        let tr_cx = w - radius;
                        let tr_cy = radius;
                        for i in 0..segments_per_corner {
                            let t = i as f32 / seg;
                            let angle = 3.0 * PI / 2.0 + t * (PI / 2.0);
                            pts.push((tr_cx + radius * angle.cos(), tr_cy + radius * angle.sin()));
                        }
                        // Right edge
                        pts.push((w, h - radius));

                        // Bottom-right corner: arc from 0deg to 90deg
                        let br_cx = w - radius;
                        let br_cy = h - radius;
                        for i in 0..segments_per_corner {
                            let t = i as f32 / seg;
                            let angle = 0.0 + t * (PI / 2.0);
                            pts.push((br_cx + radius * angle.cos(), br_cy + radius * angle.sin()));
                        }
                        // Bottom edge
                        pts.push((radius, h));

                        // Bottom-left corner: arc from 90deg to 180deg
                        let bl_cx = radius;
                        let bl_cy = h - radius;
                        for i in 0..segments_per_corner {
                            let t = i as f32 / seg;
                            let angle = PI / 2.0 + t * (PI / 2.0);
                            pts.push((bl_cx + radius * angle.cos(), bl_cy + radius * angle.sin()));
                        }
                        pts
                    };

                let ow = rect.width;
                let oh = rect.height;
                let segments_per_corner = 8;
                let outer_poly =
                    rounded_rect(ow, oh, r.min(ow * 0.5).min(oh * 0.5), segments_per_corner);

                // inner polygon (fill shape) inset by border_thickness
                let inset = border_thickness;
                let iw = (ow - inset * 2.0).max(0.0);
                let ih = (oh - inset * 2.0).max(0.0);
                let inner_r = (r - inset).max(0.0);
                let inner_poly = rounded_rect(
                    iw,
                    ih,
                    inner_r.min(iw * 0.5).min(ih * 0.5),
                    segments_per_corner,
                )
                .into_iter()
                .map(|(x, y)| (x + inset, y + inset))
                .collect::<Vec<_>>();

                vec![
                    DrawCommand::PushTransform {
                        dx: rect.x,
                        dy: rect.y,
                    },
                    // subtle shadow under button
                    DrawCommand::DrawRect {
                        x: 0.0,
                        y: 1.0,
                        width: ow,
                        height: oh,
                        color: Color(0, 0, 0, 40),
                        radius: r,
                    },
                    // border as polygon
                    DrawCommand::DrawPolygon {
                        points: outer_poly.clone(),
                        color: border_color,
                    },
                    // fill as inner polygon so border remains visible
                    DrawCommand::DrawPolygon {
                        points: inner_poly.clone(),
                        color: fill_color,
                    },
                    DrawCommand::PushClip {
                        x: 0.0,
                        y: 0.0,
                        width: ow,
                        height: oh,
                    },
                    DrawCommand::DrawText {
                        x: 0.0,
                        y: (oh - text_style.font_size) / 2.0 - 3.0,
                        text: self.label.clone(),
                        style: TextStyle {
                            font_size: text_style.font_size,
                            font_weight: text_style.font_weight,
                            color: text_style.color,
                            text_align: crate::engine::layouter::types::TextAlign::Center,
                            ..Default::default()
                        },
                        max_width: ow * 2.0,
                    },
                    DrawCommand::PopClip,
                    DrawCommand::PopTransform,
                ]
            })
            .collect()
    }
}

pub fn find_first_text(info: &crate::engine::layouter::types::InfoNode) -> Option<String> {
    match &info.kind {
        crate::engine::layouter::types::NodeKind::Text { text, .. } => Some(text.clone()),
        _ => info
            .children
            .iter()
            .filter_map(|c| find_first_text(c))
            .next(),
    }
}

fn find_first_text_style(info: &crate::engine::layouter::types::InfoNode) -> Option<TextStyle> {
    match &info.kind {
        crate::engine::layouter::types::NodeKind::Text { style, .. } => Some(*style),
        _ => info
            .children
            .iter()
            .filter_map(|c| find_first_text_style(c))
            .next(),
    }
}

pub fn draw_from_layout(
    layout: &LayoutNode,
    info: &crate::engine::layouter::types::InfoNode,
    pointer_pos: Option<(f32, f32)>,
    pointer_down_pos: Option<(f32, f32)>,
) -> Vec<DrawCommand> {
    let default_font_size = 14.0;

    layout
        .layout_boxes
        .iter()
        .flat_map(|box_model| {
            let outer = box_model.padding_box;
            let content = box_model.content_box;
            let label = find_first_text(info).unwrap_or_else(|| "".to_string());
            let text_style = find_first_text_style(info).unwrap_or(TextStyle {
                font_size: default_font_size,
                ..Default::default()
            });
            let font_size = text_style.font_size.max(10.0);

            // clip coordinates relative to the transform origin
            let clip_x_rel = content.x - outer.x;
            let clip_y_rel = content.y - outer.y;

            // center text vertically inside content box
            let text_y = clip_y_rel + (content.height - font_size) / 2.0;

            // determine hover state from pointer_pos (pointer_pos in logical coords)
            let hovered = if let Some((px, py)) = pointer_pos {
                px >= outer.x
                    && px <= outer.x + outer.width
                    && py >= outer.y
                    && py <= outer.y + outer.height
            } else {
                false
            };

            // determine active (pressed) state from pointer_down_pos
            let active = if let Some((dx, dy)) = pointer_down_pos {
                dx >= outer.x
                    && dx <= outer.x + outer.width
                    && dy >= outer.y
                    && dy <= outer.y + outer.height
            } else {
                false
            };

            let border_col = if hovered {
                Color(175, 175, 175, 255)
            } else {
                Color(200, 200, 200, 255)
            };
            let fill_col = if active {
                // slightly lighter active color for DOM buttons
                Color(180, 180, 180, 255)
            } else if hovered {
                Color(245, 245, 245, 255)
            } else {
                Color(255, 255, 255, 255)
            };

            let r = 4.0_f32;
            let border_thickness = 1.0_f32;
            let ow = outer.width;
            let oh = outer.height;

            let outer_poly = vec![
                (r, 0.0),
                (ow - r, 0.0),
                (ow, r),
                (ow, oh - r),
                (ow - r, oh),
                (r, oh),
                (0.0, oh - r),
                (0.0, r),
            ];
            let inset = border_thickness;
            let iw = (ow - inset * 2.0).max(0.0);
            let ih = (oh - inset * 2.0).max(0.0);
            let ir = (r - inset).max(0.0);
            let inner_poly = vec![
                (inset + ir, inset),
                (inset + iw - ir, inset),
                (inset + iw, inset + ir),
                (inset + iw, inset + ih - ir),
                (inset + iw - ir, inset + ih),
                (inset + ir, inset + ih),
                (inset, inset + ih - ir),
                (inset, inset + ir),
            ];

            vec![
                DrawCommand::PushTransform {
                    dx: outer.x,
                    dy: outer.y,
                },
                DrawCommand::DrawRect {
                    x: 0.0,
                    y: 1.0,
                    width: ow,
                    height: oh,
                    color: Color(0, 0, 0, 40),
                    radius: r,
                },
                DrawCommand::DrawPolygon {
                    points: outer_poly.clone(),
                    color: border_col,
                },
                DrawCommand::DrawPolygon {
                    points: inner_poly.clone(),
                    color: fill_col,
                },
                DrawCommand::PushClip {
                    x: clip_x_rel,
                    y: clip_y_rel,
                    width: content.width,
                    height: content.height,
                },
                DrawCommand::DrawText {
                    x: clip_x_rel,
                    y: text_y,
                    text: label.clone(),
                    style: TextStyle {
                        font_size,
                        font_weight: FontWeight::BOLD,
                        color: Color(30, 30, 30, 255),
                        text_align: crate::engine::layouter::types::TextAlign::Center,
                        ..Default::default()
                    },
                    max_width: content.width,
                },
                DrawCommand::PopClip,
                DrawCommand::PopTransform,
            ]
        })
        .collect()
}

pub fn handle_pointer_down(x: f32, y: f32) {
    log::info!(
        "HTML <button> component handled PointerDown at ({}, {})",
        x,
        y
    );
}
