//! Thread-local cache for inline custom node layout results.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use ui_layout::{BoxSizing, LineSpan, Style};

use crate::engine::ui::custom_node::CustomNode;

/// Unique identifier for an inline custom element's layout result.
///
/// Allocated when an inline custom element is built (see
/// [`next_custom_inline_id`]) and used as the cache key during layout and
/// rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InlineLayoutId(pub usize);

thread_local! {
    static CUSTOM_INLINE_RESULTS: RefCell<HashMap<InlineLayoutId, CustomInlineResult>> =
        RefCell::new(HashMap::new());
}

static NEXT_CUSTOM_INLINE_ID: AtomicUsize = AtomicUsize::new(1);

#[derive(Debug, Clone)]
pub(crate) struct CustomInlineResult {
    pub(crate) spans: Vec<LineSpan>,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) border_top: f32,
    pub(crate) border_right: f32,
    pub(crate) border_bottom: f32,
    pub(crate) border_left: f32,
    pub(crate) padding_top: f32,
    pub(crate) padding_right: f32,
    pub(crate) padding_bottom: f32,
    pub(crate) padding_left: f32,
}

/// Retrieve the cached layout result for a custom inline element.
pub(crate) fn get_custom_inline_result(id: InlineLayoutId) -> Option<CustomInlineResult> {
    CUSTOM_INLINE_RESULTS.with(|m| m.borrow().get(&id).cloned())
}

/// Store a `CustomInlineResult` in the thread-local cache.
pub(crate) fn set_custom_inline_result(id: InlineLayoutId, result: CustomInlineResult) {
    CUSTOM_INLINE_RESULTS.with(|m| {
        m.borrow_mut().insert(id, result);
    });
}

/// Remove a `CustomInlineResult` from the thread-local cache.
///
/// Called when the owning inline bridge is dropped so the cache does not grow
/// unboundedly across page rebuilds.
pub(crate) fn remove_custom_inline_result(id: InlineLayoutId) {
    CUSTOM_INLINE_RESULTS.with(|m| {
        m.borrow_mut().remove(&id);
    });
}

/// Allocate a new unique inline ID.
pub(crate) fn next_custom_inline_id() -> InlineLayoutId {
    InlineLayoutId(NEXT_CUSTOM_INLINE_ID.fetch_add(1, Ordering::Relaxed))
}

/// Horizontal padding + border (content-box → border-box delta).
fn horizontal_padding_border(style: &Style) -> f32 {
    let pl = style
        .spacing
        .padding_left
        .resolve_with(None, 0.0, 0.0)
        .unwrap_or(0.0);
    let pr = style
        .spacing
        .padding_right
        .resolve_with(None, 0.0, 0.0)
        .unwrap_or(0.0);
    let bl = style
        .spacing
        .border_left
        .resolve_with(None, 0.0, 0.0)
        .unwrap_or(0.0);
    let br = style
        .spacing
        .border_right
        .resolve_with(None, 0.0, 0.0)
        .unwrap_or(0.0);
    pl + pr + bl + br
}

/// Vertical padding + border (content-box → border-box delta).
fn vertical_padding_border(style: &Style) -> f32 {
    let pt = style
        .spacing
        .padding_top
        .resolve_with(None, 0.0, 0.0)
        .unwrap_or(0.0);
    let pb = style
        .spacing
        .padding_bottom
        .resolve_with(None, 0.0, 0.0)
        .unwrap_or(0.0);
    let bt = style
        .spacing
        .border_top
        .resolve_with(None, 0.0, 0.0)
        .unwrap_or(0.0);
    let bb = style
        .spacing
        .border_bottom
        .resolve_with(None, 0.0, 0.0)
        .unwrap_or(0.0);
    pt + pb + bt + bb
}

/// Resolve CSS dimensions for a custom node, returning the **border-box**
/// size in pixels.
///
/// `resolved_width`/`resolved_height` are the resolved CSS `width`/`height`
/// values (`None` when `auto`). The return value is the element's border-box
/// size so that the caller can use it directly as a layout rect.
///
/// The resolved size honors:
/// - `box-sizing`: for `BorderBox` the CSS size already includes padding and
///   border; for `ContentBox` padding/border are added on top.
/// - `min-`/`max-` width/height constraints (absolute lengths; percentages
///   cannot be resolved without a containing block and are skipped).
pub(crate) fn resolve_custom_size(
    node: &dyn CustomNode,
    resolved_width: Option<f32>,
    resolved_height: Option<f32>,
    style: &Style,
) -> (f32, f32) {
    let (intrinsic_width, intrinsic_height) = node.intrinsic_size();
    let pb_h = horizontal_padding_border(style);
    let pb_v = vertical_padding_border(style);

    let content_width = resolved_width.map(|w| match style.box_sizing {
        BoxSizing::ContentBox => w,
        BoxSizing::BorderBox => (w - pb_h).max(0.0),
    });
    let content_height = resolved_height.map(|h| match style.box_sizing {
        BoxSizing::ContentBox => h,
        BoxSizing::BorderBox => (h - pb_v).max(0.0),
    });

    let (content_width, content_height) = if node.preserves_intrinsic_aspect_ratio() {
        match (content_width, content_height) {
            (Some(width), None) if intrinsic_width > 0.0 => {
                (width, intrinsic_height * width / intrinsic_width)
            }
            (None, Some(height)) if intrinsic_height > 0.0 => {
                (intrinsic_width * height / intrinsic_height, height)
            }
            _ => (
                content_width.unwrap_or(intrinsic_width),
                content_height.unwrap_or(intrinsic_height),
            ),
        }
    } else {
        (
            content_width.unwrap_or(intrinsic_width),
            content_height.unwrap_or(intrinsic_height),
        )
    };

    let (mut width, mut height) = (content_width, content_height);
    if let Some(min_w) = style.size.min_width.resolve_with(None, 0.0, 0.0) {
        width = width.max(min_w);
    }
    if let Some(max_w) = style.size.max_width.resolve_with(None, 0.0, 0.0) {
        width = width.min(max_w);
    }
    if let Some(min_h) = style.size.min_height.resolve_with(None, 0.0, 0.0) {
        height = height.max(min_h);
    }
    if let Some(max_h) = style.size.max_height.resolve_with(None, 0.0, 0.0) {
        height = height.min(max_h);
    }

    (width + pb_h, height + pb_v)
}

