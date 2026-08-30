//! CSS size resolution helpers shared by the layout bridges.

use ui_layout::Style;

use crate::engine::ui::custom_node::{ContentSize, CustomNode};

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

/// Resolve the **border-box** size of a custom node from its CSS style and the
/// layout context's containing block / viewport.
///
/// Shared by the block and inline layout bridges so CSS `width` / `height`
/// resolution, box sizing, min/max and aspect-ratio handling live in a single
/// place.
///
/// The content-box resolution itself is delegated to ui_layout's
/// [`ui_layout::resolve_custom_box_size`]; the effective aspect ratio prefers
/// the CSS `aspect-ratio` and falls back to the node's intrinsic ratio when
/// [`CustomNode::preserves_intrinsic_aspect_ratio`] is set.
pub(crate) fn resolve_border_box_size(
    node: &dyn CustomNode,
    style: &Style,
    containing_width: Option<f32>,
    containing_height: Option<f32>,
    viewport_width: f32,
    viewport_height: f32,
) -> ContentSize {
    let intrinsic = node.intrinsic_size();
    let aspect_ratio = {
        if node.preserves_intrinsic_aspect_ratio()
            && intrinsic.width > 0.0
            && intrinsic.height > 0.0
        {
            Some(intrinsic.width / intrinsic.height)
        } else {
            None
        }
    };

    let (content_width, content_height) = ui_layout::resolve_custom_box_size(
        style,
        intrinsic.width,
        intrinsic.height,
        aspect_ratio,
        containing_width,
        containing_height,
        viewport_width,
        viewport_height,
    );

    ContentSize {
        width: content_width + horizontal_padding_border(style),
        height: content_height + vertical_padding_border(style),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::engine::layouter::types::{TextFlowStyle, TextStyle};
    use crate::engine::renderer_model::DrawCommand;
    use ui_layout::{BoxSizing, Length, LengthOrAuto};

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

        fn preserves_intrinsic_aspect_ratio(&self) -> bool {
            self.aspect
        }
    }

    fn resolve(node: &dyn CustomNode, style: &Style) -> ContentSize {
        resolve_border_box_size(node, style, None, None, 0.0, 0.0)
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
            resolve(&node, &style),
            ContentSize {
                width: 200.0,
                height: 100.0
            }
        );
    }

    #[test]
    fn explicit_size_overrides_intrinsic() {
        let node = TestNode {
            width: 200.0,
            height: 100.0,
            aspect: false,
        };
        let style = Style {
            size: ui_layout::SizeStyle {
                width: LengthOrAuto::Length(Length::Px(50.0)),
                height: LengthOrAuto::Length(Length::Px(40.0)),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            resolve(&node, &style),
            ContentSize {
                width: 50.0,
                height: 40.0
            }
        );
    }

    #[test]
    fn aspect_ratio_preserved_when_height_auto() {
        let node = TestNode {
            width: 200.0,
            height: 100.0,
            aspect: true,
        };
        let style = Style {
            size: ui_layout::SizeStyle {
                width: LengthOrAuto::Length(Length::Px(100.0)),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            resolve(&node, &style),
            ContentSize {
                width: 100.0,
                height: 50.0
            }
        );
    }

    #[test]
    fn aspect_ratio_preserved_when_width_auto() {
        let node = TestNode {
            width: 200.0,
            height: 100.0,
            aspect: true,
        };
        let style = Style {
            size: ui_layout::SizeStyle {
                height: LengthOrAuto::Length(Length::Px(50.0)),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            resolve(&node, &style),
            ContentSize {
                width: 100.0,
                height: 50.0
            }
        );
    }

    #[test]
    fn css_aspect_ratio_derives_height() {
        let node = TestNode {
            width: 200.0,
            height: 100.0,
            aspect: false,
        };
        let style = Style {
            size: ui_layout::SizeStyle {
                width: LengthOrAuto::Length(Length::Px(100.0)),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            resolve(&node, &style),
            ContentSize {
                width: 100.0,
                height: 50.0
            }
        );
    }

    #[test]
    fn css_aspect_ratio_derives_width() {
        let node = TestNode {
            width: 200.0,
            height: 100.0,
            aspect: false,
        };
        let style = Style {
            size: ui_layout::SizeStyle {
                height: LengthOrAuto::Length(Length::Px(40.0)),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            resolve(&node, &style),
            ContentSize {
                width: 80.0,
                height: 40.0
            }
        );
    }

    #[test]
    fn content_box_adds_padding_border() {
        let node = TestNode {
            width: 200.0,
            height: 100.0,
            aspect: false,
        };
        let style = Style {
            size: ui_layout::SizeStyle {
                width: LengthOrAuto::Length(Length::Px(100.0)),
                height: LengthOrAuto::Length(Length::Px(50.0)),
                ..Default::default()
            },
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
            resolve(&node, &style),
            ContentSize {
                width: 110.0,
                height: 54.0
            }
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
            size: ui_layout::SizeStyle {
                width: LengthOrAuto::Length(Length::Px(120.0)),
                height: LengthOrAuto::Length(Length::Px(60.0)),
                ..Default::default()
            },
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
            resolve(&node, &style),
            ContentSize {
                width: 120.0,
                height: 60.0
            }
        );
    }

    #[test]
    fn min_width_constraint_applied() {
        let node = TestNode {
            width: 200.0,
            height: 100.0,
            aspect: false,
        };
        let style = Style {
            size: ui_layout::SizeStyle {
                width: LengthOrAuto::Length(Length::Px(50.0)),
                height: LengthOrAuto::Length(Length::Px(50.0)),
                min_width: LengthOrAuto::Length(Length::Px(80.0)),
                max_width: LengthOrAuto::Length(Length::Px(90.0)),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            resolve(&node, &style),
            ContentSize {
                width: 80.0,
                height: 50.0
            }
        );

        let style = Style {
            size: ui_layout::SizeStyle {
                width: LengthOrAuto::Length(Length::Px(95.0)),
                min_width: LengthOrAuto::Length(Length::Px(80.0)),
                max_width: LengthOrAuto::Length(Length::Px(90.0)),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            resolve(&node, &style),
            ContentSize {
                width: 90.0,
                height: 100.0
            }
        );
    }
}
