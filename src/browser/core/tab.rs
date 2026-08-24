//! ブラウザのタブ機能。WebView を保持し、ページのタイトルや URL などのメタ情報を管理する。

use crate::{
    browser::core::{
        resource_loader::{BrowserNetworkError, BrowserResponse},
        webview::JsPolicy,
    },
    engine::{
        html::HtmlNodeType,
        js::JsFetchResponse,
        layouter::types::{ColorScheme, InfoNode},
        tree::TreeNode,
    },
};
use ui_layout::LayoutNode;
use url::Url;

pub use super::webview::{CssApplicationStrategy, FetchKind, WebView, WebViewTask};

pub enum TabTask {
    Fetch {
        url: Url,
        kind: FetchKind,
    },
    NeedsRedraw,
    /// A page asked the DevTools bridge to inspect rendered state.
    DevToolsRequest {
        id: u64,
        method: String,
        params: String,
    },
}

#[derive(Debug)]
enum TabError {
    NetworkError(BrowserNetworkError),
}

#[derive(Debug)]
enum TabState {
    Loading,
    Loaded,
    Error(TabError, Option<Url>), // エラーの種類と、失敗した URL（ある場合）
}

/// Tab はブラウザで開かれた 1 つのページを表す構造体です。
///
/// 主な責務:
/// - 現在表示しているページのタイトルの保持
/// - ページ内容を扱う WebView の保持
///
/// WebView が「ページそのもの」の状態を管理するのに対し、
/// Tab は UI 上のタブとしてのメタ情報（タイトルなど）を管理します。
pub struct Tab {
    title: Option<String>,
    base_url: Option<Url>,
    docment_url: Option<Url>,
    webview: Option<WebView>,

    system_color_scheme: ColorScheme,

    /// Policy controlling whether page scripts are executed and how
    /// `<noscript>` contents are parsed.
    js_policy: JsPolicy,

    state: TabState,
    /// Previously visited URLs, most recent last. Used by the back button.
    history: Vec<Url>,
}

impl Default for Tab {
    fn default() -> Self {
        Self::new(ColorScheme::default(), JsPolicy::default())
    }
}

