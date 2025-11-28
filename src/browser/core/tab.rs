use crate::renderer::RenderTree;

use super::webview::WebView;

pub struct Tab {
    title: Option<String>,
    webview: Option<WebView>
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
            webview: None
        }
    }
    pub fn load_from_raw_html(&mut self, html_source: &str) {
        let mut view = WebView::new();
        view.load(html_source, None);
        self.title = view.title.clone();

        self.webview = Some(view);
    }

    pub fn render_tree(&self) -> Option<&RenderTree> {
        self.webview
            .as_ref()
            .and_then(|wv| wv.render.as_ref())
    }
}
