//! Render model and draw command generation module.

mod box_model;
mod draw_command;
mod geom;
mod path;

pub use box_model::{StickyViewport, generate_draw_commands, is_scrollport, sticky_offset};
pub use draw_command::{Brush, DrawCommand, FillRule, Image, Paint, SystemUiKind};
pub use geom::{AffineTransform, Rect};
pub use path::{
    Path, PathCommand, Stroke, StrokeCap, StrokeJoin, clamp_radii, ellipse_path, line_path,
    offset_path, polygon_path, polyline_path, rect_path, rounded_rect_path, stroke_path,
    transform_path,
};
