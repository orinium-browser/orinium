//! The browser's OS-facing command vocabulary.
//!
//! The enum itself lives in [`platform::system::shell`] because every variant
//! is an instruction for the OS shell, not browser state. It is re-exported
//! here under its historical name so browser-internal code keeps reading
//! naturally.

pub use crate::platform::system::shell::ShellCommand as BrowserCommand;
