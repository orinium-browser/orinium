#[derive(Debug, Clone, Copy)]
pub enum BrowserCommand {
    None,
    Exit,
    RequestRedraw,
    RenameWindowTitle,
}
