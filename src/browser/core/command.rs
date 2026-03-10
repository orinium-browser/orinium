#[derive(Debug, Clone)]
pub enum BrowserCommand {
    None,
    Exit,
    RequestRedraw,
    RenameWindowTitle,
    OpenNewWindow { tab_id: usize },
}