/// Resolve the **border-box** size of a custom node from its resolved CSS
/// style and the layout context's containing block / viewport.
///
/// Shared by the block and inline layout bridges so CSS `width` / `height`
/// resolution and box sizing live in a single place.
pub(crate) fn resolve_border_box_size(
    node: &dyn CustomNode,
    style: &Style,
    containing_width: Option<f32>,
    containing_height: Option<f32>,
    viewport_width: f32,
    viewport_height: f32,
) -> (f32, f32) {
    let resolved_width =
        style
            .size
            .width
            .resolve_with(containing_width, viewport_width, viewport_height);
    let resolved_height =
        style
            .size
            .height
            .resolve_with(containing_height, viewport_width, viewport_height);
    resolve_custom_size(node, resolved_width, resolved_height, style)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::engine::layouter::types::TextStyle;
    use crate::engine::renderer_model::DrawCommand;
    use ui_layout::{Length, LengthOrAuto};

    #[derive(Debug)]
    struct TestNode {
        width: f32,
        height: f32,
        aspect: bool,
    }

    impl CustomNode for TestNode {
        fn draw_sized(
            &self,
            _cmd_buf: &mut Vec<DrawCommand>,
            _text_style: &TextStyle,
            _style: &Style,
            _size: (f32, f32),
        ) {
        }

        fn intrinsic_size(&self) -> (f32, f32) {
            (self.width, self.height)
        }

        fn preserves_intrinsic_aspect_ratio(&self) -> bool {
            self.aspect
        }
    }

    #[test]
    fn auto_sizes_use_intrinsic() {
        let node = TestNode {
            width: 200.0,
            height: 100.0,
            aspect: true,
        };
        let style = Style::default();
        assert_eq!(
            resolve_custom_size(&node, None, None, &style),
            (200.0, 100.0)
        );
    }

    #[test]
    fn explicit_size_overrides_intrinsic() {
        let node = TestNode {
            width: 200.0,
            height: 100.0,
            aspect: false,
        };
        let style = Style::default();
        assert_eq!(
            resolve_custom_size(&node, Some(50.0), Some(40.0), &style),
            (50.0, 40.0)
        );
    }

    #[test]
    fn aspect_ratio_preserved_when_height_auto() {
        let node = TestNode {
            width: 200.0,
            height: 100.0,
            aspect: true,
        };
        let style = Style::default();
        assert_eq!(
            resolve_custom_size(&node, Some(100.0), None, &style),
            (100.0, 50.0)
        );
    }

    #[test]
    fn aspect_ratio_preserved_when_width_auto() {
        let node = TestNode {
            width: 200.0,
            height: 100.0,
            aspect: true,
        };
        let style = Style::default();
        assert_eq!(
            resolve_custom_size(&node, None, Some(50.0), &style),
            (100.0, 50.0)
        );
    }

    #[test]
    fn content_box_adds_padding_border() {
        let node = TestNode {
            width: 200.0,
            height: 100.0,
            aspect: false,
        };
        let mut style = Style::default();
        style.spacing.padding_left = Length::Px(5.0);
        style.spacing.padding_right = Length::Px(5.0);
        style.spacing.border_top = Length::Px(2.0);
        style.spacing.border_bottom = Length::Px(2.0);
        assert_eq!(
            resolve_custom_size(&node, Some(100.0), Some(50.0), &style),
            (110.0, 54.0)
        );
    }

    #[test]
    fn border_box_subtracts_padding_border() {
        let node = TestNode {
            width: 200.0,
            height: 100.0,
            aspect: false,
        };
        let style = Style {
            box_sizing: BoxSizing::BorderBox,
            spacing: ui_layout::Spacing {
                padding_left: Length::Px(5.0),
                padding_right: Length::Px(5.0),
                border_top: Length::Px(2.0),
                border_bottom: Length::Px(2.0),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            resolve_custom_size(&node, Some(120.0), Some(60.0), &style),
            (120.0, 60.0)
        );
    }

    #[test]
    fn min_width_constraint_applied() {
        let node = TestNode {
            width: 200.0,
            height: 100.0,
            aspect: false,
        };
        let mut style = Style::default();
        style.size.min_width = LengthOrAuto::Length(Length::Px(80.0));
        style.size.max_width = LengthOrAuto::Length(Length::Px(90.0));
        assert_eq!(
            resolve_custom_size(&node, Some(50.0), Some(50.0), &style),
            (80.0, 50.0)
        );
        assert_eq!(
            resolve_custom_size(&node, Some(95.0), Some(50.0), &style),
            (90.0, 50.0)
        );
    }
}
