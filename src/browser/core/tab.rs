use std::sync::Arc;

use super::resource_loader::BrowserResourceLoader;
use crate::network::NetworkCore;

use crate::engine::layouter::InfoNode;
use ui_layout::LayoutNode;

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
    net: Arc<BrowserResourceLoader>,
    title: Option<String>,
    url: Option<String>,
    webview: Option<WebView>,
}

impl Default for Tab {
    fn default() -> Self {
        let net = Arc::new(BrowserResourceLoader::new(Some(Arc::new(
            NetworkCore::new(),
        ))));
        Self::new(net)
    }
}

impl Tab {
    pub fn new(net: Arc<BrowserResourceLoader>) -> Self {
        Self {
            net,
            title: None,
            url: None,
            webview: None,
        }
    }
    pub fn load_from_raw_html(&mut self, html_source: &str) {
        let mut view = WebView::new();
        view.load_from_raw_source(html_source, None);
        self.title = view.title.clone();

        self.webview = Some(view);
    }

    pub async fn load_from_url(&mut self, url: &str) -> anyhow::Result<()> {
        let net = self.net.clone();
        let mut view = WebView::new();
        view.load_from_url(url, net).await?;
        self.title = view.title.clone();

        self.webview = Some(view);
        Ok(())
    }

    pub fn layout_and_info(&mut self) -> Option<&mut (LayoutNode, InfoNode)> {
        self.webview
            .as_mut()
            .and_then(|wv| wv.layout_and_info.as_mut())
    }

    pub fn title(&self) -> Option<String> {
        self.title.clone()
    }

    pub fn url(&self) -> Option<String> {
        self.url.clone()
    }

    pub fn scroll_page(&mut self, delta_x: f32, delta_y: f32) {
        if let Some(webview) = &mut self.webview {
            webview.scroll_page(delta_x, delta_y);
        }
    }
}
