//! Render model and draw command generation module.

mod draw_command;
mod path;

pub use draw_command::{
    AffineTransform, Brush, DrawCommand, FillRule, Paint, Rect, SystemUiKind,
    generate_draw_commands,
};
pub use path::{Path, PathCommand, ellipse_path, polygon_path, rect_path};
