//! ブラウザのタブ機能。WebView を保持し、ページのタイトルや URL などのメタ情報を管理する。

use std::sync::Arc;

use crate::{
    browser::core::{
        resource_loader::{BrowserNetworkError, BrowserResponse},
        webview::JsPolicy,
    },
    engine::{
        html::HtmlNodeType,
        input::HitItem,
        js::JsFetchResponse,
        layouter::{
            self,
            types::{ColorScheme, ContainerRole, InfoNode, NodeKind},
        },
        origin::Origin,
        renderer_model::{self, DrawCommand},
        tree::TreeNode,
        ui::{CustomNode, PointerEvent, input_text_types::InputTextEvent},
    },
};
use ui_layout::LayoutNode;
use url::Url;
use winit::event::ElementState;

pub use super::webview::{CssApplicationStrategy, FetchKind, WebView, WebViewTask};

pub enum TabTask {
    Fetch {
        url: Url,
        kind: FetchKind,
        /// The origin of the document that requested this resource.
        origin: Origin,
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
    document_url: Option<Url>,
    webview: Option<WebView>,

    system_color_scheme: ColorScheme,

    /// Policy controlling whether page scripts are executed and how
    /// `<noscript>` contents are parsed.
    js_policy: JsPolicy,

    state: TabState,
    /// Previously visited URLs, most recent last. Used by the back button.
    history: Vec<Url>,

    /// The custom node currently under the pointer, if any.
    hovered: Option<Arc<dyn CustomNode>>,
    /// The DOM node under the pointer when the left button was pressed.
    ///
    /// Used to detect a completed click (press and release on the same node),
    /// which is forwarded to the page's JS `onclick` handler.
    pressed_dom_id: Option<u32>,
}

impl std::fmt::Debug for Tab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tab")
            .field("title", &self.title)
            .field("base_url", &self.base_url)
            .field("document_url", &self.document_url)
            .field("system_color_scheme", &self.system_color_scheme)
            .field("js_policy", &self.js_policy)
            .field("state", &self.state)
            .field("history", &self.history)
            .finish()
    }
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
            document_url: None,
            webview: None,

            system_color_scheme,

            js_policy,

            state: TabState::Loading,
            history: Vec::new(),

            hovered: None,
            pressed_dom_id: None,
        }
    }

    /// The tab's current page origin, derived from the loaded document URL.
    ///
    /// Opaque until a document has been fetched, so pages backed by internal
    /// schemes (e.g. the DevTools page) are treated as non-network origins.
    fn page_origin(&self) -> Origin {
        self.document_url
            .as_ref()
            .map(Origin::from_url)
            .unwrap_or_else(Origin::opaque)
    }

    /// Whether a `fetch()`/`XMLHttpRequest` response may be read by the given
    /// initiator.
    ///
    /// Internal (opaque) initiators may always read responses; web origins are
    /// restricted to same-origin responses and cross-origin responses that opt
    /// in via `Access-Control-Allow-Origin`. Responses targeted at internal
    /// schemes are exempt here because the resource loader already prevented
    /// web origins from ever receiving them.
    fn may_read_fetch_response(
        &self,
        initiator: &Origin,
        url: &Url,
        headers: &[(String, String)],
    ) -> bool {
        if !initiator.is_network() {
            return true;
        }
        match url.scheme() {
            "http" | "https" => {
                let response_origin = Origin::from_url_string(url.as_str());
                initiator.same_origin(&response_origin) || headers_allow_cors(headers, initiator)
            }
            _ => true,
        }
    }

    /// Tab 内の状態を 1 ステップ進める
    ///
    /// - WebView.tick() を呼び出す
    /// - 発生した Task を BrowserApp に返す
    pub fn tick(&mut self) -> Vec<TabTask> {
        let mut tasks = Vec::new();
        let page_origin = self.page_origin();
        let Some(wv) = self.webview.as_mut() else {
            return tasks;
        };

        for task in wv.tick() {
            match task {
                WebViewTask::Fetch { url, kind } => {
                    log::info!("Fetch requested in Tab: url={}", url);
                    tasks.push(TabTask::Fetch {
                        url,
                        kind,
                        origin: page_origin.clone(),
                    });
                }
                WebViewTask::AskTabHtml => {
                    tasks.push(TabTask::Fetch {
                        url: self.document_url.as_ref().unwrap().clone(),
                        kind: FetchKind::Html,
                        origin: page_origin.clone(),
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

    /// Delivers the result of a resource fetch to this tab.
    ///
    /// This is shared by regular browser tabs and the DevTools tab embedded in
    /// the browser chrome so both follow the same loading and error handling
    /// path.
    pub(crate) fn deliver_fetch(
        &mut self,
        kind: FetchKind,
        url: Url,
        response: Result<BrowserResponse, BrowserNetworkError>,
    ) {
        match response {
            Ok(resp) => match kind {
                FetchKind::Html => {
                    let html = String::from_utf8_lossy(&resp.body).to_string();
                    self.on_fetch_succeeded_html(html);
                }
                FetchKind::Css => {
                    let css = String::from_utf8_lossy(&resp.body).to_string();
                    self.on_fetch_succeeded_css_from(css, &url);
                }
                FetchKind::Script { index } => {
                    let source = String::from_utf8_lossy(&resp.body).to_string();
                    self.on_fetch_succeeded_script(index, source);
                }
                FetchKind::DynamicScript { node_id } => {
                    let source = String::from_utf8_lossy(&resp.body).to_string();
                    self.on_fetch_succeeded_dynamic_script(node_id, source);
                }
                FetchKind::DynamicCss { node_id } => {
                    let source = String::from_utf8_lossy(&resp.body).to_string();
                    self.on_fetch_succeeded_dynamic_style(node_id, source);
                }
                FetchKind::Image { source } => {
                    self.on_fetch_succeeded_image(source, &resp.body);
                }
                FetchKind::Audio { source } => {
                    self.on_fetch_succeeded_audio(source, &resp.body);
                }
                FetchKind::Iframe { dom_id } => {
                    let html = String::from_utf8_lossy(&resp.body).to_string();
                    self.on_fetch_succeeded_iframe(dom_id, html);
                }
                FetchKind::JavaScript { request_id, .. } => {
                    let initiator = self.page_origin();
                    if self.may_read_fetch_response(&initiator, &url, &resp.headers) {
                        let redirected = resp.url != url.as_str();
                        self.on_fetch_succeeded_js(request_id, resp, redirected);
                    } else {
                        log::warn!(
                            "Blocked CORS read of {url} from {}",
                            initiator.ascii_serialization()
                        );
                        self.on_fetch_failed_js(
                            request_id,
                            "Cross-origin response blocked by CORS policy".to_string(),
                        );
                    }
                }
            },
            Err(err) => match kind {
                FetchKind::Image { .. } | FetchKind::Audio { .. } => {
                    log::warn!("Media fetch failed without aborting page load: {url}");
                }
                FetchKind::Iframe { dom_id } => {
                    log::warn!("Iframe fetch failed without aborting page load: {url}");
                    self.on_fetch_failed_iframe(dom_id);
                }
                FetchKind::Script { index } => {
                    log::warn!("Classic script fetch failed without aborting page load: {url}");
                    self.on_fetch_failed_script(index);
                }
                FetchKind::DynamicScript { node_id } => {
                    log::warn!("Dynamic script fetch failed without aborting page load: {url}");
                    self.on_fetch_failed_dynamic_script(node_id);
                }
                FetchKind::DynamicCss { node_id } => {
                    log::warn!("Dynamic stylesheet fetch failed without aborting page load: {url}");
                    self.on_fetch_failed_dynamic_style(node_id);
                }
                FetchKind::JavaScript { request_id, .. } => {
                    self.on_fetch_failed_js(request_id, err.to_string());
                }
                FetchKind::Html | FetchKind::Css => self.on_fetch_failed(err, url),
            },
        }
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

        wv.on_html_fetched(html, self.document_url.as_ref().unwrap().clone());
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

    /// Installs fetched HTML as an `<iframe>` element's `contentDocument`.
    pub fn on_fetch_succeeded_iframe(&mut self, dom_id: u64, html: String) {
        if let Some(webview) = self.webview.as_mut() {
            webview.on_iframe_fetched(dom_id, html);
        }
    }

    /// Marks an iframe load as failed so later `contentDocument` accesses do
    /// not keep re-queuing a fetch.
    pub fn on_fetch_failed_iframe(&mut self, dom_id: u64) {
        if let Some(webview) = self.webview.as_mut() {
            webview.on_iframe_fetch_failed(dom_id);
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

    pub fn draw(&mut self, cmd_buf: &mut Vec<DrawCommand>, width: f32, height: f32) {
        self.relayout((width, height));

        if let Some((layout, info)) = self.layout_and_info() {
            renderer_model::generate_draw_commands(cmd_buf, layout, info, (width, height));
            self.clear_redraw_flag();
        } else {
            log::debug!(target: "Tab", "No layout/info available for tab");
        }
    }

    pub fn handle_mouse_input(&mut self, px: f32, py: f32, state: ElementState) -> (bool, bool) {
        // Hit-test the content area, dispatch the pointer event to custom
        // nodes, and remember which DOM element the press/release landed on.
        let Some((_, info)) = self.layout_and_info() else {
            return (false, false);
        };

        let path = self.hit_test(px, py);
        let dom_id = crate::engine::input::hit_dom_id(&path);

        let mut repaint = false;

        match state {
            ElementState::Pressed => {
                crate::engine::input::dismiss_open_popups(info, &path);

                let input_target = path.iter().find_map(|hit| {
                    if let layouter::types::NodeKind::Custom { node, .. } = &hit.info.kind
                        && node.accepts_text_input()
                    {
                        Some(Arc::clone(node))
                    } else {
                        None
                    }
                });

                let input_focused =
                    crate::engine::input::focus_text_input(info, input_target.as_ref());

                crate::engine::input::dispatch_pointer(&path, PointerEvent::Down { x: px, y: py });

                if let Some(href) = path.iter().find_map(|hit| {
                    if let layouter::types::NodeKind::Container { role, .. } = &hit.info.kind
                        && let layouter::types::ContainerRole::Link { href } = role
                    {
                        Some(href.clone())
                    } else {
                        None
                    }
                }) {
                    self.move_to(&href);
                }

                repaint |= input_focused;

                self.pressed_dom_id = dom_id;
            }

            ElementState::Released => {
                crate::engine::input::dispatch_pointer(&path, PointerEvent::Up { x: px, y: py });

                let pressed = self.pressed_dom_id.take();
                if let (Some(pressed), Some(released)) = (pressed, dom_id)
                    && pressed == released
                {
                    repaint |= self.on_js_click(released);
                }
            }
        }

        let (move_repaint, move_focused) = self.handle_pointer_move(px, py);
        repaint |= move_repaint;

        (repaint, move_focused)
    }

    /// Dispatches a pointer move and updates hover state without touching
    /// click bookkeeping. Unlike [`Tab::handle_mouse_input`] it never
    /// synthesizes press/release events, so it is safe to call on every
    /// cursor movement.
    pub fn handle_pointer_move(&mut self, px: f32, py: f32) -> (bool, bool) {
        let move_path = self.hit_test(px, py);

        let input_focused =
            crate::engine::input::dispatch_pointer(&move_path, PointerEvent::Move { x: px, y: py });

        let hover_changed = {
            let previous = self.hovered.as_ref();
            crate::engine::input::update_hover(&move_path, previous)
        };

        let mut repaint = false;
        if hover_changed {
            repaint = true;
            self.hovered = crate::engine::input::hit_custom_node(&move_path).cloned();
        }

        (repaint, input_focused)
    }

    /// Whether a press is waiting for its release to complete a click.
    #[cfg(test)]
    pub(crate) fn has_pending_press(&self) -> bool {
        self.pressed_dom_id.is_some()
    }

    /// Sends a text-input event (key, insert, composition) to the focused
    /// text input, if one exists.
    pub fn dispatch_text_input(&self, event: InputTextEvent) -> bool {
        let Some((_, info)) = self.layout_and_info() else {
            return false;
        };
        crate::engine::input::dispatch_text_input(info, event)
    }

    /// Defocuses any focused text input.
    pub fn defocus_text_input(&self) -> bool {
        let Some((_, info)) = self.layout_and_info() else {
            return false;
        };
        crate::engine::input::focus_text_input(info, None)
    }

    /// Returns the href of the link under the given page coordinates, if any.
    pub fn link_at(&self, px: f32, py: f32) -> Option<String> {
        let (layout, info) = self.layout_and_info()?;
        let path = crate::engine::input::hit_test(layout, info, px, py);
        path.iter().find_map(|hit| {
            if let NodeKind::Container {
                role: ContainerRole::Link { href },
                ..
            } = &hit.info.kind
            {
                Some(href.clone())
            } else {
                None
            }
        })
    }

    /// Scrolls the scrollable container under the cursor.
    pub fn scroll_at(&mut self, px: f32, py: f32, dx: f32, dy: f32, viewport: (f32, f32)) {
        let Some(scrolled_id) = self.layout_and_info_mut().and_then(|(layout, info)| {
            crate::engine::input::scroll_at(layout, info, viewport, px, py, dx, dy)
        }) else {
            return;
        };
        if scrolled_id != crate::engine::input::NO_SCROLL_DOM_ID
            && let Some(wv) = self.webview.as_mut()
        {
            wv.on_js_scroll(scrolled_id);
        }
    }

    /// Whether this tab has a layout tree ready for drawing or hit-testing.
    pub fn has_layout(&self) -> bool {
        self.layout_and_info().is_some()
    }

    /// Whether the currently focused text input is in the middle of a
    /// composition (e.g. IME preedit).
    pub fn is_text_input_composing(&self) -> bool {
        let Some((_, info)) = self.layout_and_info() else {
            return false;
        };
        crate::engine::input::focused_text_input_is_composing(info)
    }

    fn hit_test<'a>(&'a self, px: f32, py: f32) -> Vec<HitItem<'a>> {
        let Some((layout, info)) = self.layout_and_info() else {
            return vec![];
        };

        crate::engine::input::hit_test(layout, info, px, py)
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
        if let Some(url) = self.document_url.clone() {
            self.navigate_internal(url, false);
        }
    }

    /// Returns whether the back button can navigate to a previous page.
    pub fn can_go_back(&self) -> bool {
        !self.history.is_empty()
    }

    fn navigate_internal(&mut self, url: Url, record_history: bool) {
        if record_history
            && self.document_url.as_ref() != Some(&url)
            && let Some(previous) = self.document_url.clone()
        {
            self.history.push(previous);
        }
        self.document_url = Some(url);
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
    pub(crate) fn layout_and_info_mut(&mut self) -> Option<(&LayoutNode, &mut InfoNode)> {
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
        self.document_url.clone()
    }

    pub(crate) fn layout_and_info(&self) -> Option<(&LayoutNode, &InfoNode)> {
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

/// Whether the response headers allow `initiator` to read the response.
///
/// Credentials are not tracked, so a wildcard `*` grants access exactly as an
/// explicit origin would.
fn headers_allow_cors(headers: &[(String, String)], initiator: &Origin) -> bool {
    let serialized = initiator.ascii_serialization();
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("access-control-allow-origin"))
        .is_some_and(|(_, value)| value == "*" || value == &serialized)
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

    #[test]
    fn pointer_move_between_press_and_release_keeps_click_pending() {
        let mut tab = Tab::default();
        tab.navigate(url("https://example.test/a"));
        tab.on_fetch_succeeded_html("<html><body><p>click me</p></body></html>".to_string());

        // Force a relayout, wait for the background layout thread, then draw
        // again so the applied tree gets its boxes positioned.
        let mut buf = Vec::new();
        tab.draw(&mut buf, 800.0, 600.0);
        for _ in 0..500 {
            for _ in tab.tick() {}
            if tab.layout_and_info().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        tab.draw(&mut buf, 800.0, 600.0);

        // Press on the page, move before releasing: the move must not
        // complete or cancel the in-flight press.
        tab.handle_mouse_input(400.0, 12.0, ElementState::Pressed);
        assert!(tab.has_pending_press(), "press should land on an element");

        tab.handle_pointer_move(401.0, 13.0);
        assert!(
            tab.has_pending_press(),
            "a pointer move must not cancel an in-flight press"
        );

        // The release completes the click and clears the pending press.
        tab.handle_mouse_input(401.0, 13.0, ElementState::Released);
        assert!(!tab.has_pending_press());
    }

    #[test]
    fn cross_origin_fetch_requires_matching_cors_header() {
        let tab = Tab::default();
        let web_origin = Origin::from_url_string("https://example.test/index.html");
        let url = Url::parse("https://other.test/data.json").unwrap();

        assert!(!tab.may_read_fetch_response(&web_origin, &url, &[]));

        let wildcard = vec![("Access-Control-Allow-Origin".to_string(), "*".to_string())];
        assert!(tab.may_read_fetch_response(&web_origin, &url, &wildcard));

        let matching = vec![(
            "Access-Control-Allow-Origin".to_string(),
            "https://example.test".to_string(),
        )];
        assert!(tab.may_read_fetch_response(&web_origin, &url, &matching));

        let other = vec![(
            "Access-Control-Allow-Origin".to_string(),
            "https://other.test".to_string(),
        )];
        assert!(!tab.may_read_fetch_response(&web_origin, &url, &other));
    }

    #[test]
    fn same_origin_fetch_is_readable_without_cors_header() {
        let tab = Tab::default();
        let web_origin = Origin::from_url_string("https://example.test/");
        let url = Url::parse("https://example.test:443/api").unwrap();
        assert!(tab.may_read_fetch_response(&web_origin, &url, &[]));
    }

    #[test]
    fn internal_page_reads_any_response_without_cors() {
        let tab = Tab::default();
        let internal = Origin::opaque();
        let url = Url::parse("https://other.test/api").unwrap();
        assert!(tab.may_read_fetch_response(&internal, &url, &[]));
    }

    #[test]
    fn web_page_reads_internal_scheme_responses_without_cors() {
        // The resource loader refuses to serve internal schemes to web origins,
        // so such responses never reach this check in practice.
        let tab = Tab::default();
        let web_origin = Origin::from_url_string("https://example.test/");
        let url = Url::parse("data:text/plain,hello").unwrap();
        assert!(tab.may_read_fetch_response(&web_origin, &url, &[]));
    }

    #[test]
    fn headers_allow_cors_requires_exact_origin_value() {
        let initiator = Origin::from_url_string("https://example.test/");
        assert!(headers_allow_cors(
            &[("Access-Control-Allow-Origin".to_string(), "*".to_string())],
            &initiator
        ));
        assert!(headers_allow_cors(
            &[(
                "ACCEss-cOntRoL-aLLow-orIgIn".to_string(),
                "https://example.test".to_string()
            )],
            &initiator
        ));
        assert!(!headers_allow_cors(
            &[(
                "Access-Control-Allow-Origin".to_string(),
                "https://other.test".to_string()
            )],
            &initiator
        ));
    }
}
