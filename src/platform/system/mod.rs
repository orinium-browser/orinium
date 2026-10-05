//! System platform abstraction. Window and event management.

pub mod app;
pub mod shell;

pub use app::App;
pub use app::WindowState;
pub use shell::{BrowserHost, ShellCommand, WindowGeometry};
