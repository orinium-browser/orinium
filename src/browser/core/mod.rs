//! Browser core: application lifecycle, tab management, and engine integration.
//!
//! This module contains the core glue between the browser engine (`engine`)
//! and the platform (`platform`). It exposes the high-level application
//! entrypoint [`BrowserApp`], the [`Tab`] abstraction, and command primitives.
//!
//! Processing flow (high-level)
//! - Resource acquisition: `resource_loader` / platform network → HTML/CSS input
//! - Parsing: HTML tokenizer/parser → DOM
//! - Style resolution: CSS parser + cascade → computed style
//! - Layout: layout builder produces layout tree
//! - Render model: generate draw commands from layout tree
//! - Platform render: platform renderer consumes draw commands and composites output
//!
//! Quick example (for contributors)
//! ```no_run
//! use orinium_browser::browser::{BrowserApp, BrowserUi, Tab};
//!
//! // Navigate the tab to a resource or URL (error handling elided)
//! let mut tab = Tab::new();
//! tab.navigate("resource:///test/test.html".parse().unwrap());
//!
//! // Create browser with the pre-configured tab and run it
//! let mut browser = BrowserApp::default();
//! browser.set_default_ui(BrowserUi::with_tab(tab));
//! browser.run().unwrap();
//! ```
//!
//! Contributor notes
//! - Prefer small, focused commits that add tests for new behavior.
//! - Read module-level docs in `engine` (parser / layouter) and `platform`
//!   (network / renderer) before changing cross-cutting logic.
//! - Typical edit cycle: add unit tests → implement change in `engine` → verify
//!   draw-command output → ensure platform paints correctly.
//!
//! See submodules for specifics and examples.

mod app;
mod command;
pub mod resource_loader;
pub mod tab;
pub mod ui;
pub mod webview;

pub use app::BrowserApp;
pub use command::BrowserCommand;
pub use tab::Tab;
pub use ui::{
    BasicChrome, BrowserRenderer, BrowserUi, Chrome, ChromeAction, ChromeEventResult, RenderState,
};
