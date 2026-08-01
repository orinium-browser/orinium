//! Bridge between the layout engine's traits and
//! the engine's [`CustomNode`] rendering interface.
//!
//! [`BlockLayouter`] (from `ui_layout`) handles block-level layout positioning:
//! it reports the node's border-box rect to the layout engine.
//!
//! [`FlowLayouter`] (from `ui_layout`) handles inline-level layout:
//! it reports line spans for participation in an inline formatting context.
//!
//! The corresponding [`CustomNode`](super::super::custom_node::CustomNode)
//! (stored in [`NodeKind::Custom`](super::super::layouter::types::NodeKind))
//! handles draw-command generation.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ui_layout::{
    BlockLayouter, FlowLayoutContext, FlowLayouter, LayoutContext, LineSpan, MeasureResult, Rect,
    Style,
};

use crate::engine::layouter::types::ContainerStyle;

use super::super::custom_node::CustomNode;

fn resolve_custom_size(
    node: &dyn CustomNode,
    resolved_width: Option<f32>,
    resolved_height: Option<f32>,
) -> (f32, f32) {
    let (intrinsic_width, intrinsic_height) = node.intrinsic_size();

    if node.preserves_intrinsic_aspect_ratio() {
        match (resolved_width, resolved_height) {
            (Some(width), None) if intrinsic_width > 0.0 => {
                return (width, intrinsic_height * width / intrinsic_width);
            }
            (None, Some(height)) if intrinsic_height > 0.0 => {
                return (intrinsic_width * height / intrinsic_height, height);
            }
            _ => {}
        }
    }

    (
        resolved_width.unwrap_or(intrinsic_width),
        resolved_height.unwrap_or(intrinsic_height),
    )
}

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
        let viewport_width = ctx.containing_block_width.unwrap_or(0.0);
        let viewport_height = ctx.containing_block_height.unwrap_or(0.0);
        let resolved_width = self.layout_style.size.width.resolve_with(
            ctx.containing_block_width,
            viewport_width,
            viewport_height,
        );
        let resolved_height = self.layout_style.size.height.resolve_with(
            ctx.containing_block_height,
            viewport_width,
            viewport_height,
        );
        let (width, height) =
            resolve_custom_size(self.node.as_ref(), resolved_width, resolved_height);

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

// --------------------------------
// Inline layout (FlowLayouter)
// --------------------------------

thread_local! {
    static CUSTOM_INLINE_RESULTS: RefCell<HashMap<usize, CustomInlineResult>> =
        RefCell::new(HashMap::new());
}

static NEXT_CUSTOM_INLINE_ID: AtomicUsize = AtomicUsize::new(1);

#[derive(Debug, Clone)]
pub struct CustomInlineResult {
    pub spans: Vec<LineSpan>,
    pub width: f32,
    pub height: f32,
    pub border_top: f32,
    pub border_right: f32,
    pub border_bottom: f32,
    pub border_left: f32,
    pub padding_top: f32,
    pub padding_right: f32,
    pub padding_bottom: f32,
    pub padding_left: f32,
}

/// An inline-level bridge for custom nodes implementing [`FlowLayouter`].
///
/// During layout, the engine calls [`layout`](Self::layout) which stores
/// the result in a thread-local cache keyed by [`id`](Self::id).  The
/// rendering layer later retrieves it via
/// [`get_custom_inline_result`].
#[derive(Debug)]
pub struct CustomInlineBridge {
    pub id: usize,
    node: Rc<dyn CustomNode>,
    layout_style: Style,
    style: ContainerStyle,
}

impl CustomInlineBridge {
    pub fn new(node: Rc<dyn CustomNode>, layout_style: Style, style: ContainerStyle) -> Self {
        Self {
            id: NEXT_CUSTOM_INLINE_ID.fetch_add(1, Ordering::Relaxed),
            node,
            layout_style,
            style,
        }
    }

