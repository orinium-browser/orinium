//! Inline-level layout bridge for custom nodes.

use ui_layout::{FlowLayoutContext, FlowLayouter, LayoutContext, Style};

use crate::engine::layouter::types::ContainerStyle;
use crate::engine::ui::custom_node::{ContentSize, CustomNode};

use super::inline_cache::{
    CustomInlineResult, InlineLayoutId, get_custom_inline_result, next_custom_inline_id,
    remove_custom_inline_result, resolve_border_box_size, set_custom_inline_result,
};

/// An inline-level bridge for custom nodes implementing [`FlowLayouter`].
///
/// During layout, the engine calls [`layout`](Self::layout) which stores
/// the result in a thread-local cache keyed by [`id`](Self::id).  The
/// rendering layer later retrieves it via
/// [`get_custom_inline_result`].
#[derive(Debug)]
pub struct CustomInlineBridge {
    pub id: InlineLayoutId,
    node: std::rc::Rc<dyn CustomNode>,
    layout_style: Style,
    style: ContainerStyle,
}

impl CustomInlineBridge {
    pub fn new(
        node: std::rc::Rc<dyn CustomNode>,
        layout_style: Style,
        style: ContainerStyle,
    ) -> Self {
        Self {
            id: next_custom_inline_id(),
            node,
            layout_style,
            style,
        }
    }

    pub fn node(&self) -> &std::rc::Rc<dyn CustomNode> {
        &self.node
    }

    pub fn style(&self) -> &ContainerStyle {
        &self.style
    }

    fn resolve_size(
        &self,
        containing_width: Option<f32>,
        containing_height: Option<f32>,
        viewport_width: f32,
        viewport_height: f32,
    ) -> ContentSize {
        resolve_border_box_size(
            self.node.as_ref(),
            &self.layout_style,
            containing_width,
            containing_height,
            viewport_width,
            viewport_height,
        )
    }
}

impl Drop for CustomInlineBridge {
    fn drop(&mut self) {
        remove_custom_inline_result(self.id);
    }
}

impl FlowLayouter for CustomInlineBridge {
    fn layout(&self, ctx: &FlowLayoutContext) -> Vec<ui_layout::LineSpan> {
        let x = ctx.start_pos.0;
        let y = ctx.start_pos.1;

        let (use_width, use_height) = get_custom_inline_result(self.id).map_or_else(
            || {
                let resolved = self.resolve_size(
                    Some(ctx.available_inline_size),
                    None,
                    ctx.available_inline_size,
                    0.0,
                );
                (resolved.width, resolved.height)
            },
            |r| (r.width, r.height),
        );

        let spans = vec![ui_layout::LineSpan {
            x_range: x..(x + use_width),
            line_pos: (x, y),
            line_index: 0,
        }];

        set_custom_inline_result(
            self.id,
            CustomInlineResult {
                spans: spans.clone(),
                width: use_width,
                height: use_height,
                border_top: 0.0,
                border_right: 0.0,
                border_bottom: 0.0,
                border_left: 0.0,
                padding_top: 0.0,
                padding_right: 0.0,
                padding_bottom: 0.0,
                padding_left: 0.0,
            },
        );

        spans
    }

    fn measure(&self, ctx: &LayoutContext) -> ui_layout::MeasureResult {
        let vw = ctx.containing_block_width.unwrap_or(0.0);
        let vh = ctx.containing_block_height.unwrap_or(0.0);

        let resolved = self.resolve_size(
            ctx.containing_block_width,
            ctx.containing_block_height,
            vw,
            vh,
        );
        let css_w = resolved.width;
        let css_h = resolved.height;

        let sp = &self.layout_style.spacing;
        let b_top = sp
            .border_top
            .resolve_with(ctx.containing_block_width, vw, vh)
            .unwrap_or(0.0);
        let b_right = sp
            .border_right
            .resolve_with(ctx.containing_block_width, vw, vh)
            .unwrap_or(0.0);
        let b_bottom = sp
            .border_bottom
            .resolve_with(ctx.containing_block_width, vw, vh)
            .unwrap_or(0.0);
        let b_left = sp
            .border_left
            .resolve_with(ctx.containing_block_width, vw, vh)
            .unwrap_or(0.0);
        let p_top = sp
            .padding_top
            .resolve_with(ctx.containing_block_width, vw, vh)
            .unwrap_or(0.0);
        let p_right = sp
            .padding_right
            .resolve_with(ctx.containing_block_width, vw, vh)
            .unwrap_or(0.0);
        let p_bottom = sp
            .padding_bottom
            .resolve_with(ctx.containing_block_width, vw, vh)
            .unwrap_or(0.0);
        let p_left = sp
            .padding_left
            .resolve_with(ctx.containing_block_width, vw, vh)
            .unwrap_or(0.0);

        set_custom_inline_result(
            self.id,
            CustomInlineResult {
                spans: Vec::new(),
                width: css_w,
                height: css_h,
                border_top: b_top,
                border_right: b_right,
                border_bottom: b_bottom,
                border_left: b_left,
                padding_top: p_top,
                padding_right: p_right,
                padding_bottom: p_bottom,
                padding_left: p_left,
            },
        );

        ui_layout::MeasureResult {
            width: css_w,
            height: css_h,
        }
    }

    fn write_debug(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CustomInlineBridge(id={:?})", self.id)
    }
}
