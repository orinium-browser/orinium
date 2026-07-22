use ui_layout::{BlockLayouter, LayoutContext, LayoutEngine, LayoutNode, Rect};

use crate::engine::layouter::types::ContainerStyle;

pub struct ContainerComp {
    style: ContainerStyle,
    scroll_x: bool,
    scroll_y: bool,
    scroll_offset_x: f32,
    scroll_offset_y: f32,
    node: LayoutNode,
}

impl ContainerComp {
    pub fn new(style: ContainerStyle, scroll_x: bool, scroll_y: bool, node: LayoutNode) -> Self {
        Self {
            style,
            scroll_x,
            scroll_y,
            scroll_offset_x: 0.0,
            scroll_offset_y: 0.0,
            node,
        }
    }

    pub fn scroll_offset(&self) -> (f32, f32) {
        (self.scroll_offset_x, self.scroll_offset_y)
    }
}

impl std::fmt::Debug for ContainerComp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContainerComp")
            .field("style", &self.style)
            .field("scroll", &(self.scroll_x, self.scroll_y))
            .finish_non_exhaustive()
    }
}

impl BlockLayouter for ContainerComp {
    fn layout(&mut self, ctx: &LayoutContext) -> Rect {
        let width = ctx.containing_block_width.unwrap_or(0.0);
        let height = ctx.containing_block_height.unwrap_or(0.0);

        LayoutEngine::layout(&mut self.node, width, height);

        let rect = match &self.node.layout_box {
            ui_layout::LayoutBox::BlockBox(bm) => bm.border_box,
            _ => Rect::default(),
        };

        rect
    }

    fn write_debug(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Container")
    }
}
