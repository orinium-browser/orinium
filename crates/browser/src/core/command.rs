#[derive(Debug, Clone)]
pub enum BrowserCommand {
    None,
    Exit,
    RequestRedraw,
    RenameWindowTitle,
    OpenNewWindow,
    /// Enables or disables OS IME input near the last click position.
    SetImeAllowed {
        allowed: bool,
        position: (f64, f64),
    },
}
