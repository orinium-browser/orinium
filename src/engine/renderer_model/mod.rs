//! Render model and draw command generation module.

mod draw_command;

pub use draw_command::{DrawCommand, SystemUiKind, generate_draw_commands};
