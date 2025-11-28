use super::tab::Tab;
// use super::ui::init_browser_ui;

use crate::engine::renderer::{DrawCommand, RenderTree};
use crate::platform::renderer::gpu::GpuRenderer;
use crate::system::App;

use anyhow::Result;
use winit::event::WindowEvent;

pub enum BrowserCommand {
    Exit,
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
/// - tabsの実装
pub struct BrowserApp {
    #[allow(unused)]
    tabs: Vec<Tab>,
    // render_tree: RenderTree,
    draw_commands: Vec<DrawCommand>,
    window_size: (u32, u32), // (x, y)
    window_title: String,
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
        Self {
            tabs: vec![],
            // render_tree,
            draw_commands: vec![],
            window_size,
            window_title,
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
                if let Ok(animating) = gpu.render() {
                    if animating {
                        self.apply_draw_commands(gpu);
                    }
                }
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
}
