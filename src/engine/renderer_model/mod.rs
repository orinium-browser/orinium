//! Render model and draw command generation module.

mod draw_command;
mod path;

pub use draw_command::{
    AffineTransform, Brush, DrawCommand, FillRule, Paint, Rect, SystemUiKind,
    generate_draw_commands,
};
pub use path::{
    Path, PathCommand, clamp_radii, ellipse_path, polygon_path, rect_path, rounded_rect_path,
};
