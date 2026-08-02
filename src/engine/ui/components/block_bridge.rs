//! Block-level layout bridge for custom nodes.

use ui_layout::{BlockLayouter, LayoutContext, Rect};

use crate::engine::layouter::types::ContainerStyle;

use super::super::custom_node::CustomNode;
use super::inline_cache::resolve_border_box_size;

/// A [`BlockLayouter`] implementation for custom nodes.
///
/// Wraps a [`ContainerStyle`] so the layout engine can account for
/// borders and padding when sizing the element.  The actual content
/// rendering is delegated to a separate [`CustomNode`](super::super::custom_node::CustomNode).
#[derive(Debug)]
pub struct CustomLayoutBridge {
    style: ContainerStyle,
    layout_style: ui_layout::Style,
    node: std::rc::Rc<dyn CustomNode>,
}

impl CustomLayoutBridge {
    pub fn new(
        style: ContainerStyle,
        layout_style: ui_layout::Style,
        node: std::rc::Rc<dyn CustomNode>,
    ) -> Self {
        Self {
            style,
            layout_style,
            node,
        }
    }

    pub fn style(&self) -> &ContainerStyle {
        &self.style
    }
}

impl BlockLayouter for CustomLayoutBridge {
    fn layout(&mut self, ctx: &LayoutContext) -> Rect {
        let viewport_width = ctx.containing_block_width.unwrap_or(0.0);
        let viewport_height = ctx.containing_block_height.unwrap_or(0.0);
        let (width, height) = resolve_border_box_size(
            self.node.as_ref(),
            &self.layout_style,
            ctx.containing_block_width,
            ctx.containing_block_height,
            viewport_width,
            viewport_height,
        );

        Rect {
            x: 0.0,
            y: 0.0,
            width,
            height,
        }
    }

    fn write_debug(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CustomLayoutBridge")
    }
}
