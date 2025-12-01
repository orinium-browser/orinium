use std::sync::Arc;

use super::tab::Tab;
// use super::ui::init_browser_ui;

use crate::platform::network::NetworkCore;

use crate::engine::renderer::{DrawCommand, RenderTree, Renderer};
use crate::platform::renderer::gpu::GpuRenderer;
use crate::system::App;

use anyhow::Result;
use winit::event::WindowEvent;

pub enum BrowserCommand {
    Exit,
    RenameWindowTitle,
    RequestRedraw,
    None,
}

/// BrowserApp はブラウザ全体のアプリケーション状態を管理する構造体です。
///
/// 主な役割:
/// - 複数のタブ(Tab)の管理
/// - アクティブタブの切り替え
/// - 各種 UI イベントの受け取りと WebView への伝達
/// - エンジン更新処理（レイアウト・描画・ネットワークなど）を統合的に呼び出す
///
/// アプリケーションの「外側の枠組み」を担当し、
/// ブラウザ起動 → イベントループ → 描画の流れを制御します。
///
/// TODO:
/// - ネットワーク機構の実装
pub struct BrowserApp {
    tabs: Vec<Tab>,
    // render_tree: RenderTree,
    draw_commands: Vec<DrawCommand>,
    window_size: (u32, u32), // (x, y)
    window_title: String,
    #[allow(unused)]
    network: Arc<NetworkCore>,
}

impl Default for BrowserApp {
    fn default() -> Self {
        Self::new((800, 600), "Orinium Browser".to_string())
    }
}

impl BrowserApp {
    /// ブラウザのメインループを開始
    pub fn run(self) -> Result<()> {
        let event_loop =
            winit::event_loop::EventLoop::<crate::platform::system::State>::with_user_event()
                .build()?;

        let mut app = App::new(self);

        event_loop.run_app(&mut app)?;

        Ok(())
    }

    pub fn new(window_size: (u32, u32), window_title: String) -> Self {
        //let (render_tree, draw_commands) = init_browser_ui(window_size);
        let network = Arc::new(NetworkCore::new());
        Self {
            tabs: vec![],
            // render_tree,
            draw_commands: vec![],
            window_size,
            window_title,
            network,
        }
    }

    // 開発テスト用
    pub fn with_draw_info(
        mut self,
        _render_tree: RenderTree,
        draw_commands: Vec<DrawCommand>,
    ) -> Self {
        // self.render_tree = render_tree;
        self.draw_commands = draw_commands;
        self
    }

    pub fn add_tab(&mut self, tab: Tab) {
        self.tabs.push(tab);
    }

    fn build_from_tabs(&mut self) {
        if let Some(active) = self.tabs.first() {
            let tree = active.render_tree().unwrap();
            let renderer = Renderer::new();
            self.draw_commands = renderer.generate_draw_commands(tree);

            let title = active.title();
            if let Some(t) = title
                && !t.is_empty()
            {
                self.window_title = t;
            } else if let Some(url) = active.url()
                && !url.is_empty()
            {
                self.window_title = url;
            }
        }
    }

    pub fn apply_draw_commands(&self, gpu: &mut GpuRenderer) {
        gpu.parse_draw_commands(&self.draw_commands);
    }

    /// ウィンドウイベントの処理
    pub fn handle_window_event(
        &mut self,
        event: WindowEvent,
        gpu: &mut GpuRenderer,
    ) -> BrowserCommand {
        match event {
            WindowEvent::CloseRequested => BrowserCommand::Exit,

            WindowEvent::RedrawRequested => {
                self.build_from_tabs();
                self.apply_draw_commands(gpu);

                // Ok(animationg)
                if let Ok(true) = gpu.render() {
                    self.apply_draw_commands(gpu);
                }
                BrowserCommand::RenameWindowTitle
            }

            WindowEvent::Resized(pysical_size) => {
                let width = pysical_size.width;
                let height = pysical_size.height;
                self.window_size = (width, height);
                gpu.resize(pysical_size);

                BrowserCommand::None
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                gpu.set_scale_factor(scale_factor);

                BrowserCommand::None
            }

            /*
            WindowEvent::MouseWheel { delta, .. } => {
                let scroll_amount = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => -y * 60.0,
                    winit::event::MouseScrollDelta::PixelDelta(pos) => -pos.y as f32,
                };
                // TODO: スクロール対象のタブ/レンダーツリーに反映
                self.apply_draw_commands(gpu);
                BrowserCommand::RequestRedraw
            }
            */
            _ => BrowserCommand::None,
        }
    }

    pub fn window_size(&self) -> (u32, u32) {
        self.window_size
    }

    pub fn window_title(&self) -> String {
        self.window_title.clone()
    }

    pub fn network(&self) -> Arc<NetworkCore> {
        self.network.clone()
    }
}
