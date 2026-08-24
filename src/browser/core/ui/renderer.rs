//! ブラウザの描画機能。タブと chrome の描画リクエストを DrawCommand に変換し、
//! プラットフォームの GPU レンダラへ送る。

use std::collections::HashMap;

use crate::browser::core::ui::TabId;
use crate::engine::renderer_model::{AffineTransform, DrawCommand, FillRule, Rect, rect_path};
use crate::platform::renderer::gpu::GpuRenderer;

use super::{BasicChrome, BasicContextMenu, Chrome, ContextMenu, RenderState};
use crate::browser::core::tab::Tab;

/// BrowserRenderer は実際の描画を担当する。
///
/// 責務:
/// - アクティブタブのページとブラウザ chrome から DrawCommand を生成する
/// - DevTools ペインが開いている場合は分割ビューとして両ペインを生成する
/// - 開いているコンテキストメニューを最前面オーバーレイとして生成する
/// - DrawCommand をプラットフォームの GPU レンダラへ渡し、描画を実行する
/// - ウィンドウのサイズ・スケール・タイトルなどの描画状態を保持する
///
/// BrowserApp / BrowserUi は直接の描画実装を持たず、このレンダラへ処理を委譲する。
#[derive(Debug)]
pub struct BrowserRenderer {
    /// ウィンドウの描画状態（DrawCommand、サイズ、スケール、タイトル）。
    pub render_state: RenderState,
    /// ブラウザ chrome（ツールバーなどの UI 描画）。既定は [`BasicChrome`]。
    pub chrome: Box<dyn Chrome>,
    /// WebView 右クリックで開くコンテキストメニュー。既定は [`BasicContextMenu`]。
    pub menu: Box<dyn ContextMenu>,
}

impl Default for BrowserRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserRenderer {
    /// 既定の [`BasicChrome`] と [`BasicContextMenu`] でレンダラを生成する。
    pub fn new() -> Self {
        Self::with_chrome(Box::new(BasicChrome::new()))
    }

    /// 任意の chrome 実装と既定の [`BasicContextMenu`] でレンダラを生成する。
    pub fn with_chrome(chrome: Box<dyn Chrome>) -> Self {
        Self::with_chrome_and_menu(chrome, Box::new(BasicContextMenu::new()))
    }

    /// ウィンドウの初期サイズ・スケール・タイトルでレンダラを生成する。
    pub fn with_window(window_size: (u32, u32), scale_factor: f64, window_title: String) -> Self {
        Self {
            render_state: RenderState::new(window_size, scale_factor, window_title),
            chrome: Box::new(BasicChrome::new()),
            menu: Box::new(BasicContextMenu::new()),
        }
    }

    /// 任意の chrome 実装と任意のコンテキストメニュー実装でレンダラを生成する。
    pub fn with_chrome_and_menu(chrome: Box<dyn Chrome>, menu: Box<dyn ContextMenu>) -> Self {
        Self {
            render_state: RenderState::default(),
            chrome,
            menu,
        }
    }

    /// ウィンドウのサイズ・スケール・タイトルを設定する。
    pub fn set_window(&mut self, window_size: (u32, u32), scale_factor: f64, window_title: String) {
        self.render_state.window_size = window_size;
        self.render_state.scale_factor = scale_factor;
        self.render_state.window_title = window_title;
    }

    /// 指定されたアクティブタブのレイアウトを更新し、ページと chrome の
    /// DrawCommand を再生成する。
    pub fn rebuild(&mut self, tabs: &mut HashMap<TabId, Tab>, active_id: Option<TabId>) {
        let (width, height) = self.render_state.viewport();

        let Rect {
            x,
            y,
            width: content_width,
            height: content_height,
        } = self.chrome.content_rect(width, height);

        // Reuse allocation
        let mut draw_commands = std::mem::take(&mut self.render_state.draw_commands);
        draw_commands.clear();

        // Page area: below the chrome, clipped so page content never overlaps it.
        draw_commands.push(DrawCommand::PushClip {
            path: rect_path(x, y, content_width, content_height),
            rule: FillRule::NonZero,
        });
        draw_commands.push(DrawCommand::PushTransform {
            transform: AffineTransform::translate(x, y),
        });

        let title = if let Some(active_tab) = active_id
            && let Some(tab) = tabs.get_mut(&active_tab)
        {
            // Keep the chrome in sync with the active tab.
            let url = tab.document_url().map(|url| url.to_string());
            self.chrome.sync_url(url.as_deref());

            tab.draw(&mut draw_commands, content_width, content_height);

            tab.title()
        } else {
            None
        };

        draw_commands.push(DrawCommand::PopTransform);
        draw_commands.push(DrawCommand::PopClip);

        // Chrome drawn on top of the page area.
        self.chrome.draw(&mut draw_commands, width, height);

        // The context menu is drawn last: topmost overlay above the chrome.
        self.menu.draw(&mut draw_commands, width, height);

        // Return reused buffer
        self.render_state.draw_commands = draw_commands;

        if let Some(title) = title {
            self.render_state.window_title = title;
        }
    }

    /// 現在の DrawCommand を GPU レンダラへ送る。
    pub fn apply_draw_commands(&self, gpu: &mut GpuRenderer) {
        gpu.parse_draw_commands(&self.render_state.draw_commands);
    }

    /// DrawCommand を再生成して GPU に送り、実際の描画を実行する。
    pub fn redraw(
        &mut self,
        tabs: &mut HashMap<TabId, Tab>,
        active_id: Option<TabId>,
        gpu: &mut GpuRenderer,
    ) {
        self.rebuild(tabs, active_id);
        self.apply_draw_commands(gpu);
        if let Err(e) = gpu.render() {
            log::error!(target: "BrowserRenderer::redraw", "Render error occurred: {}", e);
        }
    }
}
