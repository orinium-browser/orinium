use crate::{
    browser::core::resource_loader::BrowserNetworkError, engine::layouter::types::InfoNode,
};
use ui_layout::LayoutNode;
use url::Url;

pub use super::webview::{FetchKind, WebView, WebViewTask};

pub enum TabTask {
    Fetch { url: Url, kind: FetchKind },
    NeedsRedraw,
}

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
    base_url: Option<Url>,
    docment_url: Option<Url>,
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
            base_url: None,
            docment_url: None,
            webview: None,
        }
    }

    /// Tab 内の状態を 1 ステップ進める
    ///
    /// - WebView.tick() を呼び出す
    /// - 発生した Task を BrowserApp に返す
    pub fn tick(&mut self) -> Vec<TabTask> {
        let mut tasks = Vec::new();
        let Some(wv) = self.webview.as_mut() else {
            return tasks;
        };

        for task in wv.tick() {
            match task {
                WebViewTask::Fetch { url, kind } => {
                    println!("Fetch requested in Tab: url={}", url);
                    tasks.push(TabTask::Fetch { url, kind });
                }
                WebViewTask::AskTabHtml => {
                    tasks.push(TabTask::Fetch {
                        url: self.docment_url.as_ref().unwrap().clone(),
                        kind: FetchKind::Html,
                    });
                }
            }
        }

        if wv.needs_redraw() {
            tasks.push(TabTask::NeedsRedraw);
        }

        tasks
    }

    /// BrowserApp から CSS fetch 完了を通知
    pub fn on_css_fetched(&mut self, css: String) {
        println!("CSS fetched in Tab");
        if let Some(webview) = self.webview.as_mut() {
            webview.on_css_fetched(css);
        }
    }

    /// BrowserApp からの HTML fetch 完了を通知
    pub fn on_fetch_succeeded_html(&mut self, html: String) {
        let Some(wv) = self.webview.as_mut() else {
            return;
        };

        wv.on_html_fetched(html, self.docment_url.as_ref().unwrap().clone());
        self.title = wv.title().cloned();
        let base_url = wv.base_url().unwrap().clone();
        println!("HTML fetched, base_url={}", base_url);
        self.base_url = Some(base_url);
    }

    pub fn on_fetch_succeeded_css(&mut self, css: String) {
        let Some(wv) = self.webview.as_mut() else {
            return;
        };

        wv.on_css_fetched(css);
    }

    /// Stub
    pub fn on_fetch_failed(&mut self, _err: BrowserNetworkError) {}

    pub fn navigate(&mut self, url: Url) {
        self.docment_url = Some(url.clone());
        let mut webview = WebView::new();
        webview.navigate();
        self.webview = Some(webview);
    }

    pub fn move_to(&mut self, href: &str) {
        let base_url = match self.base_url.as_ref() {
            Some(u) => u,
            None => return,
        };

        let url = super::webview::resolve_url(base_url, href).unwrap();

        // navigate と同じ扱い
        self.navigate(url)
    }

    pub fn relayout(&mut self, viewport: (f32, f32)) {
        if let Some(wv) = self.webview.as_mut() {
            wv.relayout(viewport);
        }
    }

    pub fn layout_and_info_mut(&mut self) -> Option<(&LayoutNode, &mut InfoNode)> {
        self.webview
            .as_mut()
            .and_then(|wv| wv.layout_and_info_mut())
    }

    pub fn title(&self) -> Option<String> {
        self.title.clone()
    }

    pub fn document_url(&self) -> Option<Url> {
        self.docment_url.clone()
    }

    pub fn layout_and_info(&self) -> Option<(&LayoutNode, &InfoNode)> {
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
