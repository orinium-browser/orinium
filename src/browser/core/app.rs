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
}

pub struct BrowserApp {
    tabs: Vec<Tab>,
    active_tab: usize,

    render: RenderState,
    window_title: String,

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
            },
            window_title,
            network,
        }
    }

    fn rebuild_render_tree(&mut self) {
        let size = self.window_size();

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

        ui_layout::LayoutEngine::layout(layout, size.0, size.1);
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
                BrowserCommand::None
            }

            WindowEvent::MouseWheel { delta, .. } => {
                self.handle_scroll(delta);
                BrowserCommand::RequestRedraw
            }

            _ => BrowserCommand::None,
        }
    }

    fn handle_scroll(&mut self, delta: winit::event::MouseScrollDelta) {
        let _scroll_amount = match delta {
            winit::event::MouseScrollDelta::LineDelta(_, y) => -y * 60.0,
            winit::event::MouseScrollDelta::PixelDelta(pos) => -pos.y as f32,
        };
    }

    pub fn redraw(&mut self, gpu: &mut GpuRenderer) {
        self.rebuild_render_tree();
        self.apply_draw_commands(gpu);
        // Ok(animationg)
        if let Ok(true) = gpu.render() {
            self.apply_draw_commands(gpu);
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
}
