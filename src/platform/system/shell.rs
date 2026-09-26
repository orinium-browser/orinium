//! The contract between the browser core and the OS shell.
//!
//! The browser core drives the browser; [`App`](super::App) owns the `winit` event loop,
//! the OS windows and the GPU surface. Neither side may depend on the other's
//! internals, so the seam is declared *here*, in the layer that owns the OS,
//! and implemented by the browser core.
//!
//! Everything crossing this seam is a value owned by `platform`:
//!
//! - [`WindowGeometry`] instead of a per-window UI object
//! - [`ShellCommand`] instead of a browser-internal command enum
//! - plain `winit` / GPU handles instead of engine state
//!
//! That way the event loop can hold any [`BrowserHost`] — including one
//! living in another process — without naming a single browser type.
use winit::event::WindowEvent;
use winit::window::WindowId;

use crate::platform::renderer::gpu::GpuRenderer;

/// An instruction for the OS shell, returned after handling an event.
#[derive(Debug, Clone, PartialEq)]
pub enum ShellCommand {
    /// Nothing to do.
    None,
    /// Close the window; the shell exits once no window remains.
    Exit,
    /// Repaint the window.
    RequestRedraw,
    /// Re-read the window title from the host and apply it.
    RenameWindowTitle,
    /// Create an additional window.
    OpenNewWindow,
    /// Enable or disable OS IME input near the last click position.
    SetImeAllowed { allowed: bool, position: (f64, f64) },
}

/// The properties the shell needs to realise a window.
#[derive(Debug, Clone)]
pub struct WindowGeometry {
    /// Initial inner size, in physical pixels.
    pub size: (u32, u32),
    /// Initial title.
    pub title: String,
    /// Display scale factor at creation time.
    pub scale_factor: f64,
}

/// The browser side of the shell contract.
///
/// The shell calls these methods; the browser core implements them. Every
/// method takes and returns `platform`-owned values only.
pub trait BrowserHost {
    /// The size to give a newly opened window.
    fn default_window_size(&self) -> (u32, u32);

    /// The title to give a newly opened window.
    fn default_window_title(&self) -> String;

    /// Registers a window with the host.
    ///
    /// The host decides what the window shows: the first call installs the UI
    /// it was given before the loop started, and any later call opens an
    /// empty window.
    fn open_window(&mut self, window_id: WindowId, geometry: WindowGeometry);

    /// Whether a UI is still waiting to be installed in the first window.
    ///
    /// The shell checks this before creating the first window so it can report
    /// a misconfigured startup instead of silently opening a blank window.
    fn has_pending_default_window(&self) -> bool;

    /// Forgets a window that the shell closed.
    fn close_window(&mut self, window_id: WindowId);

    /// The current title for a window.
    fn window_title(&self, window_id: WindowId) -> String;

    /// Handles one OS event for a window and reports what the shell should do.
    fn handle_window_event(
        &mut self,
        window_id: WindowId,
        event: WindowEvent,
        gpu: &mut GpuRenderer,
    ) -> ShellCommand;

    /// Uploads the window's pending draw commands to the GPU.
    fn apply_draw_commands(&mut self, window_id: WindowId, gpu: &mut GpuRenderer);

    /// Advances background page work for a window, returning whether it needs
    /// a repaint.
    fn poll_window(&mut self, window_id: WindowId) -> bool;
}