    pub fn node(&self) -> &Rc<dyn CustomNode> {
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
    ) -> (f32, f32) {
        let resolved_width = self.layout_style.size.width.resolve_with(
            containing_width,
            viewport_width,
            viewport_height,
        );
        let resolved_height = self.layout_style.size.height.resolve_with(
            containing_height,
            viewport_width,
            viewport_height,
        );
        resolve_custom_size(self.node.as_ref(), resolved_width, resolved_height)
    }
}

impl FlowLayouter for CustomInlineBridge {
    fn layout(&self, ctx: &FlowLayoutContext) -> Vec<LineSpan> {
        let x = ctx.start_pos.0;
        let y = ctx.start_pos.1;

        // Regular inline flow calls layout() without measure(), so resolve
        // CSS dimensions here when no flex/auto-size measurement was cached.
        let (use_width, use_height) = CUSTOM_INLINE_RESULTS.with(|m| {
            m.borrow().get(&self.id).map_or_else(
                || {
                    self.resolve_size(
                        Some(ctx.available_inline_size),
                        None,
                        ctx.available_inline_size,
                        0.0,
                    )
                },
                |r| (r.width, r.height),
            )
        });

        let spans = vec![LineSpan {
            x_range: x..(x + use_width),
            line_pos: (x, y),
            line_index: 0,
        }];

        // Update the cache entry with spans (measure() may have pre-filled it)
        CUSTOM_INLINE_RESULTS.with(|m| {
            if let Some(entry) = m.borrow_mut().get_mut(&self.id) {
                entry.spans = spans.clone();
            } else {
                m.borrow_mut().insert(
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
            }
        });

        spans
    }

    fn measure(&self, ctx: &LayoutContext) -> MeasureResult {
        let vw = ctx.containing_block_width.unwrap_or(0.0);
        let vh = ctx.containing_block_height.unwrap_or(0.0);

        let (css_w, css_h) = self.resolve_size(
            ctx.containing_block_width,
            ctx.containing_block_height,
            vw,
            vh,
        );

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

        CUSTOM_INLINE_RESULTS.with(|m| {
            m.borrow_mut().insert(
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
        });

        MeasureResult {
            width: css_w,
            height: css_h,
        }
    }

    fn write_debug(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CustomInlineBridge(id={})", self.id)
    }
}

/// Retrieve the cached layout result for a custom inline element.
pub fn get_custom_inline_result(id: usize) -> Option<CustomInlineResult> {
    CUSTOM_INLINE_RESULTS.with(|m| m.borrow().get(&id).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui_layout::{Length, LengthOrAuto};

    use crate::engine::renderer_model::Image;
    use crate::engine::ui::image::ImageComponent;

    fn image_bridge(width: LengthOrAuto, height: LengthOrAuto) -> CustomInlineBridge {
        let image = Image::from_rgba(4, 2, vec![255; 32]).unwrap();
        let node: Rc<dyn CustomNode> = Rc::new(ImageComponent { image: Some(image) });
        let mut style = Style::default();
        style.size.width = width;
        style.size.height = height;
        CustomInlineBridge::new(node, style, ContainerStyle::default())
    }

    #[test]
    fn inline_layout_resolves_css_size_without_measurement() {
        let bridge = image_bridge(
            LengthOrAuto::Length(Length::Px(128.0)),
            LengthOrAuto::Length(Length::Px(96.0)),
        );

        let spans = bridge.layout(&FlowLayoutContext {
            start_pos: (10.0, 20.0),
            available_inline_size: 800.0,
            line_height: 16.0,
        });
        let result = get_custom_inline_result(bridge.id).unwrap();

        assert_eq!(spans[0].x_range, 10.0..138.0);
        assert_eq!((result.width, result.height), (128.0, 96.0));
    }

    #[test]
    fn image_with_one_css_dimension_preserves_aspect_ratio() {
        let bridge = image_bridge(LengthOrAuto::Length(Length::Px(100.0)), LengthOrAuto::Auto);

        bridge.layout(&FlowLayoutContext {
            start_pos: (0.0, 0.0),
            available_inline_size: 800.0,
            line_height: 16.0,
        });
        let result = get_custom_inline_result(bridge.id).unwrap();

        assert_eq!((result.width, result.height), (100.0, 50.0));
    }
}
