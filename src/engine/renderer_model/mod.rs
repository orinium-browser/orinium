//! Render model and draw command generation module.

mod box_model;
mod draw_command;
mod geom;
mod path;

pub use box_model::generate_draw_commands;
pub use draw_command::{Brush, DrawCommand, FillRule, Paint, SystemUiKind};
pub use geom::{AffineTransform, Rect};
pub use path::{
    Path, PathCommand, clamp_radii, ellipse_path, polygon_path, rect_path, rounded_rect_path,
};
