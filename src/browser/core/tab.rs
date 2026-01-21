use std::sync::Arc;

use super::resource_loader::BrowserResourceLoader;
use crate::network::NetworkCore;

use crate::engine::layouter::types::InfoNode;
use ui_layout::LayoutNode;

use super::webview::{WebView, WebViewTask};

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

    /// Tab 内の状態を 1 ステップ進める
    ///
    /// - WebView.tick() を呼び出す
    /// - 発生した Task を BrowserApp に返す
    pub fn tick(&mut self) -> Vec<WebViewTask> {
        let Some(webview) = self.webview.as_mut() else {
            return Vec::new();
        };

        webview.tick()
    }

    /// BrowserApp から HTML fetch 完了を通知
    pub fn on_html_fetched(&mut self, html: String, base_url: &str) {
        if let Some(webview) = self.webview.as_mut() {
            self.url = Some(base_url.to_string());
            webview.on_html_fetched(html, base_url);

            // title は WebView から吸い上げる
            self.title = webview.title().cloned();
        }
    }

    /// BrowserApp から CSS fetch 完了を通知
    pub fn on_css_fetched(&mut self, css: String) {
        if let Some(webview) = self.webview.as_mut() {
            webview.on_css_fetched(css);
        }
    }

    /// 初回ナビゲーション開始
    pub fn navigate(&mut self, url: String) -> Vec<WebViewTask> {
        self.url = Some(url.clone());
        self.webview = Some(WebView::new());

        // 最初の HTML fetch 要求をそのまま上に投げる
        vec![WebViewTask::Fetch {
            url,
            kind: super::webview::FetchKind::Html,
        }]
    }

    pub fn title(&self) -> Option<String> {
        self.title.clone()
    }

    pub fn url(&self) -> Option<String> {
        self.url.clone()
    }

    pub fn layout_and_info(&self) -> Option<&(LayoutNode, InfoNode)> {
        self.webview.as_ref().and_then(|wv| wv.layout_and_info())
    }

    pub fn needs_redraw(&self) -> bool {
        self.webview
            .as_ref()
            .map(|wv| wv.needs_redraw())
            .unwrap_or(false)
    }

    pub fn clear_redraw_flag(&mut self) {
        if let Some(wv) = self.webview.as_mut() {
            wv.clear_redraw_flag();
        }
    }
}
