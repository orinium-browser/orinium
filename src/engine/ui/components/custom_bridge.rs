//! Bridge between the layout engine's [`BlockLayouter`] trait and
//! the engine's [`CustomNode`] rendering interface.
//!
//! [`BlockLayouter`] (from `ui_layout`) handles layout positioning:
//! it reports the node's border-box rect to the layout engine.
//!
//! The corresponding [`CustomNode`](super::super::custom_node::CustomNode)
//! (stored in [`NodeKind::Custom`](super::super::layouter::types::NodeKind))
//! handles draw-command generation.

use std::rc::Rc;

use ui_layout::{BlockLayouter, LayoutContext, LengthOrAuto, Rect, Style};

use crate::engine::layouter::types::ContainerStyle;

use super::super::custom_node::CustomNode;

/// A [`BlockLayouter`] implementation for custom nodes.
///
/// Wraps a [`ContainerStyle`] so the layout engine can account for
/// borders and padding when sizing the element.  The actual content
/// rendering is delegated to a separate [`CustomNode`](super::super::custom_node::CustomNode).
#[derive(Debug)]
pub struct CustomLayoutBridge {
    style: ContainerStyle,
    layout_style: Style,
    node: Rc<dyn CustomNode>,
}

impl CustomLayoutBridge {
    pub fn new(style: ContainerStyle, layout_style: Style, node: Rc<dyn CustomNode>) -> Self {
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
        let (intrinsic_w, intrinsic_h) = self.node.intrinsic_size();

        let width = match &self.layout_style.size.width {
            LengthOrAuto::Length(len) => len
                .resolve_with(
                    ctx.containing_block_width,
                    ctx.containing_block_width.unwrap_or(0.0),
                    ctx.containing_block_height.unwrap_or(0.0),
                )
                .unwrap_or(intrinsic_w),
            LengthOrAuto::Auto => intrinsic_w,
        };

        let height = match &self.layout_style.size.height {
            LengthOrAuto::Length(len) => len
                .resolve_with(
                    ctx.containing_block_height,
                    ctx.containing_block_width.unwrap_or(0.0),
                    ctx.containing_block_height.unwrap_or(0.0),
                )
                .unwrap_or(intrinsic_h),
            LengthOrAuto::Auto => intrinsic_h,
        };

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
