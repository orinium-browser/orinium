#[derive(Debug, Clone)]
pub enum BrowserCommand {
    None,
    Exit,
    RequestRedraw,
    RenameWindowTitle,
    OpenNewWindow,
}
