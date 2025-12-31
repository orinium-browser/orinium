//! TODO: コアモジュール以外にwebviewなどの外部アプリ向けのっモジュールを公開

pub mod core;
pub mod url_solver;

pub use core::BrowserApp;
pub use core::BrowserCommand;
pub use core::Tab;
