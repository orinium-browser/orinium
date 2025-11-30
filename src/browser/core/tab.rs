use crate::renderer::RenderTree;

use super::webview::WebView;

/// Tab はブラウザで開かれた 1 つのページを表す構造体です。
///
/// 主な責務:
/// - 現在表示しているページのタイトルの保持
/// - ページ内容を扱う WebView の保持
///
/// WebView が「ページそのもの」の状態を管理するのに対し、
/// Tab は UI 上のタブとしてのメタ情報（タイトルなど）を管理します。
///
/// TODO:
/// - ページの状態（Error、loading）の管理を追加
pub struct Tab {
    title: Option<String>,
    url: Option<String>,
    webview: Option<WebView>,
}

impl Default for Tab {
    fn default() -> Self {
        Self::new()
    }
}

impl Tab {
    pub fn new() -> Self {
        Self {
            title: None,
            url: None,
            webview: None,
        }
    }
    pub fn load_from_raw_html(&mut self, html_source: &str) {
        let mut view = WebView::new();
        view.load(html_source, None);
        self.title = view.title.clone();

        self.webview = Some(view);
    }

    pub fn render_tree(&self) -> Option<&RenderTree> {
        self.webview.as_ref().and_then(|wv| wv.render.as_ref())
    }

    pub fn title(&self) -> Option<String> {
        self.title.clone()
    }

    pub fn url(&self) -> Option<String> {
        self.url.clone()
    }
}
