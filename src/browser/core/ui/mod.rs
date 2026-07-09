//! Browser UI components.

use std::fmt;
use ui_layout::{CustomLayout, LayoutContext, MeasureResult, Rect};

use crate::browser::Tab;
use crate::engine::renderer_model::{self, DrawCommand};
use crate::engine::ui::{UiComponent, UiEvent};

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

impl CustomLayout for BrowserUi {
    fn layout(&self, ctx: &LayoutContext) -> Rect {
        let width = ctx
            .parent_assigned_border_width
            .or(ctx.available_width)
            .or(ctx.containing_block_width)
            .unwrap_or(800.0);
        let height = ctx
            .parent_assigned_border_height
            .or(ctx.containing_block_height)
            .unwrap_or(30.0);
        Rect {
            x: 0.0,
            y: 0.0,
            width,
            height,
        }
    }

    fn measure(&self, _ctx: &LayoutContext) -> MeasureResult {
        MeasureResult {
            width: 0.0,
            height: 30.0,
        }
    }

    fn write_debug(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BrowserUi(tabs={})", self.tabs.len())
    }
}

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
