use anyhow::Result;
use std::sync::Arc;
use winit::event::WindowEvent;

use super::tab::Tab;
// use super::ui::init_browser_ui;

use super::BrowserCommand;
use super::resource_loader::BrowserResourceLoader;
use crate::engine::layouter::{self, DrawCommand};
use crate::platform::network::NetworkCore;
use crate::platform::renderer::gpu::GpuRenderer;
use crate::system::App;

pub struct RenderState {
    pub draw_commands: Vec<DrawCommand>,
    pub window_size: (u32, u32),
    pub scale_factor: f64,
}

#[derive(Default)]
pub struct InputState {
    // Add fields to track input state, e.g., mouse position, pressed keys, etc.
    pub mouse_position: (f64, f64),
}

pub struct BrowserApp {
    tabs: Vec<Tab>,
    active_tab: usize,

    render: RenderState,
    window_title: String,

    input: InputState,

    network: Arc<BrowserResourceLoader>,
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
        let network = Arc::new(BrowserResourceLoader::new(Some(Arc::new(
            NetworkCore::new(),
        ))));

        Self {
            tabs: vec![],
            active_tab: 0,
            render: RenderState {
                draw_commands: vec![],
                window_size,
                scale_factor: 1.0,
            },
            window_title,
            network,
            input: InputState::default(),
        }
    }

    fn rebuild_render_tree(&mut self) {
        let size = self.window_size();
        let sf = self.render.scale_factor as f32;

        let (layout, info, title) = {
            let Some(tab) = self.active_tab_mut() else {
                return;
            };

            let title = if let Some(t) = tab.title().filter(|t| !t.is_empty()) {
                Some(t)
            } else {
                tab.url().filter(|u| !u.is_empty())
            };
            let (layout, info) = tab.layout_and_info().unwrap();

            (layout, info, title)
        };

        ui_layout::LayoutEngine::layout(layout, size.0 / sf, size.1 / sf);
        self.render.draw_commands = layouter::generate_draw_commands(layout, info);

        if let Some(t) = title {
            self.window_title = t;
        }
    }

    fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active_tab)
    }

    pub fn handle_window_event(
        &mut self,
        event: WindowEvent,
        gpu: &mut GpuRenderer,
    ) -> BrowserCommand {
        match event {
            WindowEvent::CloseRequested => BrowserCommand::Exit,

            WindowEvent::RedrawRequested => {
                self.redraw(gpu);
                BrowserCommand::RenameWindowTitle
            }

            WindowEvent::Resized(size) => {
                self.render.window_size = (size.width, size.height);
                gpu.resize(size);
                self.redraw(gpu);
                BrowserCommand::RequestRedraw
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                gpu.set_scale_factor(scale_factor);
                self.render.scale_factor = scale_factor;
                self.redraw(gpu);
                BrowserCommand::RequestRedraw
            }

            WindowEvent::MouseWheel { delta, .. } => {
                self.handle_scroll(delta);
                BrowserCommand::RequestRedraw
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.input.mouse_position = (position.x, position.y);
                BrowserCommand::None
            }

            WindowEvent::MouseInput { button, .. } => match button {
                winit::event::MouseButton::Left => {
                    let (x, y) = self.input.mouse_position;
                    let sf = self.render.scale_factor;
                    if let Some(tab) = self.active_tab_mut() {
                        Self::handle_mouse_click(tab, (x / sf) as f32, (y / sf) as f32);
                        BrowserCommand::RequestRedraw
                    } else {
                        BrowserCommand::None
                    }
                }
                _ => BrowserCommand::None,
            },

            _ => BrowserCommand::None,
        }
    }

    fn handle_scroll(&mut self, delta: winit::event::MouseScrollDelta) {
        let scroll_amount = match delta {
            winit::event::MouseScrollDelta::LineDelta(_, y) => -y * 60.0,
            winit::event::MouseScrollDelta::PixelDelta(pos) => -pos.y as f32,
        };

        if let Some(tab) = self.active_tab_mut() {
            if let Some((_layout, info)) = tab.layout_and_info() {
                // ルートコンテナにスクロール量を加算
                if let layouter::NodeKind::Container {
                    scroll_offset_x: _,
                    scroll_offset_y,
                    ..
                } = &mut info.kind
                {
                    *scroll_offset_y += scroll_amount;

                    // 上下限のチェック（簡易）
                    *scroll_offset_y = scroll_offset_y.clamp(0.0, std::f32::MAX);
                }
            }
        }
    }

    pub fn handle_mouse_click(tab: &mut Tab, x: f32, y: f32) {
        println!("clicked");
        let hit_path = if let Some((layout, info)) = tab.layout_and_info() {
            crate::engine::input::hit_test(layout, info, x, y)
        } else {
            return;
        };
        if let Some(hit) = hit_path
            .iter()
            .find(|&e| matches!(e.info.kind, layouter::NodeKind::Link { .. }))
        {
            match &hit.info.kind {
                layouter::NodeKind::Link { href, .. } => {
                    let href = href.clone();
                    println!("リンククリック: {}", href);
                }

                _ => unreachable!(),
            }
        }
    }

    pub fn redraw(&mut self, gpu: &mut GpuRenderer) {
        self.rebuild_render_tree();
        self.apply_draw_commands(gpu);
        let render_result = gpu.render();
        if let Err(e) = render_result {
            log::error!("Render error occured: {}", e);
        }
    }

    pub fn apply_draw_commands(&self, gpu: &mut GpuRenderer) {
        gpu.parse_draw_commands(&self.render.draw_commands);
    }

    pub fn add_tab(&mut self, tab: Tab) {
        self.tabs.push(tab);
    }

    pub fn window_size(&self) -> (f32, f32) {
        (
            self.render.window_size.0 as f32,
            self.render.window_size.1 as f32,
        )
    }
    pub fn window_title(&self) -> String {
        self.window_title.to_string()
    }
    pub fn network(&self) -> Arc<BrowserResourceLoader> {
        Arc::clone(&self.network)
    }

    pub fn set_scale_factor(&mut self, sf: f64) {
        self.render.scale_factor = sf
    }
}
