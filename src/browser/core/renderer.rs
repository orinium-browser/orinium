//! ブラウザの描画機能。タブと chrome の描画リクエストを DrawCommand に変換し、
//! プラットフォームの GPU レンダラへ送る。

use crate::engine::renderer_model;
use crate::engine::renderer_model::{AffineTransform, DrawCommand, FillRule, rect_path};
use crate::platform::renderer::gpu::GpuRenderer;

use super::tab::Tab;
use super::ui::{BrowserLayout, RenderState};

/// BrowserRenderer は実際の描画を担当する。
///
/// 責務:
/// - アクティブタブのページとブラウザ chrome から DrawCommand を生成する
/// - DrawCommand をプラットフォームの GPU レンダラへ渡し、描画を実行する
/// - ウィンドウのサイズ・スケール・タイトルなどの描画状態を保持する
///
/// BrowserApp / BrowserUi は直接の描画実装を持たず、このレンダラへ処理を委譲する。
#[derive(Debug)]
pub struct BrowserRenderer {
    /// ウィンドウの描画状態（DrawCommand、サイズ、スケール、タイトル）。
    pub render_state: RenderState,
    /// ブラウザ chrome（ツールバーなどの UI 描画）。
    pub layout: BrowserLayout,
}

impl Default for BrowserRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserRenderer {
    pub fn new() -> Self {
        Self {
            render_state: RenderState::default(),
            layout: BrowserLayout::new(),
        }
    }

    /// ウィンドウの初期サイズ・スケール・タイトルでレンダラを生成する。
    pub fn with_window(window_size: (u32, u32), scale_factor: f64, window_title: String) -> Self {
        Self {
            render_state: RenderState::new(window_size, scale_factor, window_title),
            layout: BrowserLayout::new(),
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
    pub fn rebuild(&mut self, tabs: &mut [Tab], active_tab: usize) {
        let (width, height) = self.render_state.viewport();
        let toolbar_height = self.layout.toolbar_rects(width).height();
        let content_height = (height - toolbar_height).max(0.0);

        // Reuse allocation
        let mut draw_commands = std::mem::take(&mut self.render_state.draw_commands);
        draw_commands.clear();

        // Page area: below the toolbar, clipped so page content never overlaps
        // the chrome.
        draw_commands.push(DrawCommand::PushClip {
            path: rect_path(0.0, toolbar_height, width, content_height),
            rule: FillRule::NonZero,
        });
        draw_commands.push(DrawCommand::PushTransform {
            transform: AffineTransform::translate(0.0, toolbar_height),
        });

        let title = if let Some(tab) = tabs.get_mut(active_tab) {
            tab.relayout((width, content_height));

            // Keep the address bar in sync with the active tab.
            let url = tab.document_url().map(|url| url.to_string());
            self.layout.sync_url(url.as_deref());

            if let Some((layout, info)) = tab.layout_and_info() {
                renderer_model::generate_draw_commands(&mut draw_commands, layout, info);
                tab.title()
            } else {
                log::debug!("No layout/info available for tab {}", active_tab);
                None
            }
        } else {
            None
        };

        draw_commands.push(DrawCommand::PopTransform);
        draw_commands.push(DrawCommand::PopClip);

        // Chrome toolbar drawn on top of the page area.
        self.layout.draw_chrome(&mut draw_commands, width);

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
    pub fn redraw(&mut self, tabs: &mut [Tab], active_tab: usize, gpu: &mut GpuRenderer) {
        self.rebuild(tabs, active_tab);
        self.apply_draw_commands(gpu);
        if let Err(e) = gpu.render() {
            log::error!(target: "BrowserRenderer::redraw", "Render error occurred: {}", e);
        }
    }
}
