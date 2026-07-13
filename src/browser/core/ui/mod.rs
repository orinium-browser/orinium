//! Browser UI components.

use crate::browser::Tab;
// use crate::engine::renderer_model::{self, DrawCommand};
// use crate::engine::ui::{UiComponent, UiEvent};

#[derive(Debug)]
pub struct BrowserUi {
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
}

impl BrowserUi {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active_tab: 0,
        }
    }

    pub fn with_tab(tab: Tab) -> Self {
        Self {
            tabs: vec![tab],
            active_tab: 0,
        }
    }
}

/*
impl UiComponent for BrowserUi {
    fn receive_event(&self, event: UiEvent) {
        match event {
            UiEvent::Scroll { dx, dy } => {
                log::info!("BrowserUi scroll: dx={}, dy={}", dx, dy);
            }
        }
    }

    fn draw(&self, cmd_buf: &mut Vec<DrawCommand>) {
        let Some(tab) = self.tabs.get(self.active_tab) else {
            return;
        };
        let Some((layout, info)) = tab.layout_and_info() else {
            return;
        };
        renderer_model::generate_draw_commands(cmd_buf, layout, info);
    }
}
*/
