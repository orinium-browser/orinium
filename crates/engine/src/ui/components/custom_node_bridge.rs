//! Unified layout bridge for custom nodes.
//!
//! Implements the unified [`CustomLayouter`] trait for both block and inline
//! formatting contexts. The resolved [`OuterDisplay`] is taken from the owned
//! [`Style`] (`layout_style`) at layout time and drives the shape of the
//! returned [`LayoutBox`].

use ui_layout::{
    BoxModel, CustomLayouter, InlineBox, LayoutBox, LayoutContext, OuterDisplay, Rect, Style,
};

use crate::ui::custom_node::{ContentSize, CustomNode};

use super::inline_cache::resolve_border_box_size;

/// A [`CustomLayouter`] implementation for custom nodes.
///
/// Holds the resolved [`Style`] so the layout engine can account for borders
/// and padding when sizing the element. The actual content rendering is
/// delegated to a separate [`CustomNode`](crate::ui::CustomNode).
///
/// The element participates in the formatting context reported by the owned
/// style's [`OuterDisplay`]:
///
/// - [`OuterDisplay::Inline`] → `layout` returns an atomic
///   [`LayoutBox::InlineBox`]. The flow engine creates its line span so the
///   element's full border-box height contributes to the line box.
/// - [`OuterDisplay::Block`] → `layout` returns a [`LayoutBox::BlockBox`]
///   positioned at the origin; the engine translates it to its final position.
/// - [`OuterDisplay::None`] → the element produces nothing.
///
/// [`measure`](Self::measure) is implemented for every context so flex sizing
/// and auto-height work regardless of the display value.
#[derive(Debug)]
pub struct CustomNodeBridge {
    node: std::sync::Arc<dyn CustomNode>,
    layout_style: Style,
}

impl CustomNodeBridge {
    pub fn new(node: std::sync::Arc<dyn CustomNode>, layout_style: Style) -> Self {
        Self { node, layout_style }
    }