impl Tab {
    pub fn new(system_color_scheme: ColorScheme, js_policy: JsPolicy) -> Self {
        Self {
            title: None,
            base_url: None,
            docment_url: None,
            webview: None,

            system_color_scheme,

            js_policy,

            state: TabState::Loading,
            history: Vec::new(),
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
                    log::info!("Fetch requested in Tab: url={}", url);
                    tasks.push(TabTask::Fetch { url, kind });
                }
                WebViewTask::AskTabHtml => {
                    tasks.push(TabTask::Fetch {
                        url: self.docment_url.as_ref().unwrap().clone(),
                        kind: FetchKind::Html,
                    });
                }
                WebViewTask::DevToolsRequest { id, method, params } => {
                    tasks.push(TabTask::DevToolsRequest { id, method, params });
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
        log::info!("CSS fetched in Tab");
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
        log::info!("HTML fetched, base_url={}", base_url);
        self.base_url = Some(base_url);

        if let TabState::Error(TabError::NetworkError(err), url_opt) = &self.state {
            let error_message = match url_opt {
                Some(url) => format!("Failed to load {}: {}", url, err),
                None => format!("Failed to load page: {}", err),
            };

            let error_message_element = wv
                .document_info()
                .unwrap()
                .dom
                .get_elements_by_class_name("error-message");
            let error_message_element = error_message_element.first().unwrap();
            let new_child = TreeNode::new(HtmlNodeType::Text(error_message));
            TreeNode::replace_child(error_message_element, 0, new_child);

            // Update page to show error message
            // This is a stub implementation for now as you can see in WebView.update_page().
            wv.update_page();
        } else {
            self.state = TabState::Loaded;
        }
    }

    pub fn on_fetch_succeeded_css(&mut self, css: String) {
        let Some(wv) = self.webview.as_mut() else {
            return;
        };

        wv.on_css_fetched(css);
    }

    pub fn on_fetch_succeeded_css_from(&mut self, css: String, stylesheet_url: &Url) {
        let Some(wv) = self.webview.as_mut() else {
            return;
        };
        wv.on_css_fetched_from(css, stylesheet_url);
    }

    /// Delivers encoded image bytes to the page that requested them.
    pub fn on_fetch_succeeded_image(&mut self, source: String, bytes: &[u8]) {
        let Some(wv) = self.webview.as_mut() else {
            return;
        };
        if let Err(error) = wv.on_image_fetched(source, bytes) {
            log::warn!("Failed to decode fetched image: {error}");
        }
    }

    /// Delivers encoded audio bytes to the page that requested them.
    pub fn on_fetch_succeeded_audio(&mut self, source: String, bytes: &[u8]) {
        if let Some(webview) = self.webview.as_mut() {
            webview.on_audio_fetched(source, bytes);
        }
    }

    /// Delivers a fetched external classic script in document order.
    pub fn on_fetch_succeeded_script(&mut self, index: usize, source: String) {
        if let Some(webview) = self.webview.as_mut() {
            webview.on_script_fetched(index, source);
        }
    }

    /// Skips a failed external classic script without replacing the page.
    pub fn on_fetch_failed_script(&mut self, index: usize) {
        if let Some(webview) = self.webview.as_mut() {
            webview.on_script_fetch_failed(index);
        }
    }

    pub fn on_fetch_succeeded_dynamic_script(&mut self, node_id: u64, source: String) {
        if let Some(webview) = self.webview.as_mut() {
            webview.on_dynamic_script_fetched(node_id, source);
        }
    }

    pub fn on_fetch_failed_dynamic_script(&mut self, node_id: u64) {
        if let Some(webview) = self.webview.as_mut() {
            webview.on_dynamic_script_fetch_failed(node_id);
        }
    }

    pub fn on_fetch_succeeded_dynamic_style(&mut self, node_id: u64, source: String) {
        if let Some(webview) = self.webview.as_mut() {
            webview.on_dynamic_style_fetched(node_id, source);
        }
    }

    pub fn on_fetch_failed_dynamic_style(&mut self, node_id: u64) {
        if let Some(webview) = self.webview.as_mut() {
            webview.on_dynamic_style_fetch_failed(node_id);
        }
    }

    /// Delivers a completed JavaScript `fetch()` response.
    pub fn on_fetch_succeeded_js(
        &mut self,
        request_id: u64,
        response: BrowserResponse,
        redirected: bool,
    ) {
        if let Some(webview) = self.webview.as_mut() {
            webview.on_js_fetch_succeeded(
                request_id,
                JsFetchResponse {
                    url: response.url,
                    status: response.status.as_u16(),
                    status_text: response.status_text,
                    redirected,
                    body: response.body,
                    headers: response.headers,
                },
            );
        }
    }

    /// Delivers a JavaScript `fetch()` network failure without navigating away.
    pub fn on_fetch_failed_js(&mut self, request_id: u64, reason: String) {
        if let Some(webview) = self.webview.as_mut() {
            webview.on_js_fetch_failed(request_id, reason);
        }
    }

    /// Answers a DevTools inspection query against this tab's page.
    pub fn inspect(&mut self, method: &str, params: &str) -> Result<serde_json::Value, String> {
        match self.webview.as_mut() {
            Some(webview) => webview.inspect(method, params),
            None => Err("no page".to_string()),
        }
    }

    /// Settles a DevTools inspection request with its JSON envelope.
    pub fn on_devtools_response(&mut self, id: u64, result: String) {
        if let Some(webview) = self.webview.as_mut() {
            webview.on_devtools_response(id, result);
        }
    }

    /// Display error page on fetch failure
    pub fn on_fetch_failed(&mut self, err: BrowserNetworkError, failed_url: Url) {
        self.navigate("resource:///error.html".parse().unwrap());
        self.state = TabState::Error(TabError::NetworkError(err), Some(failed_url));
    }

    pub fn navigate(&mut self, url: Url) {
        self.navigate_internal(url, true);
    }

    /// Navigates to the previous URL in the history, if any.
    ///
    /// Returns `false` when there is no history to go back to.
    pub fn go_back(&mut self) -> bool {
        let Some(previous) = self.history.pop() else {
            return false;
        };
        self.navigate_internal(previous, false);
        true
    }

    /// Reloads the current document, if one is loaded.
    pub fn reload(&mut self) {
        if let Some(url) = self.docment_url.clone() {
            self.navigate_internal(url, false);
        }
    }

    /// Returns whether the back button can navigate to a previous page.
    pub fn can_go_back(&self) -> bool {
        !self.history.is_empty()
    }

    fn navigate_internal(&mut self, url: Url, record_history: bool) {
        if record_history
            && self.docment_url.as_ref() != Some(&url)
            && let Some(previous) = self.docment_url.clone()
        {
            self.history.push(previous);
        }
        self.docment_url = Some(url);
        let mut webview = WebView::new(self.system_color_scheme, self.js_policy);
        webview.navigate();
        self.webview = Some(webview);
        self.state = TabState::Loading;
        self.title = None;
        self.base_url = None;
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

    /// Returns layout_and_info
    /// Only InfoNode will be mutable.
    pub fn layout_and_info_mut(&mut self) -> Option<(&LayoutNode, &mut InfoNode)> {
        self.webview
            .as_mut()
            .and_then(|wv| wv.layout_and_info_mut())
    }

    /// Dispatches a click on a DOM node to the page's JS `onclick` handler.
    ///
    /// Returns whether the click mutated the DOM and needs a redraw.
    pub fn on_js_click(&mut self, dom_id: u32) -> bool {
        self.webview
            .as_mut()
            .is_some_and(|wv| wv.on_js_click(dom_id))
    }

    pub fn set_system_color_scheme(&mut self, scheme: ColorScheme) {
        self.system_color_scheme = scheme;
        if let Some(wv) = self.webview.as_mut() {
            wv.set_system_color_scheme(scheme)
        }
    }

    /// Returns title of the document
    pub fn title(&self) -> Option<String> {
        self.title.clone()
    }

    /// Returns document url
    pub fn document_url(&self) -> Option<Url> {
        self.docment_url.clone()
    }

    pub fn layout_and_info(&self) -> Option<(&LayoutNode, &InfoNode)> {
        self.webview.as_ref().and_then(|wv| wv.layout_and_info())
    }

    pub fn needs_redraw(&self) -> bool {
        self.webview.as_ref().is_some_and(|wv| wv.needs_redraw())
    }

    pub fn clear_redraw_flag(&mut self) {
        if let Some(wv) = self.webview.as_mut() {
            wv.clear_redraw_flag();
        }
    }

    pub fn set_js_policy(&mut self, policy: JsPolicy) {
        self.js_policy = policy;
        if let Some(wv) = self.webview.as_mut() {
            wv.set_js_policy(policy);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn navigate_records_history() {
        let mut tab = Tab::default();
        tab.navigate(url("https://example.test/a"));
        tab.navigate(url("https://example.test/b"));

        assert!(tab.can_go_back());
        assert_eq!(
            tab.document_url().as_ref().map(Url::as_str),
            Some("https://example.test/b")
        );
    }

    #[test]
    fn navigating_to_same_url_does_not_duplicate_history() {
        let mut tab = Tab::default();
        tab.navigate(url("https://example.test/a"));
        tab.navigate(url("https://example.test/a"));
        assert!(!tab.can_go_back());
    }

    #[test]
    fn go_back_restores_previous_url() {
        let mut tab = Tab::default();
        tab.navigate(url("https://example.test/a"));
        tab.navigate(url("https://example.test/b"));

        assert!(tab.go_back());
        assert_eq!(
            tab.document_url().as_ref().map(Url::as_str),
            Some("https://example.test/a")
        );
        assert!(!tab.can_go_back());
        // No history left: going back again reports failure.
        assert!(!tab.go_back());
    }

    #[test]
    fn reload_keeps_url_without_recording_history() {
        let mut tab = Tab::default();
        tab.navigate(url("https://example.test/a"));
        tab.reload();
        assert_eq!(
            tab.document_url().as_ref().map(Url::as_str),
            Some("https://example.test/a")
        );
        assert!(!tab.can_go_back());
    }
}