    /// Returns the resolved [`Style`] that drives this element's layout.
    ///
    /// The layout engine reads [`Display`](ui_layout::Display) from it, exactly
    /// as for a childless `LayoutNode`.
    pub fn style(&self) -> &Style {
        &self.layout_style
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

impl CustomLayouter for CustomNodeBridge {
    fn layout(&mut self, ctx: &LayoutContext) -> LayoutBox {
        match self.layout_style.display.outer {
            OuterDisplay::Inline => {
                let x = ctx.start_pos.0;
                let y = ctx.start_pos.1;

                let resolved = self.resolve_size(
                    Some(ctx.available_inline_size),
                    None,
                    ctx.viewport_width,
                    ctx.viewport_height,
                );
                let (use_width, use_height) = (resolved.width, resolved.height);

                let rect = Rect {
                    x,
                    y,
                    width: use_width,
                    height: use_height,
                };
                let box_model = BoxModel {
                    sticky_edges: None,
                    border_box: rect,
                    padding_box: rect,
                    content_box: rect,
                    children_box: rect,
                };

                LayoutBox::InlineBox(InlineBox {
                    box_model,
                    // Atomic inline replaced elements leave positioning to
                    // the flow engine. A pre-populated span is interpreted as
                    // text-flow output and only contributes `line-height`,
                    // which collapses a tall image's containing inline-block.
                    line_spans: Vec::new(),
                })
            }
            OuterDisplay::Block => {
                let resolved = self.resolve_size(
                    ctx.containing_block_width,
                    ctx.containing_block_height,
                    ctx.viewport_width,
                    ctx.viewport_height,
                );

                let rect = Rect {
                    x: 0.0,
                    y: 0.0,
                    width: resolved.width,
                    height: resolved.height,
                };
                let box_model = BoxModel {
                    sticky_edges: None,
                    border_box: rect,
                    padding_box: rect,
                    content_box: rect,
                    children_box: rect,
                };

                LayoutBox::BlockBox(box_model)
            }
            OuterDisplay::None => LayoutBox::None,
        }
    }

    fn measure(&self, ctx: &LayoutContext) -> ui_layout::MeasureResult {
        let resolved = self.resolve_size(
            ctx.containing_block_width,
            ctx.containing_block_height,
            ctx.viewport_width,
            ctx.viewport_height,
        );

        ui_layout::MeasureResult {
            width: resolved.width,
            height: resolved.height,
        }
    }

    fn write_debug(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CustomNodeBridge")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::layouter::types::{TextFlowStyle, TextStyle};
    use crate::renderer_model::DrawCommand;

    #[derive(Debug)]
    struct TestNode {
        width: f32,
        height: f32,
    }

    impl CustomNode for TestNode {
        fn draw_sized(
            &self,
            _cmd_buf: &mut Vec<DrawCommand>,
            _text_style: &TextStyle,
            _text_flow_style: &TextFlowStyle,
            _style: &Style,
            _size: ContentSize,
        ) {
        }

        fn intrinsic_size(&self) -> ContentSize {
            ContentSize {
                width: self.width,
                height: self.height,
            }
        }
    }

    fn bridge(display: OuterDisplay) -> CustomNodeBridge {
        let mut style = Style::default();
        style.display.outer = display;
        CustomNodeBridge::new(
            std::sync::Arc::new(TestNode {
                width: 200.0,
                height: 100.0,
            }),
            style,
        )
    }

    #[test]
    fn block_context_reports_block() {
        assert_eq!(
            bridge(OuterDisplay::Block).style().display.outer,
            OuterDisplay::Block
        );
    }

    #[test]
    fn block_layout_returns_block_box_at_origin() {
        let mut b = bridge(OuterDisplay::Block);
        match b.layout(&LayoutContext::default()) {
            LayoutBox::BlockBox(bm) => {
                assert_eq!(bm.border_box.x, 0.0);
                assert_eq!(bm.border_box.y, 0.0);
                assert_eq!(bm.border_box.width, 200.0);
                assert_eq!(bm.border_box.height, 100.0);
            }
            other => panic!("expected BlockBox, got {:?}", other),
        }
    }

    #[test]
    fn block_measure_returns_intrinsic_size() {
        let b = bridge(OuterDisplay::Block);
        let m = b.measure(&LayoutContext::default());
        assert_eq!(m.width, 200.0);
        assert_eq!(m.height, 100.0);
    }

    #[test]
    fn inline_context_reports_inline() {
        assert_eq!(
            bridge(OuterDisplay::Inline).style().display.outer,
            OuterDisplay::Inline
        );
    }

    #[test]
    fn inline_layout_returns_an_atomic_unpositioned_box() {
        let mut b = bridge(OuterDisplay::Inline);
        let ctx = LayoutContext {
            start_pos: (10.0, 20.0),
            available_inline_size: 300.0,
            viewport_width: 800.0,
            viewport_height: 600.0,
            ..LayoutContext::default()
        };
        match b.layout(&ctx) {
            LayoutBox::InlineBox(inline) => {
                assert_eq!(inline.box_model.border_box.x, 10.0);
                assert_eq!(inline.box_model.border_box.y, 20.0);
                assert_eq!(inline.box_model.border_box.width, 200.0);
                assert_eq!(inline.box_model.border_box.height, 100.0);
                assert!(inline.line_spans.is_empty());
            }
            other => panic!("expected InlineBox, got {:?}", other),
        }
    }

    #[test]
    fn inline_measure_returns_intrinsic_size() {
        let b = bridge(OuterDisplay::Inline);
        let m = b.measure(&LayoutContext::default());
        assert_eq!(m.width, 200.0);
        assert_eq!(m.height, 100.0);
    }

    #[test]
    fn none_context_skips_element() {
        let mut b = bridge(OuterDisplay::None);
        assert_eq!(b.style().display.outer, OuterDisplay::None);
        assert!(matches!(
            b.layout(&LayoutContext::default()),
            LayoutBox::None
        ));
    }

    #[test]
    fn inline_size_resolves_against_available_inline_size() {
        use ui_layout::{Length, LengthOrAuto};

        let mut style = Style {
            size: ui_layout::SizeStyle {
                width: LengthOrAuto::Length(Length::Percent(50.0)),
                ..Default::default()
            },
            ..Default::default()
        };
        style.display.outer = OuterDisplay::Inline;
        let b = CustomNodeBridge::new(
            std::sync::Arc::new(TestNode {
                width: 200.0,
                height: 100.0,
            }),
            style,
        );
        let mut b = b;
        let ctx = LayoutContext {
            start_pos: (0.0, 0.0),
            available_inline_size: 150.0,
            viewport_width: 800.0,
            viewport_height: 600.0,
            ..LayoutContext::default()
        };
        match b.layout(&ctx) {
            LayoutBox::InlineBox(inline) => {
                assert_eq!(inline.box_model.border_box.width, 75.0);
            }
            other => panic!("expected InlineBox, got {:?}", other),
        }
    }
}
