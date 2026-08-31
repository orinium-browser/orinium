use super::*;
use crate::engine::bridge::text::FallbackTextMeasurer;
use crate::engine::css::parser::Parser as CssParser;
use crate::engine::html::parser::Parser as HtmlParser;
use crate::engine::layouter::css_resolver::CssResolver;
use std::sync::Arc;

fn apply_layout_property(name: &str, value: CssValue) -> Style {
    let mut style = Style::default();
    let mut container_style = ContainerStyle::default();
    let mut text_style = TextStyle::default();
    let mut text_flow_style = TextFlowStyle::default();
    let mut overflow = Overflow::default();
    let parsed = apply_declaration(
        name,
        &value,
        &mut style,
        &mut container_style,
        &mut text_style,
        &mut text_flow_style,
        &Style::default(),
        &ContainerStyle::default(),
        &TextStyle::default(),
        &TextFlowStyle::default(),
        &mut overflow,
        ColorScheme::Light,
    );
    assert!(parsed.is_some());
    style
}

fn apply_container_property(name: &str, value: CssValue) -> ContainerStyle {
    let mut style = Style::default();
    let mut container_style = ContainerStyle::default();
    let mut text_style = TextStyle::default();
    let mut text_flow_style = TextFlowStyle::default();
    let mut overflow = Overflow::default();
    let parsed = apply_declaration(
        name,
        &value,
        &mut style,
        &mut container_style,
        &mut text_style,
        &mut text_flow_style,
        &Style::default(),
        &ContainerStyle::default(),
        &TextStyle::default(),
        &TextFlowStyle::default(),
        &mut overflow,
        ColorScheme::Light,
    );
    assert!(parsed.is_some());
    container_style
}

#[test]
fn position_keywords_map_to_layout_position() {
    for (keyword, expected) in [
        ("static", Position::Static),
        ("relative", Position::Relative),
        ("absolute", Position::Absolute),
        ("fixed", Position::Fixed),
    ] {
        let style = apply_layout_property("position", CssValue::Keyword(keyword.into()));
        assert_eq!(style.position.kind, expected);
    }
}

#[test]
fn z_index_accepts_integers_and_auto() {
    assert_eq!(
        apply_container_property("z-index", CssValue::Number(999999.0)).z_index,
        Some(999999)
    );
    assert_eq!(
        apply_container_property("z-index", CssValue::Keyword("auto".into())).z_index,
        None
    );
}

#[test]
fn float_keywords_are_preserved_for_layout_blockification() {
    assert_eq!(
        apply_container_property("float", CssValue::Keyword("left".into())).css_float,
        CssFloat::Left
    );
    assert_eq!(
        apply_container_property("float", CssValue::Keyword("right".into())).css_float,
        CssFloat::Right
    );
    assert_eq!(
        apply_container_property("float", CssValue::Keyword("none".into())).css_float,
        CssFloat::None
    );
}

#[test]
fn inset_shorthand_expands_lengths_and_auto() {
    let style = apply_layout_property(
        "inset",
        CssValue::List(vec![
            CssValue::Length(10.0, Unit::Px),
            CssValue::Length(20.0, Unit::Percent),
            CssValue::Keyword("auto".into()),
            CssValue::Length(4.0, Unit::Px),
        ]),
    );

    assert_eq!(style.position.top, Length::Px(10.0).into());
    assert_eq!(style.position.right, Length::Percent(20.0).into());
    assert_eq!(style.position.bottom, LengthOrAuto::Auto);
    assert_eq!(style.position.left, Length::Px(4.0).into());
}

#[test]
fn positioned_insets_map_to_individual_sides() {
    for (name, value) in [("top", 1.0), ("right", 2.0), ("bottom", 3.0), ("left", 4.0)] {
        let style = apply_layout_property(name, CssValue::Length(value, Unit::Px));
        let actual = match name {
            "top" => style.position.top,
            "right" => style.position.right,
            "bottom" => style.position.bottom,
            "left" => style.position.left,
            _ => unreachable!(),
        };
        assert_eq!(actual, Length::Px(value).into());
    }
}

#[test]
fn nested_calc_operands_resolve_with_type_checking() {
    // calc(calc(1 - 0) * 10px) => Length::Mul(Px(10), 1.0)
    let style = apply_layout_property(
        "margin-top",
        CssValue::Function(
            "calc".into(),
            vec![vec![
                CssValue::Function(
                    "calc".into(),
                    vec![vec![
                        CssValue::Number(1.0),
                        CssValue::Keyword("-".into()),
                        CssValue::Number(0.0),
                    ]],
                ),
                CssValue::Keyword("*".into()),
                CssValue::Length(10.0, Unit::Px),
            ]],
        ),
    );
    assert_eq!(
        style.spacing.margin_top,
        LengthOrAuto::Length(Length::Mul(Box::new(Length::Px(10.0)), 1.0))
    );
}

#[test]
fn calc_rejects_mixed_number_length_arithmetic() {
    let text_flow_style = TextFlowStyle::default();
    // 10px + 5 is invalid: cannot add a number to a length
    let add = CssValue::Function(
        "calc".into(),
        vec![vec![
            CssValue::Length(10.0, Unit::Px),
            CssValue::Keyword("+".into()),
            CssValue::Number(5.0),
        ]],
    );
    assert_eq!(
        resolve_css_len("margin-top", std::slice::from_ref(&add), &text_flow_style),
        None
    );
    // 10px * 5px is invalid: cannot multiply two lengths
    let mul = CssValue::Function(
        "calc".into(),
        vec![vec![
            CssValue::Length(10.0, Unit::Px),
            CssValue::Keyword("*".into()),
            CssValue::Length(5.0, Unit::Px),
        ]],
    );
    assert_eq!(
        resolve_css_len("margin-top", std::slice::from_ref(&mul), &text_flow_style),
        None
    );
}

#[test]
fn flat_component_slice_resolves_as_arithmetic() {
    let text_flow_style = TextFlowStyle::default();

    // A single primitive component resolves directly.
    let single = [CssValue::Length(10.0, Unit::Px)];
    assert_eq!(
        resolve_css_len("width", &single, &text_flow_style),
        Some(Length::Px(10.0))
    );

    // A flat `[a, +, b]` expression resolves without a `calc()` wrapper.
    let add = [
        CssValue::Length(10.0, Unit::Px),
        CssValue::Keyword("+".into()),
        CssValue::Length(5.0, Unit::Px),
    ];
    assert_eq!(
        resolve_css_len("width", &add, &text_flow_style),
        Some(Length::Add(
            Box::new(Length::Px(10.0)),
            Box::new(Length::Px(5.0))
        ))
    );

    // Longer left-associative chains are supported.
    let chain = [
        CssValue::Number(2.0),
        CssValue::Keyword("*".into()),
        CssValue::Length(10.0, Unit::Px),
        CssValue::Keyword("+".into()),
        CssValue::Length(4.0, Unit::Px),
    ];
    assert_eq!(
        resolve_css_len("width", &chain, &text_flow_style),
        Some(Length::Add(
            Box::new(Length::Mul(Box::new(Length::Px(10.0)), 2.0)),
            Box::new(Length::Px(4.0)),
        ))
    );

    // Mixed number/length arithmetic type-checks and is rejected.
    let invalid = [
        CssValue::Length(10.0, Unit::Px),
        CssValue::Keyword("+".into()),
        CssValue::Number(5.0),
    ];
    assert_eq!(resolve_css_len("width", &invalid, &text_flow_style), None);

    // An empty slice resolves to no length.
    assert_eq!(resolve_css_len("width", &[], &text_flow_style), None);
}

#[test]
fn flex_shorthand_expands_common_forms() {
    let one = apply_layout_property("flex", CssValue::Number(1.0));
    assert_eq!(one.item_style.flex_grow, 1.0);
    assert_eq!(one.item_style.flex_shrink, 1.0);
    assert_eq!(
        one.item_style.flex_basis,
        LengthOrAuto::Length(Length::Percent(0.0))
    );

    let explicit = apply_layout_property(
        "flex",
        CssValue::List(vec![
            CssValue::Number(2.0),
            CssValue::Number(0.0),
            CssValue::Length(10.0, Unit::Px),
        ]),
    );
    assert_eq!(explicit.item_style.flex_grow, 2.0);
    assert_eq!(explicit.item_style.flex_shrink, 0.0);
    assert_eq!(
        explicit.item_style.flex_basis,
        LengthOrAuto::Length(Length::Px(10.0))
    );
}

#[test]
fn flex_shorthand_expands_keywords() {
    for (keyword, expected_grow, expected_shrink, expected_basis) in [
        ("none", 0.0, 0.0, LengthOrAuto::Auto),
        ("auto", 1.0, 1.0, LengthOrAuto::Auto),
        ("initial", 0.0, 1.0, LengthOrAuto::Auto),
    ] {
        let style = apply_layout_property("flex", CssValue::Keyword(keyword.into()));
        assert_eq!(style.item_style.flex_grow, expected_grow);
        assert_eq!(style.item_style.flex_shrink, expected_shrink);
        assert_eq!(style.item_style.flex_basis, expected_basis);
    }
}

#[test]
fn flex_wrap_and_align_content_map_to_layout() {
    let wrap = apply_layout_property("flex-wrap", CssValue::Keyword("wrap-reverse".into()));
    assert_eq!(wrap.flex_wrap, FlexWrap::WrapReverse);

    for (keyword, expected) in [
        ("normal", AlignContent::Stretch),
        ("flex-start", AlignContent::Start),
        ("center", AlignContent::Center),
        ("flex-end", AlignContent::End),
        ("space-between", AlignContent::SpaceBetween),
        ("space-around", AlignContent::SpaceAround),
        ("space-evenly", AlignContent::SpaceEvenly),
    ] {
        let style = apply_layout_property("align-content", CssValue::Keyword(keyword.into()));
        assert_eq!(style.align_content, expected);
    }
}

#[test]
fn flex_flow_expands_direction_and_wrap_in_either_order() {
    for values in [
        vec![
            CssValue::Keyword("column".into()),
            CssValue::Keyword("wrap".into()),
        ],
        vec![
            CssValue::Keyword("wrap".into()),
            CssValue::Keyword("column".into()),
        ],
    ] {
        let style = apply_layout_property("flex-flow", CssValue::List(values));
        assert_eq!(style.flex_direction, FlexDirection::Column);
        assert_eq!(style.flex_wrap, FlexWrap::Wrap);
    }

    let wrap_only = apply_layout_property("flex-flow", CssValue::Keyword("wrap-reverse".into()));
    assert_eq!(wrap_only.flex_direction, FlexDirection::Row);
    assert_eq!(wrap_only.flex_wrap, FlexWrap::WrapReverse);
}

#[test]
fn grid_tracks_map_lengths_fractions_and_auto() {
    let columns = apply_layout_property(
        "grid-template-columns",
        CssValue::List(vec![
            CssValue::Length(100.0, Unit::Px),
            CssValue::Length(2.0, Unit::Fr),
            CssValue::Keyword("auto".into()),
        ]),
    );
    assert_eq!(
        columns.grid_template_columns,
        vec![
            GridTrack::Breadth(LengthOrAuto::Length(Length::Px(100.0))),
            GridTrack::Flex(2.0),
            GridTrack::default(),
        ]
    );

    let rows = apply_layout_property(
        "grid-template-rows",
        CssValue::List(vec![
            CssValue::Keyword("auto".into()),
            CssValue::Length(25.0, Unit::Percent),
        ]),
    );
    assert_eq!(
        rows.grid_template_rows,
        vec![
            GridTrack::default(),
            GridTrack::Breadth(LengthOrAuto::Length(Length::Percent(25.0))),
        ]
    );

    let none = apply_layout_property("grid-template-columns", CssValue::Keyword("none".into()));
    assert!(none.grid_template_columns.is_empty());
}

#[test]
fn grid_tracks_map_repeat_and_minmax() {
    let style = apply_layout_property(
        "grid-template-columns",
        CssValue::Function(
            "repeat".into(),
            vec![
                vec![CssValue::Keyword("auto-fit".into())],
                vec![CssValue::Function(
                    "minmax".into(),
                    vec![
                        vec![CssValue::Length(100.0, Unit::Px)],
                        vec![CssValue::Length(1.0, Unit::Fr)],
                    ],
                )],
            ],
        ),
    );
    assert_eq!(
        style.grid_template_columns,
        vec![GridTrack::Repeat(
            GridRepeat::AutoFit,
            vec![GridTrack::MinMax(
                Box::new(GridTrack::Breadth(LengthOrAuto::Length(Length::Px(100.0,)))),
                Box::new(GridTrack::Flex(1.0)),
            )],
        )]
    );
}

#[test]
fn grid_named_areas_map_rows_and_item_name() {
    let template = apply_layout_property(
        "grid-template-areas",
        CssValue::List(vec![
            CssValue::String("header header".into()),
            CssValue::String("sidebar main".into()),
            CssValue::String("footer footer".into()),
        ]),
    );
    assert_eq!(
        template.grid_template_areas,
        vec![
            vec!["header".to_string(), "header".to_string()],
            vec!["sidebar".to_string(), "main".to_string()],
            vec!["footer".to_string(), "footer".to_string()],
        ]
    );

    let item = apply_layout_property("grid-area", CssValue::Keyword("header".into()));
    assert_eq!(item.grid_area.as_deref(), Some("header"));
}

#[test]
fn absolute_and_fixed_inline_boxes_are_blockified() {
    for position in [Position::Absolute, Position::Fixed] {
        let mut style = Style {
            display: Display::OutsideInner {
                outer: OuterDisplay::Inline,
                inner: InnerDisplay::Flow,
            },
            position: ui_layout::PositionStyle {
                kind: position,
                ..Default::default()
            },
            ..Default::default()
        };
        blockify_out_of_flow_positioned(&mut style);
        assert_eq!(style.display.outer(), Some(OuterDisplay::Block));
    }
}

fn apply_overflow(value: CssValue) -> Overflow {
    let mut style = Style::default();
    let mut container_style = ContainerStyle::default();
    let mut text_style = TextStyle::default();
    let mut text_flow_style = TextFlowStyle::default();
    let mut overflow = Overflow::default();
    let parsed = apply_declaration(
        "overflow",
        &value,
        &mut style,
        &mut container_style,
        &mut text_style,
        &mut text_flow_style,
        &Style::default(),
        &ContainerStyle::default(),
        &TextStyle::default(),
        &TextFlowStyle::default(),
        &mut overflow,
        ColorScheme::Light,
    );
    assert!(parsed.is_some());
    overflow
}

#[test]
fn overflow_single_keyword_sets_both_axes() {
    assert_eq!(
        apply_overflow(CssValue::Keyword("hidden".into())),
        Overflow { x: true, y: true }
    );
    assert_eq!(
        apply_overflow(CssValue::Keyword("auto".into())),
        Overflow { x: true, y: true }
    );
    assert_eq!(
        apply_overflow(CssValue::Keyword("visible".into())),
        Overflow { x: false, y: false }
    );
    assert_eq!(
        apply_overflow(CssValue::Keyword("clip".into())),
        Overflow { x: false, y: false }
    );
}

#[test]
fn overflow_two_keywords_set_axes_independently() {
    assert_eq!(
        apply_overflow(CssValue::List(vec![
            CssValue::Keyword("hidden".into()),
            CssValue::Keyword("visible".into()),
        ])),
        Overflow { x: true, y: false }
    );
    assert_eq!(
        apply_overflow(CssValue::List(vec![
            CssValue::Keyword("visible".into()),
            CssValue::Keyword("auto".into()),
        ])),
        Overflow { x: false, y: true }
    );
}

#[test]
fn overflow_axis_properties_set_single_axis() {
    let mut style = Style::default();
    let mut container_style = ContainerStyle::default();
    let mut text_style = TextStyle::default();
    let mut text_flow_style = TextFlowStyle::default();
    let mut overflow = Overflow::default();

    assert!(
        apply_declaration(
            "overflow-x",
            &CssValue::Keyword("scroll".into()),
            &mut style,
            &mut container_style,
            &mut text_style,
            &mut text_flow_style,
            &Style::default(),
            &ContainerStyle::default(),
            &TextStyle::default(),
            &TextFlowStyle::default(),
            &mut overflow,
            ColorScheme::Light,
        )
        .is_some()
    );
    assert_eq!(overflow, Overflow { x: true, y: false });

    assert!(
        apply_declaration(
            "overflow-y",
            &CssValue::Keyword("auto".into()),
            &mut style,
            &mut container_style,
            &mut text_style,
            &mut text_flow_style,
            &Style::default(),
            &ContainerStyle::default(),
            &TextStyle::default(),
            &TextFlowStyle::default(),
            &mut overflow,
            ColorScheme::Light,
        )
        .is_some()
    );
    assert_eq!(overflow, Overflow { x: true, y: true });
}

#[test]
fn logical_inline_margins_apply_to_both_physical_sides() {
    let mut style = Style::default();
    let mut container_style = ContainerStyle::default();
    let mut text_style = TextStyle::default();
    let mut text_flow_style = TextFlowStyle::default();
    let mut overflow = Overflow::default();

    assert!(
        apply_declaration(
            "margin-inline",
            &CssValue::Keyword("auto".into()),
            &mut style,
            &mut container_style,
            &mut text_style,
            &mut text_flow_style,
            &Style::default(),
            &ContainerStyle::default(),
            &TextStyle::default(),
            &TextFlowStyle::default(),
            &mut overflow,
            ColorScheme::Light,
        )
        .is_some()
    );
    assert_eq!(style.spacing.margin_left, LengthOrAuto::Auto);
    assert_eq!(style.spacing.margin_right, LengthOrAuto::Auto);
}

#[test]
fn clamp_font_size_uses_pixel_bound_for_viewport_preference() {
    let mut style = Style::default();
    let mut container_style = ContainerStyle::default();
    let mut text_style = TextStyle::default();
    let mut text_flow_style = TextFlowStyle::default();
    let mut overflow = Overflow::default();
    let clamp = CssValue::Function(
        "clamp".into(),
        vec![
            vec![CssValue::Length(60.0, Unit::Px)],
            vec![CssValue::Length(8.4, Unit::Vw)],
            vec![CssValue::Length(100.0, Unit::Px)],
        ],
    );

    assert!(
        apply_declaration(
            "font-size",
            &clamp,
            &mut style,
            &mut container_style,
            &mut text_style,
            &mut text_flow_style,
            &Style::default(),
            &ContainerStyle::default(),
            &TextStyle::default(),
            &TextFlowStyle::default(),
            &mut overflow,
            ColorScheme::Light,
        )
        .is_some()
    );
    assert_eq!(text_flow_style.font_size, 100.0);
}

#[test]
fn unsupported_overflow_value_is_rejected() {
    let mut style = Style::default();
    let mut container_style = ContainerStyle::default();
    let mut text_style = TextStyle::default();
    let mut text_flow_style = TextFlowStyle::default();
    let mut overflow = Overflow::default();
    assert!(
        apply_declaration(
            "overflow",
            &CssValue::Number(1.0),
            &mut style,
            &mut container_style,
            &mut text_style,
            &mut text_flow_style,
            &Style::default(),
            &ContainerStyle::default(),
            &TextStyle::default(),
            &TextFlowStyle::default(),
            &mut overflow,
            ColorScheme::Light,
        )
        .is_none()
    );
    assert_eq!(overflow, Overflow::default());
}

fn layout_for(html: &str, css: &str) -> InfoNode {
    layout_and_info_for(html, css).1
}

fn layout_and_info_for(html: &str, css: &str) -> (LayoutNode, InfoNode) {
    let dom = HtmlParser::new(html).parse();
    let mut resolved = ResolvedStyles::default();
    if !css.is_empty() {
        let sheet = CssParser::new(css).parse().unwrap();
        resolved.extend(CssResolver::resolve(&sheet));
    }
    build_layout_and_info(
        &dom.root,
        &resolved,
        Arc::new(FallbackTextMeasurer),
        InheritedCss::default(),
        ElementChain::default(),
        ColorScheme::Light,
        ScriptingMode::default(),
    )
}

fn text_content(info: &InfoNode) -> String {
    let mut text = match &info.kind {
        NodeKind::Text { text, .. } => text.clone(),
        _ => String::new(),
    };
    for child in &info.children {
        text.push_str(&text_content(child));
    }
    text
}

#[test]
fn noscript_content_is_absent_from_layout_when_scripting_is_enabled() {
    let info = layout_for(
        "<html><body><p>before</p><noscript><p>fallback</p></noscript><p>after</p></body></html>",
        "",
    );
    let text = text_content(&info);
    assert!(text.contains("before"));
    assert!(text.contains("after"));
    assert!(!text.contains("fallback"));
}

#[test]
fn named_grid_area_css_controls_final_layout() {
    let html = r#"<html><body><div class="grid"><div class="header"></div><div class="sidebar"></div><div class="main"></div><div class="footer"></div></div></body></html>"#;
    let css = r#"
            .grid {
                display: grid;
                width: 300px;
                grid-template-areas: "header header" "sidebar main" "footer footer";
                grid-template-columns: 1fr 2fr;
                gap: 10px;
            }
            .grid > div { height: 20px; }
            .header { grid-area: header; }
            .sidebar { grid-area: sidebar; }
            .main { grid-area: main; }
            .footer { grid-area: footer; }
        "#;
    let (mut layout, _) = layout_and_info_for(html, css);
    ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

    fn find_grid(node: &LayoutNode) -> Option<&LayoutNode> {
        if node.style.display.inner() == Some(InnerDisplay::Grid) {
            return Some(node);
        }
        node.children
            .iter()
            .filter_map(LayoutChild::node)
            .find_map(find_grid)
    }
    let grid = find_grid(&layout).expect("grid container");
    let items: Vec<_> = grid.children.iter().filter_map(LayoutChild::node).collect();
    assert_eq!(items.len(), 4);
    assert_eq!(items[0].layout_box.width_box(), 300.0);
    assert_eq!(items[1].layout_box.iter().next().unwrap().border_box.x, 0.0);
    assert!((items[2].layout_box.iter().next().unwrap().border_box.x - 106.66667).abs() < 0.01);
    assert_eq!(items[3].layout_box.width_box(), 300.0);
    assert!(items[3].layout_box.iter().next().unwrap().border_box.y >= 60.0);
}

#[test]
fn flex_wrap_css_creates_multiple_lines() {
    let html = r#"<html><body><div class="flex"><div></div><div></div></div></body></html>"#;
    let css = r#"
            .flex {
                display: flex;
                width: 100px;
                flex-flow: row wrap;
                align-content: flex-start;
            }
            .flex > div {
                width: 60px;
                height: 20px;
            }
        "#;
    let (mut layout, _) = layout_and_info_for(html, css);
    ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

    fn find_flex(node: &LayoutNode) -> Option<&LayoutNode> {
        if node.style.display.inner() == Some(InnerDisplay::Flex) {
            return Some(node);
        }
        node.children
            .iter()
            .filter_map(LayoutChild::node)
            .find_map(find_flex)
    }
    let flex = find_flex(&layout).expect("flex container");
    let items: Vec<_> = flex.children.iter().filter_map(LayoutChild::node).collect();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].layout_box.iter().next().unwrap().border_box.y, 0.0);
    assert_eq!(
        items[1].layout_box.iter().next().unwrap().border_box.y,
        20.0
    );
}

#[test]
fn floated_carousel_slides_shrink_to_fixed_descendant_width() {
    let html = r#"
            <html><body>
                <div class="track">
                    <div class="slide"><div class="card"></div></div>
                    <div class="slide"><div class="card"></div></div>
                </div>
            </body></html>
        "#;
    let css = r#"
            .track { width: 1000px; }
            .slide { float: left; padding-right: 30px; }
            .card { width: 144px; height: 100px; }
        "#;
    let (mut layout, _) = layout_and_info_for(html, css);
    ui_layout::LayoutEngine::layout(&mut layout, 1200.0, 600.0);

    fn floated_children(node: &LayoutNode) -> Option<Vec<&LayoutNode>> {
        let children: Vec<_> = node
            .children
            .iter()
            .filter_map(LayoutChild::node)
            .filter(|child| {
                child.style.size.auto_behavior == AutoSizeBehavior::ShrinkToFit
                    && child.style.display.inner() == Some(InnerDisplay::FlowRoot)
            })
            .collect();
        if children.len() == 2 {
            return Some(children);
        }
        node.children
            .iter()
            .filter_map(LayoutChild::node)
            .find_map(floated_children)
    }

    let slides = floated_children(&layout).expect("two floated slides");
    let first = slides[0].layout_box.iter().next().unwrap();
    let second = slides[1].layout_box.iter().next().unwrap();
    assert_eq!(first.content_box.width, 144.0);
    assert_eq!(first.border_box.width, 174.0);
    assert_eq!(second.border_box.x, 174.0);
    assert_eq!(second.border_box.y, 0.0);
}

#[test]
fn adjacent_inline_blocks_advance_past_padding_and_margins() {
    let html = r#"
            <html><body><div class="row"><a>First</a><a>Second</a></div></body></html>
        "#;
    let css = r#"
            a { display: inline-block; padding: 0 12px; margin-right: 12px; }
        "#;
    let (mut layout, _) = layout_and_info_for(html, css);
    ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

    fn inline_blocks(node: &LayoutNode) -> Option<Vec<ui_layout::Rect>> {
        let boxes: Vec<_> = node
            .children
            .iter()
            .filter_map(LayoutChild::node)
            .filter(|child| child.style.display.inner() == Some(InnerDisplay::FlowRoot))
            .filter_map(|child| child.layout_box.iter().next().map(|model| model.border_box))
            .collect();
        if boxes.len() == 2 {
            return Some(boxes);
        }
        node.children
            .iter()
            .filter_map(LayoutChild::node)
            .find_map(inline_blocks)
    }

    correct_atomic_inline_spacing(&mut layout);

    let boxes = inline_blocks(&layout).expect("two inline blocks");
    assert!(boxes[1].x >= boxes[0].right() + 12.0);
}

#[test]
fn atomic_inline_block_starts_below_its_top_margin() {
    let html = r#"
            <html><body><main><div>Content</div></main></body></html>
        "#;
    let css = r#"
            main { display: inline-block; width: 100%; margin-top: 50px; }
        "#;
    let (mut layout, _) = layout_and_info_for(html, css);
    ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

    // Debug BEFORE correct_atomic_inline_spacing
    {
        fn find_flex_links(node: &LayoutNode) -> Option<(&LayoutNode, &LayoutNode)> {
            let links: Vec<_> = node
                .children
                .iter()
                .filter_map(LayoutChild::node)
                .filter(|child| child.style.display.outer() == Some(OuterDisplay::Block))
                .collect();
            if node.style.display.inner() == Some(InnerDisplay::Flex) && links.len() == 2 {
                return Some((links[0], links[1]));
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(find_flex_links)
        }
        if let Some((f, s)) = find_flex_links(&layout) {
            let fb = f.layout_box.iter().next().unwrap();
            let sb = s.layout_box.iter().next().unwrap();
            eprintln!(
                "PRE-CORRECT first bb: x={}, w={}, right={}",
                fb.border_box.x,
                fb.border_box.width,
                fb.border_box.right()
            );
            eprintln!(
                "PRE-CORRECT second bb: x={}, w={}",
                sb.border_box.x, sb.border_box.width
            );
            // Check the button inside first
            if let Some(btn) = f.children.iter().filter_map(LayoutChild::node).next() {
                if let Some(bm) = btn.layout_box.iter().next() {
                    eprintln!(
                        "PRE-CORRECT button bb: x={}, w={}",
                        bm.border_box.x, bm.border_box.width
                    );
                }
            }
        }
    }

    correct_atomic_inline_spacing(&mut layout);

    fn main_box(node: &LayoutNode) -> Option<ui_layout::Rect> {
        if node.style.display
            == (Display::OutsideInner {
                outer: OuterDisplay::Inline,
                inner: InnerDisplay::FlowRoot,
            })
            && node.style.spacing.margin_top == LengthOrAuto::Length(Length::Px(50.0))
        {
            return node.layout_box.iter().next().map(|model| model.border_box);
        }
        node.children
            .iter()
            .filter_map(LayoutChild::node)
            .find_map(main_box)
    }

    let main = main_box(&layout).expect("main inline block");
    assert_eq!(main.y, 50.0);
}

#[test]
fn atomic_inline_after_block_starts_below_the_block_margin() {
    let html = r#"
            <html><body><div class="copy">
                <p class="overline">Status</p><h2>Heading</h2><p class="summary">Summary</p><a>Continue</a>
            </div></body></html>
        "#;
    let css = r#"
            .copy { width: 600px; text-align: center; }
            .copy .overline { height: 25px; margin: 0 0 18px; }
            .copy h2 { height: 58px; margin: 0 0 24px; }
            .copy .summary { width: 560px; height: 80px; margin: 0 0 28px; }
            .copy a { display: inline-flex; width: 200px; height: 30px; }
        "#;
    let (mut layout, info) = layout_and_info_for(html, css);
    ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);
    correct_atomic_inline_spacing_with_info(&mut layout, &info);

    fn box_with_width(node: &LayoutNode, width: f32) -> Option<ui_layout::Rect> {
        if node.style.size.width == LengthOrAuto::Length(Length::Px(width)) {
            return node.layout_box.iter().next().map(|model| model.border_box);
        }
        node.children
            .iter()
            .filter_map(LayoutChild::node)
            .find_map(|child| box_with_width(child, width))
    }

    let summary = box_with_width(&layout, 560.0).expect("summary");
    let action = box_with_width(&layout, 200.0).expect("inline action");
    assert_eq!(action.y, summary.bottom() + 28.0);
    assert_eq!(action.x, 200.0);
}

#[test]
fn indentation_before_block_does_not_create_anonymous_line() {
    let html = "<html><body><div>\n    <section></section>\n</div></body></html>";
    let css = "section { display: block; width: 100px; height: 20px; }";
    let (mut layout, _) = layout_and_info_for(html, css);
    ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

    fn section_box(node: &LayoutNode) -> Option<ui_layout::Rect> {
        if node.style.size.width == LengthOrAuto::Length(Length::Px(100.0)) {
            return node.layout_box.iter().next().map(|model| model.border_box);
        }
        node.children
            .iter()
            .filter_map(LayoutChild::node)
            .find_map(section_box)
    }

    assert_eq!(section_box(&layout).expect("section").y, 0.0);
}

#[test]
fn auto_flex_height_includes_child_vertical_margins() {
    let html = "<html><body><div class='row'><span></span></div></body></html>";
    let css = r#"
            .row { display: flex; }
            span { display: inline-block; width: 20px; height: 30px; margin: 10px 0; }
        "#;
    let (mut layout, _) = layout_and_info_for(html, css);
    ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

    // Debug BEFORE correct_atomic_inline_spacing
    {
        fn find_flex_links(node: &LayoutNode) -> Option<(&LayoutNode, &LayoutNode)> {
            let links: Vec<_> = node
                .children
                .iter()
                .filter_map(LayoutChild::node)
                .filter(|child| child.style.display.outer() == Some(OuterDisplay::Block))
                .collect();
            if node.style.display.inner() == Some(InnerDisplay::Flex) && links.len() == 2 {
                return Some((links[0], links[1]));
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(find_flex_links)
        }
        if let Some((f, s)) = find_flex_links(&layout) {
            let fb = f.layout_box.iter().next().unwrap();
            let sb = s.layout_box.iter().next().unwrap();
            eprintln!(
                "PRE-CORRECT first bb: x={}, w={}, right={}",
                fb.border_box.x,
                fb.border_box.width,
                fb.border_box.right()
            );
            eprintln!(
                "PRE-CORRECT second bb: x={}, w={}",
                sb.border_box.x, sb.border_box.width
            );
            // Check the button inside first
            if let Some(btn) = f.children.iter().filter_map(LayoutChild::node).next() {
                if let Some(bm) = btn.layout_box.iter().next() {
                    eprintln!(
                        "PRE-CORRECT button bb: x={}, w={}",
                        bm.border_box.x, bm.border_box.width
                    );
                }
            }
        }
    }

    correct_atomic_inline_spacing(&mut layout);

    fn flex_box(node: &LayoutNode) -> Option<ui_layout::Rect> {
        if node.style.display.inner() == Some(InnerDisplay::Flex) {
            return node.layout_box.iter().next().map(|model| model.border_box);
        }
        node.children
            .iter()
            .filter_map(LayoutChild::node)
            .find_map(flex_box)
    }

    assert!(flex_box(&layout).expect("flex row").height >= 50.0);
}

#[test]
fn grid_min_height_pushes_later_block_flow_content() {
    let html = "<html><body><header></header><main></main></body></html>";
    let css = r#"
            header { display: grid; min-height: 48px; }
            main { display: block; height: 20px; }
        "#;
    let (mut layout, _) = layout_and_info_for(html, css);
    ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

    fn children(node: &LayoutNode) -> Option<Vec<ui_layout::Rect>> {
        let boxes: Vec<_> = node
            .children
            .iter()
            .filter_map(LayoutChild::node)
            .filter(|child| child.style.display.outer() == Some(OuterDisplay::Block))
            .filter_map(|child| child.layout_box.iter().next().map(|model| model.border_box))
            .collect();
        if boxes.len() == 2 {
            return Some(boxes);
        }
        node.children
            .iter()
            .filter_map(LayoutChild::node)
            .find_map(children)
    }

    let boxes = children(&layout).expect("header and main");
    assert!(boxes[0].height >= 48.0);
    assert!(boxes[1].y >= boxes[0].bottom());
}

#[test]
fn grid_auto_track_can_measure_flex_contents() {
    let html = "<html><body><div class='grid'><a>A</a><nav><span></span><span></span></nav><a>B</a></div></body></html>";
    let css = r#"
            .grid { display: grid; width: 1024px; grid-template-columns: 1fr auto 1fr; }
            nav { display: flex; gap: 10px; }
            nav span { display: block; width: 100px; height: 10px; }
        "#;
    let (mut layout, _) = layout_and_info_for(html, css);
    ui_layout::LayoutEngine::layout(&mut layout, 1280.0, 600.0);
    assert!(constrain_auto_grid_track_items(&mut layout));
    ui_layout::LayoutEngine::layout(&mut layout, 1280.0, 600.0);

    fn grid(node: &LayoutNode) -> Option<&LayoutNode> {
        if node.style.display.inner() == Some(InnerDisplay::Grid) {
            return Some(node);
        }
        node.children
            .iter()
            .filter_map(LayoutChild::node)
            .find_map(grid)
    }
    let grid = grid(&layout).expect("grid");
    let items: Vec<_> = grid.children.iter().filter_map(LayoutChild::node).collect();
    assert_eq!(items[0].style.display.outer(), Some(OuterDisplay::Block));
    assert_eq!(items[2].style.display.outer(), Some(OuterDisplay::Block));
    let middle = items[1].layout_box.iter().next().expect("middle");
    assert!((middle.content_box.width - 210.0).abs() < 0.5);
    assert!(items[0].layout_box.width_box() > 400.0);
    assert!(items[2].layout_box.width_box() > 400.0);
}

#[test]
fn negative_grid_end_line_spans_to_the_last_explicit_track() {
    let html = "<html><body><div class='grid'><article class='large'></article><article></article></div></body></html>";
    let css = r#"
            .grid { display: grid; width: 600px; grid-template-columns: repeat(2, 1fr); }
            .large { grid-column: 1 / -1; height: 20px; }
        "#;
    let (mut layout, _) = layout_and_info_for(html, css);
    ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

    fn grid(layout: &LayoutNode) -> Option<&LayoutNode> {
        if layout.style.display.inner() == Some(InnerDisplay::Grid) {
            return Some(layout);
        }
        layout
            .children
            .iter()
            .filter_map(LayoutChild::node)
            .find_map(grid)
    }

    let grid = grid(&layout).expect("grid");
    let large = grid
        .children
        .iter()
        .filter_map(LayoutChild::node)
        .next()
        .expect("large grid item");
    assert_eq!(large.style.grid_column.start, Some(1));
    assert_eq!(
        large.style.grid_column.end,
        GridPlacementEnd::NegativeLine(1)
    );
    assert!((large.layout_box.width_box() - 600.0).abs() < 0.5);
}

#[test]
fn grid_justify_self_end_uses_the_end_of_its_track() {
    let html = "<html><body><div class='grid'><a>A</a><nav><span></span></nav><a class='end'>B</a></div></body></html>";
    let css = r#"
            .grid { display: grid; width: 300px; margin-left: 50px; grid-template-columns: 1fr auto 1fr; }
            nav { display: flex; }
            nav span { display: block; width: 100px; height: 10px; }
            .end { justify-self: end; }
        "#;
    let (mut layout, _) = layout_and_info_for(html, css);
    ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);
    assert!(constrain_auto_grid_track_items(&mut layout));
    ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);
    correct_atomic_inline_spacing(&mut layout);

    fn end_item(node: &LayoutNode) -> Option<ui_layout::Rect> {
        if node.style.item_style.justify_self == Some(JustifyItems::End) {
            return node.layout_box.iter().next().map(|model| model.border_box);
        }
        node.children
            .iter()
            .filter_map(LayoutChild::node)
            .find_map(end_item)
    }

    let item = end_item(&layout).expect("end-aligned item");
    assert!((item.right() - 300.0).abs() < 0.5, "item={item:?}");
}

#[test]
fn inline_flex_lays_out_direct_text_with_inherited_style() {
    let html = "<html><body><a class='action'>目指すこと<span>›</span></a></body></html>";
    let css = r#"
            .action { display: inline-flex; gap: 5px; color: #0066cc; font-size: 19px; }
            .action span { font-size: 22px; }
        "#;
    let (mut layout, info) = layout_and_info_for(html, css);
    ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);
    refresh_missing_text_layout_results(&mut layout, &info, (800.0, 600.0));

    fn inline_flex(layout: &LayoutNode) -> Option<&LayoutNode> {
        if layout.style.display.inner() == Some(InnerDisplay::Flex) {
            return Some(layout);
        }
        layout
            .children
            .iter()
            .filter_map(LayoutChild::node)
            .find_map(inline_flex)
    }
    let flex = inline_flex(&layout).expect("inline flex");
    let span = flex
        .children
        .iter()
        .filter_map(LayoutChild::node)
        .next()
        .expect("span flex item");
    assert_eq!(span.style.display.outer(), Some(OuterDisplay::Block));

    let label_style = text_style_for(&info, "目指すこと");
    assert_eq!(text_flow_style_for(&info, "目指すこと").font_size, 19.0);
    assert_eq!(label_style.color, Color(0, 102, 204, 255));

    fn text_id_for(info: &InfoNode, content: &str) -> Option<usize> {
        if let NodeKind::Text { text, text_id, .. } = &info.kind
            && text == content
        {
            return Some(*text_id);
        }
        info.children
            .iter()
            .find_map(|child| text_id_for(child, content))
    }

    let label_id = text_id_for(&info, "目指すこと").expect("label text id");
    let result = TextFlowLayouter::get_result(label_id).expect("label layout result");
    assert_eq!(result.line_texts, vec!["目指すこと"]);
    assert!(result.spans[0].line_pos.0 >= 0.0);
}

#[test]
fn flex_navigation_blockifies_and_spaces_inline_links() {
    let html = "<html><body><nav><a>目指す</a><a>違い</a><a>開発</a></nav></body></html>";
    let css = "nav { display: flex; gap: 30px; } a { display: inline; }";
    let (mut layout, _) = layout_and_info_for(html, css);
    ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

    // Debug BEFORE correct_atomic_inline_spacing
    {
        fn find_flex_links(node: &LayoutNode) -> Option<(&LayoutNode, &LayoutNode)> {
            let links: Vec<_> = node
                .children
                .iter()
                .filter_map(LayoutChild::node)
                .filter(|child| child.style.display.outer() == Some(OuterDisplay::Block))
                .collect();
            if node.style.display.inner() == Some(InnerDisplay::Flex) && links.len() == 2 {
                return Some((links[0], links[1]));
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(find_flex_links)
        }
        if let Some((f, s)) = find_flex_links(&layout) {
            let fb = f.layout_box.iter().next().unwrap();
            let sb = s.layout_box.iter().next().unwrap();
            eprintln!(
                "PRE-CORRECT first bb: x={}, w={}, right={}",
                fb.border_box.x,
                fb.border_box.width,
                fb.border_box.right()
            );
            eprintln!(
                "PRE-CORRECT second bb: x={}, w={}",
                sb.border_box.x, sb.border_box.width
            );
            // Check the button inside first
            if let Some(btn) = f.children.iter().filter_map(LayoutChild::node).next() {
                if let Some(bm) = btn.layout_box.iter().next() {
                    eprintln!(
                        "PRE-CORRECT button bb: x={}, w={}",
                        bm.border_box.x, bm.border_box.width
                    );
                }
            }
        }
    }

    correct_atomic_inline_spacing(&mut layout);

    fn navigation(layout: &LayoutNode) -> Option<&LayoutNode> {
        let links: Vec<_> = layout
            .children
            .iter()
            .filter_map(LayoutChild::node)
            .filter(|child| child.style.display.outer() == Some(OuterDisplay::Block))
            .collect();
        if layout.style.display.inner() == Some(InnerDisplay::Flex) && links.len() == 3 {
            return Some(layout);
        }
        layout
            .children
            .iter()
            .filter_map(LayoutChild::node)
            .find_map(navigation)
    }

    let nav = navigation(&layout).expect("flex navigation");
    let links: Vec<_> = nav
        .children
        .iter()
        .filter_map(LayoutChild::node)
        .filter_map(|child| child.layout_box.iter().next())
        .collect();
    assert_eq!(links.len(), 3);
    assert!(links[1].border_box.x >= links[0].border_box.right() + 29.5);
    assert!(links[2].border_box.x >= links[1].border_box.right() + 29.5);
}

#[test]
fn bottom_anchored_grid_repositions_after_min_height_growth() {
    let html = "<html><body><div class='parent'><div class='dialog'></div></div></body></html>";
    let css = r#"
            .parent { position: relative; width: 300px; height: 200px; }
            .dialog { position: absolute; right: 20px; bottom: -10px; display: grid; width: 80px; min-height: 100px; }
        "#;
    let (mut layout, _) = layout_and_info_for(html, css);
    ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

    // Debug BEFORE correct_atomic_inline_spacing
    {
        fn find_flex_links(node: &LayoutNode) -> Option<(&LayoutNode, &LayoutNode)> {
            let links: Vec<_> = node
                .children
                .iter()
                .filter_map(LayoutChild::node)
                .filter(|child| child.style.display.outer() == Some(OuterDisplay::Block))
                .collect();
            if node.style.display.inner() == Some(InnerDisplay::Flex) && links.len() == 2 {
                return Some((links[0], links[1]));
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(find_flex_links)
        }
        if let Some((f, s)) = find_flex_links(&layout) {
            let fb = f.layout_box.iter().next().unwrap();
            let sb = s.layout_box.iter().next().unwrap();
            eprintln!(
                "PRE-CORRECT first bb: x={}, w={}, right={}",
                fb.border_box.x,
                fb.border_box.width,
                fb.border_box.right()
            );
            eprintln!(
                "PRE-CORRECT second bb: x={}, w={}",
                sb.border_box.x, sb.border_box.width
            );
            // Check the button inside first
            if let Some(btn) = f.children.iter().filter_map(LayoutChild::node).next() {
                if let Some(bm) = btn.layout_box.iter().next() {
                    eprintln!(
                        "PRE-CORRECT button bb: x={}, w={}",
                        bm.border_box.x, bm.border_box.width
                    );
                }
            }
        }
    }

    correct_atomic_inline_spacing(&mut layout);

    fn dialog(node: &LayoutNode) -> Option<ui_layout::Rect> {
        if node.style.position.kind == Position::Absolute {
            return node.layout_box.iter().next().map(|model| model.border_box);
        }
        node.children
            .iter()
            .filter_map(LayoutChild::node)
            .find_map(dialog)
    }

    let dialog = dialog(&layout).expect("dialog");
    assert!(dialog.height >= 100.0);
    assert!((dialog.y - 110.0).abs() < 0.5, "dialog={dialog:?}");
}

#[test]
fn border_box_button_keeps_declared_size_with_padding() {
    let html = "<html><body><button>Search</button></body></html>";
    let css = "button { display: inline-block; box-sizing: border-box; width: 40px; height: 40px; padding: 12px 16px; }";
    let (mut layout, _) = layout_and_info_for(html, css);
    ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

    fn button_box(node: &LayoutNode) -> Option<ui_layout::Rect> {
        if node.style.box_sizing == BoxSizing::BorderBox
            && node.style.size.width == LengthOrAuto::Length(Length::Px(40.0))
        {
            return node.layout_box.iter().next().map(|model| model.border_box);
        }
        node.children
            .iter()
            .filter_map(LayoutChild::node)
            .find_map(button_box)
    }

    let button = button_box(&layout).expect("border-box button");
    assert_eq!((button.width, button.height), (40.0, 40.0));
}

#[test]
fn full_width_inline_blocks_wrap_onto_separate_lines() {
    let html = r#"
            <html><body><main><section>First</section><section>Second</section></main></body></html>
        "#;
    let css = r#"
            main { width: 300px; }
            section { display: inline-block; width: 100%; height: 40px; margin-bottom: 10px; }
        "#;
    let (mut layout, _) = layout_and_info_for(html, css);
    ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

    // Debug BEFORE correct_atomic_inline_spacing
    {
        fn find_flex_links(node: &LayoutNode) -> Option<(&LayoutNode, &LayoutNode)> {
            let links: Vec<_> = node
                .children
                .iter()
                .filter_map(LayoutChild::node)
                .filter(|child| child.style.display.outer() == Some(OuterDisplay::Block))
                .collect();
            if node.style.display.inner() == Some(InnerDisplay::Flex) && links.len() == 2 {
                return Some((links[0], links[1]));
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(find_flex_links)
        }
        if let Some((f, s)) = find_flex_links(&layout) {
            let fb = f.layout_box.iter().next().unwrap();
            let sb = s.layout_box.iter().next().unwrap();
            eprintln!(
                "PRE-CORRECT first bb: x={}, w={}, right={}",
                fb.border_box.x,
                fb.border_box.width,
                fb.border_box.right()
            );
            eprintln!(
                "PRE-CORRECT second bb: x={}, w={}",
                sb.border_box.x, sb.border_box.width
            );
            // Check the button inside first
            if let Some(btn) = f.children.iter().filter_map(LayoutChild::node).next() {
                if let Some(bm) = btn.layout_box.iter().next() {
                    eprintln!(
                        "PRE-CORRECT button bb: x={}, w={}",
                        bm.border_box.x, bm.border_box.width
                    );
                }
            }
        }
    }

    correct_atomic_inline_spacing(&mut layout);

    fn sections(node: &LayoutNode) -> Option<Vec<ui_layout::Rect>> {
        let boxes: Vec<_> = node
            .children
            .iter()
            .filter_map(LayoutChild::node)
            .filter(|child| {
                child.style.display
                    == (Display::OutsideInner {
                        outer: OuterDisplay::Inline,
                        inner: InnerDisplay::FlowRoot,
                    })
            })
            .filter_map(|child| child.layout_box.iter().next().map(|model| model.border_box))
            .collect();
        if boxes.len() == 2 {
            return Some(boxes);
        }
        node.children
            .iter()
            .filter_map(LayoutChild::node)
            .find_map(sections)
    }

    let boxes = sections(&layout).expect("two inline blocks");
    assert!((boxes[0].x - boxes[1].x).abs() < 0.5);
    assert!(boxes[1].y >= boxes[0].bottom() + 10.0);
}

#[test]
fn auto_flex_container_expands_to_corrected_inline_margin_boxes() {
    let html = r#"
            <html><body><div class="column"><div class="row"><a>First</a><a>Second</a></div></div></body></html>
        "#;
    let css = r#"
            .column { display: flex; flex-direction: column; align-items: center; }
            .row { display: flex; }
            a { display: inline-block; padding: 0 12px; margin-right: 12px; }
        "#;
    let (mut layout, _) = layout_and_info_for(html, css);
    ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

    // Debug BEFORE correct_atomic_inline_spacing
    {
        fn find_flex_links(node: &LayoutNode) -> Option<(&LayoutNode, &LayoutNode)> {
            let links: Vec<_> = node
                .children
                .iter()
                .filter_map(LayoutChild::node)
                .filter(|child| child.style.display.outer() == Some(OuterDisplay::Block))
                .collect();
            if node.style.display.inner() == Some(InnerDisplay::Flex) && links.len() == 2 {
                return Some((links[0], links[1]));
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(find_flex_links)
        }
        if let Some((f, s)) = find_flex_links(&layout) {
            let fb = f.layout_box.iter().next().unwrap();
            let sb = s.layout_box.iter().next().unwrap();
            eprintln!(
                "PRE-CORRECT first bb: x={}, w={}, right={}",
                fb.border_box.x,
                fb.border_box.width,
                fb.border_box.right()
            );
            eprintln!(
                "PRE-CORRECT second bb: x={}, w={}",
                sb.border_box.x, sb.border_box.width
            );
            // Check the button inside first
            if let Some(btn) = f.children.iter().filter_map(LayoutChild::node).next() {
                if let Some(bm) = btn.layout_box.iter().next() {
                    eprintln!(
                        "PRE-CORRECT button bb: x={}, w={}",
                        bm.border_box.x, bm.border_box.width
                    );
                }
            }
        }
    }

    correct_atomic_inline_spacing(&mut layout);

    fn flex_row(node: &LayoutNode) -> Option<&LayoutNode> {
        let atomic_children = node
            .children
            .iter()
            .filter_map(LayoutChild::node)
            .filter(|child| child.style.display.inner() == Some(InnerDisplay::FlowRoot))
            .count();
        if node.style.display.inner() == Some(InnerDisplay::Flex) && atomic_children == 2 {
            return Some(node);
        }
        node.children
            .iter()
            .filter_map(LayoutChild::node)
            .find_map(flex_row)
    }

    let row = flex_row(&layout).expect("flex row");
    let row_box = row.layout_box.iter().next().unwrap();
    let last = row
        .children
        .iter()
        .filter_map(LayoutChild::node)
        .last()
        .unwrap();
    let last_box = last.layout_box.iter().next().unwrap();
    let required_right = last_box.border_box.right() + 12.0;
    assert!(row_box.content_box.right() >= required_right);
}

#[test]
fn inline_flex_item_wraps_padded_atomic_child_without_overlap() {
    let html = r#"
            <html><body><div class="bar">
                <a><div class="button">One</div></a><a><div class="button">Two</div></a>
            </div></body></html>
        "#;
    let css = r#"
            .bar { display: flex; }
            a { display: inline; }
            .button { display: inline-block; margin: 0 8px; padding: 8px 24px; }
        "#;
    let (mut layout, _) = layout_and_info_for(html, css);
    ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);
    correct_atomic_inline_spacing(&mut layout);

    fn flex_links(node: &LayoutNode) -> Option<Vec<&LayoutNode>> {
        let links: Vec<_> = node
            .children
            .iter()
            .filter_map(LayoutChild::node)
            .filter(|child| child.style.display.outer() == Some(OuterDisplay::Block))
            .collect();
        if node.style.display.inner() == Some(InnerDisplay::Flex) && links.len() == 2 {
            return Some(links);
        }
        node.children
            .iter()
            .filter_map(LayoutChild::node)
            .find_map(flex_links)
    }

    let links = flex_links(&layout).expect("two inline flex items");
    let first = links[0].layout_box.iter().next().unwrap();
    let second = links[1].layout_box.iter().next().unwrap();
    let first_button = links[0]
        .children
        .iter()
        .filter_map(LayoutChild::node)
        .next()
        .unwrap()
        .layout_box
        .iter()
        .next()
        .unwrap();
    assert!(
        first.content_box.width >= first_button.border_box.width + 8.0,
        "first flex item does not contain its padded atomic child and margin"
    );
    assert!(second.border_box.x >= first.border_box.right());
}

#[test]
fn structural_selectors_apply_styles_from_html_context() {
    let html = r#"
            <html><body>
                <ul><li>first</li><li>second</li><li>third</li></ul>
                <h2>heading</h2><p>adjacent</p>
            </body></html>
        "#;
    let css = r#"
            li:first-child { color: #ff0000; }
            li:nth-child(2) { color: #008000; }
            li:last-child:not(.skip) { color: #0000ff; }
            h2 + p { color: #663399; }
        "#;
    let info = layout_for(html, css);

    assert_eq!(text_style_for(&info, "first").color, Color(255, 0, 0, 255));
    assert_eq!(text_style_for(&info, "second").color, Color(0, 128, 0, 255));
    assert_eq!(text_style_for(&info, "third").color, Color(0, 0, 255, 255));
    assert_eq!(
        text_style_for(&info, "adjacent").color,
        Color(102, 51, 153, 255)
    );
}

#[test]
fn inline_image_keeps_intrinsic_dimensions_after_layout() {
    let dom = HtmlParser::new(
        r#"<html><body><div><img src="profile.png" alt="profile"></div></body></html>"#,
    )
    .parse();
    let mut images = HashMap::new();
    images.insert(
        "profile.png".to_string(),
        Image::from_rgba(460, 460, vec![255; 460 * 460 * 4]).unwrap(),
    );
    let stylesheet = CssParser::new("img { display: inline-block; }")
        .parse()
        .unwrap();
    let resolved_styles = CssResolver::resolve(&stylesheet);
    let (mut layout, _) = build_layout_and_info_with_images(
        &dom.root,
        &resolved_styles,
        Arc::new(FallbackTextMeasurer),
        InheritedCss::default(),
        ElementChain::default(),
        ColorScheme::Light,
        ScriptingMode::default(),
        &images,
    );
    ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

    fn image_layout_size(node: &LayoutNode) -> Option<(f32, f32)> {
        if node.style.size.auto_behavior == AutoSizeBehavior::ShrinkToFit {
            return node
                .layout_box
                .iter()
                .next()
                .map(|box_model| (box_model.content_box.width, box_model.content_box.height));
        }
        node.children.iter().find_map(|child| match child {
            LayoutChild::Node(child) => image_layout_size(child),
            _ => None,
        })
    }

    assert_eq!(image_layout_size(&layout), Some((460.0, 460.0)));
}

/// Returns the body element's layout children, unwrapping the document
/// and `<html>` wrapper nodes.
fn body_layout_children<'a>(mut node: &'a LayoutNode) -> &'a [LayoutChild] {
    while let [LayoutChild::Node(child)] = node.children.as_slice() {
        node = child;
    }
    &node.children
}

/// Recursively counts whitespace-only text nodes in the `InfoNode` tree,
/// which mirrors the DOM (whitespace stays a separate node, unlike the
/// layout where it may merge into an adjacent run). This is the reliable
/// way to assert whether a whitespace node was dropped.
fn count_whitespace_text_info(info: &InfoNode) -> usize {
    let mut n = 0;
    if let NodeKind::Text { text, .. } = &info.kind {
        if text.chars().all(is_css_whitespace) {
            n += 1;
        }
    }
    n + info
        .children
        .iter()
        .map(count_whitespace_text_info)
        .sum::<usize>()
}

#[test]
fn whitespace_text_between_block_siblings_is_dropped() {
    let html = "<html><body><div>a</div>\n  <div>b</div></body></html>";
    let (layout, _) = layout_and_info_for(html, "");

    let children = body_layout_children(&layout);
    assert_eq!(children.len(), 2, "whitespace text must be dropped");
    assert!(
        children.iter().all(|child| child.node().is_some()),
        "only the two block divs remain"
    );
}

#[test]
fn whitespace_text_adjacent_to_inline_siblings_is_kept() {
    let html = "<html><body><span>a</span> <span>b</span></body></html>";
    let (layout, _) = layout_and_info_for(html, "span { display: inline; }");

    let children = body_layout_children(&layout);
    assert_eq!(children.len(), 3);
    assert!(
        matches!(&children[1], LayoutChild::Custom(_)),
        "space between inline spans must be kept"
    );
}

#[test]
fn whitespace_text_next_to_block_is_dropped() {
    // With the "side" rule, whitespace adjacent to a block on either side
    // is dropped even when the other sibling is inline.
    let html = "<html><body><div>a</div> <span>b</span></body></html>";
    let (_, info) = layout_and_info_for(html, "span { display: inline; }");

    assert_eq!(
        count_whitespace_text_info(&info),
        0,
        "whitespace next to a block must be dropped"
    );
}

#[test]
fn whitespace_text_trailing_after_block_is_dropped() {
    // Trailing whitespace after a single block is adjacent to that block,
    // so it must be dropped.
    let html = "<html><body><div>a</div> \n</body></html>";
    let (_, info) = layout_and_info_for(html, "");

    assert_eq!(
        count_whitespace_text_info(&info),
        0,
        "trailing whitespace after a single block must be dropped"
    );
}

#[test]
fn whitespace_text_around_br_between_block_siblings_is_dropped() {
    let html = "<html><body><div>a</div> \n<br>\n <div>b</div></body></html>";
    let (layout, _) = layout_and_info_for(html, "");

    let children = body_layout_children(&layout);
    assert_eq!(children.len(), 3, "only the two divs and the <br> remain");
    assert!(matches!(&children[0], LayoutChild::Node(_)));
    assert!(
        matches!(&children[1], LayoutChild::Fragment(_)),
        "<br> itself must be kept"
    );
    assert!(matches!(&children[2], LayoutChild::Node(_)));
}

#[test]
fn whitespace_text_after_br_is_dropped() {
    let html = "<html><body><p>a <br> </p></body></html>";
    let (layout, _) = layout_and_info_for(html, "");

    let children = body_layout_children(&layout);
    assert_eq!(children.len(), 2, "whitespace after <br> must be dropped");
    assert!(matches!(&children[0], LayoutChild::Custom(_)));
    assert!(matches!(&children[1], LayoutChild::Fragment(_)));
}

#[test]
fn whitespace_text_before_br_is_dropped() {
    let html = "<html><body><p> <br>b</p></body></html>";
    let (layout, _) = layout_and_info_for(html, "");

    let children = body_layout_children(&layout);
    assert_eq!(children.len(), 2, "whitespace before <br> must be dropped");
    assert!(matches!(&children[0], LayoutChild::Fragment(_)));
    assert!(matches!(&children[1], LayoutChild::Custom(_)));
}

#[test]
fn whitespace_crossing_display_none_between_blocks_is_dropped() {
    // A `display:none` element does not participate in layout, so the
    // whitespace on either side of it is still adjacent to a block once
    // the none element is skipped.
    let html =
        "<html><body><div>a</div> <span class=\"hidden\">x</span> <div>b</div></body></html>";
    let (layout, _) = layout_and_info_for(html, ".hidden { display: none; }");

    let children = body_layout_children(&layout);
    // The none span stays in the tree; only the two whitespace text nodes
    // are dropped (3 children: div, none span, div).
    assert_eq!(children.len(), 3, "two whitespace nodes must be dropped");
    assert!(
        children.iter().all(|child| child.node().is_some()),
        "no stray whitespace inline boxes remain"
    );
}

#[test]
fn whitespace_crossing_display_none_between_inlines_is_kept() {
    // Crossing a `display:none` element reveals inline spans on both
    // sides, so the whitespace is not adjacent to any block or `<br>` and
    // must be kept.
    let html =
        "<html><body><span>a</span> <span class=\"hidden\">x</span> <span>b</span></body></html>";
    let (_, info) =
        layout_and_info_for(html, "span { display: inline; } .hidden { display: none; }");

    // `count_whitespace_text_info` inspects the DOM-shaped info tree, so the
    // kept whitespace survives as its own node regardless of anonymous-block
    // wrapping in the layout.
    assert_eq!(
        count_whitespace_text_info(&info),
        2,
        "whitespace across a none element between inlines must be kept"
    );
}

/// Depth-first search for the first [`NodeKind::Text`] whose content is
/// `content`, returning a clone of its style.
fn text_style_for(info: &InfoNode, content: &str) -> TextStyle {
    fn walk(node: &InfoNode, content: &str) -> Option<TextStyle> {
        if let NodeKind::Text {
            style,
            text: actual,
            ..
        } = &node.kind
            && actual == content
        {
            return Some(style.clone());
        }
        node.children.iter().find_map(|child| walk(child, content))
    }
    walk(info, content).expect("text node with expected content must exist")
}

fn text_flow_style_for(info: &InfoNode, content: &str) -> TextFlowStyle {
    fn walk(node: &InfoNode, content: &str) -> Option<TextFlowStyle> {
        if let NodeKind::Text {
            flow_style,
            text: actual,
            ..
        } = &node.kind
            && actual == content
        {
            return Some(*flow_style);
        }
        node.children.iter().find_map(|child| walk(child, content))
    }
    walk(info, content).expect("text node with expected content must exist")
}

#[test]
fn inline_style_overrides_stylesheet_rule() {
    // A stylesheet sets color to blue; the inline attribute must win.
    let html = r#"<html><body><p id="x" style="color: red;">hello</p></body></html>"#;
    let info = layout_for(html, "p { color: blue; }");

    assert_eq!(text_style_for(&info, "hello").color, Color(255, 0, 0, 255));
}

#[test]
fn custom_properties_cascade_across_rules_and_inherit() {
    let html = r#"
            <html><body>
                <section><p>root theme</p></section>
                <section class="alternate"><p>alternate theme</p></section>
            </body></html>
        "#;
    let css = r#"
            :root { --scratch-accent: #855cd6; }
            p { color: var(--scratch-accent); }
            .alternate { --scratch-accent: #4c97ff; }
        "#;
    let info = layout_for(html, css);

    assert_eq!(
        text_style_for(&info, "root theme").color,
        Color(133, 92, 214, 255)
    );
    assert_eq!(
        text_style_for(&info, "alternate theme").color,
        Color(76, 151, 255, 255)
    );
}

#[test]
fn inline_declaration_can_use_inherited_custom_property() {
    let html = r#"
            <html><body style="--accent: #ff6680">
                <p style="color: var(--accent)">inline theme</p>
            </body></html>
        "#;
    let info = layout_for(html, "");

    assert_eq!(
        text_style_for(&info, "inline theme").color,
        Color(255, 102, 128, 255)
    );
}

#[test]
fn inline_style_non_important_loses_to_important_stylesheet() {
    let html = r#"<html><body><p id="x" style="color: red;">hello</p></body></html>"#;
    let info = layout_for(html, "p { color: blue !important; }");

    assert_eq!(text_style_for(&info, "hello").color, Color(0, 0, 255, 255));
}

#[test]
fn inline_style_important_beats_stylesheet_important() {
    let html = r#"<html><body><p id="x" style="color: red !important;">hello</p></body></html>"#;
    let info = layout_for(html, "p { color: blue !important; }");

    assert_eq!(text_style_for(&info, "hello").color, Color(255, 0, 0, 255));
}

#[test]
fn inline_style_sets_container_background() {
    let html =
        r#"<html><body><div style="background-color: rgb(0, 128, 0);">x</div></body></html>"#;
    let info = layout_for(html, "");

    fn find_div(node: &InfoNode) -> Option<&ContainerStyle> {
        if let NodeKind::Container { style, .. } = &node.kind
            && style.background != Background::default()
        {
            return Some(style);
        }
        node.children.iter().find_map(find_div)
    }
    let style = find_div(&info).expect("div container with background exists");
    assert_eq!(style.background, Background::Color(Color(0, 128, 0, 255)));
}

#[test]
fn hsl_percentage_background_is_resolved() {
    let info = layout_for(
        r#"<html><body><div id="view">x</div></body></html>"#,
        "#view { background-color: hsl(0, 0%, 99%); }",
    );

    fn find_background(node: &InfoNode) -> Option<Color> {
        if let NodeKind::Container { style, .. } = &node.kind
            && let Background::Color(color) = style.background
            && color.3 > 0
        {
            return Some(color);
        }
        node.children.iter().find_map(find_background)
    }
    assert_eq!(find_background(&info), Some(Color(252, 252, 252, 255)));
}

fn resolve_color(value: CssValue) -> Color {
    resolve_css_color("test", &value, ColorScheme::Light).expect("color resolves")
}

#[test]
fn hsla_accepts_percentage_channels_and_alpha() {
    assert_eq!(
        resolve_color(CssValue::Function(
            "hsla".into(),
            vec![vec![
                CssValue::Length(120.0, Unit::Deg),
                CssValue::Length(100.0, Unit::Percent),
                CssValue::Length(25.0, Unit::Percent),
                CssValue::Keyword("/".into()),
                CssValue::Length(50.0, Unit::Percent),
            ]],
        )),
        Color(0, 128, 0, 128)
    );
}

#[test]
fn color_mix_in_srgb_blends_weights() {
    let mixed = resolve_color(CssValue::Function(
        "color-mix".into(),
        vec![
            vec![
                CssValue::Keyword("in".into()),
                CssValue::Keyword("srgb".into()),
            ],
            vec![CssValue::Keyword("red".into())],
            vec![CssValue::Keyword("blue".into())],
        ],
    ));
    // 50/50 of red and blue in linear sRGB.
    assert_eq!(mixed, Color(188, 0, 188, 255));
}

#[test]
fn color_mix_with_percentages_and_missing_weight() {
    let mixed = resolve_color(CssValue::Function(
        "color-mix".into(),
        vec![
            vec![
                CssValue::Keyword("in".into()),
                CssValue::Keyword("srgb".into()),
            ],
            vec![
                CssValue::Keyword("red".into()),
                CssValue::Length(25.0, Unit::Percent),
            ],
            vec![CssValue::Keyword("blue".into())],
        ],
    ));
    // red 25% + blue (missing weight takes the remaining 75%).
    assert_eq!(mixed, Color(137, 0, 225, 255));
}

#[test]
fn color_mix_in_lch_produces_purple() {
    let mixed = resolve_color(CssValue::Function(
        "color-mix".into(),
        vec![
            vec![
                CssValue::Keyword("in".into()),
                CssValue::Keyword("lch".into()),
            ],
            vec![CssValue::Keyword("red".into())],
            vec![CssValue::Keyword("blue".into())],
        ],
    ));
    // Mixing red and blue in LCH stays on the purple hue arc.
    assert!(
        mixed.0 > 0 && mixed.2 > 0,
        "purple has red and blue: {mixed:?}"
    );
    assert!(mixed.1 < 50, "not green: {mixed:?}");
    assert_eq!(mixed.3, 255, "alpha is preserved");
    assert_ne!(mixed, Color(255, 0, 0, 255));
    assert_ne!(mixed, Color(0, 0, 255, 255));
}

#[test]
fn color_mix_alpha_is_premultiplied() {
    let mixed = resolve_color(CssValue::Function(
        "color-mix".into(),
        vec![
            vec![
                CssValue::Keyword("in".into()),
                CssValue::Keyword("srgb".into()),
            ],
            vec![CssValue::Keyword("transparent".into())],
            vec![CssValue::Keyword("blue".into())],
        ],
    ));
    // transparent is (0,0,0,0); mixing with opaque blue gives half alpha.
    assert_eq!(mixed.3, 128);
}

#[test]
fn conic_gradient_parses_stops_and_kind() {
    let args = vec![
        CssValue::Keyword("red".into()),
        CssValue::Length(0.0, Unit::Deg),
        CssValue::Keyword("red".into()),
        CssValue::Length(0.0, Unit::Deg),
        CssValue::Length(1.0, Unit::Deg),
        CssValue::Keyword("red".into()),
        CssValue::Length(2.0, Unit::Deg),
    ];
    let gradient = parse_gradient(
        "conic-gradient",
        &args,
        &TextStyle::default(),
        &TextFlowStyle::default(),
        ColorScheme::Light,
    )
    .expect("conic gradient parses");

    assert!(matches!(
        gradient.kind,
        GradientKind::Conic {
            angle: 0.0,
            position: (0.5, 0.5)
        }
    ));
    assert_eq!(gradient.stops.len(), 4);
    for (stop, expected) in gradient
        .stops
        .iter()
        .zip([0.0f32, 0.0, 1.0 / 360.0, 2.0 / 360.0])
    {
        assert_eq!(stop.position, Some(expected));
        assert_eq!(stop.color, Color(255, 0, 0, 255));
    }
}

#[test]
fn conic_gradient_background_shorthand() {
    let container = apply_container_property(
        "background",
        CssValue::Function(
            "conic-gradient".into(),
            vec![
                vec![CssValue::Keyword("red".into())],
                vec![CssValue::Length(0.0, Unit::Deg)],
                vec![CssValue::Keyword("blue".into())],
                vec![CssValue::Length(180.0, Unit::Deg)],
            ],
        ),
    );
    let Background::Gradient(gradient) = container.background else {
        panic!("expected gradient background");
    };
    assert!(matches!(
        gradient.kind,
        GradientKind::Conic {
            angle: 0.0,
            position: (0.5, 0.5)
        }
    ));
    assert_eq!(gradient.stops.len(), 2);
}

#[test]
fn background_image_shorthand_keeps_its_fallback_color() {
    let container = apply_container_property(
        "background",
        CssValue::List(vec![
            CssValue::Function(
                "url".into(),
                vec![vec![CssValue::String("/images/caret.svg".into())]],
            ),
            CssValue::Keyword("no-repeat".into()),
            CssValue::Keyword("right".into()),
            CssValue::Keyword("center".into()),
            CssValue::Keyword("white".into()),
        ]),
    );
    assert!(matches!(
        container.background,
        Background::Image {
            source,
            image: None,
            color: Color(255, 255, 255, 255)
        } if source == "/images/caret.svg"
    ));
}

#[test]
fn background_none_resolves_to_transparent() {
    let container = apply_container_property("background", CssValue::Keyword("none".into()));
    assert_eq!(container.background, Background::default());
}

#[test]
fn background_image_longhand_preserves_background_color() {
    let mut style = Style::default();
    let mut container = ContainerStyle::default();
    let mut text = TextStyle::default();
    let mut text_flow = TextFlowStyle::default();
    let mut overflow = Overflow::default();
    for (name, value) in [
        (
            "background-color",
            CssValue::Keyword("rebeccapurple".into()),
        ),
        (
            "background-image",
            CssValue::Function(
                "url".into(),
                vec![vec![CssValue::String("/images/hero.svg".into())]],
            ),
        ),
    ] {
        apply_declaration(
            name,
            &value,
            &mut style,
            &mut container,
            &mut text,
            &mut text_flow,
            &Style::default(),
            &ContainerStyle::default(),
            &TextStyle::default(),
            &TextFlowStyle::default(),
            &mut overflow,
            ColorScheme::Light,
        )
        .expect("background declaration is accepted");
    }
    assert!(matches!(
        container.background,
        Background::Image {
            source,
            color: Color(102, 51, 153, 255),
            ..
        } if source == "/images/hero.svg"
    ));
}

#[test]
fn scratch_background_geometry_longhands_are_parsed() {
    let mut style = Style::default();
    let mut container = ContainerStyle::default();
    let mut text = TextStyle::default();
    let mut text_flow = TextFlowStyle::default();
    let mut overflow = Overflow::default();
    for (name, value) in [
        ("background-repeat", CssValue::Keyword("no-repeat".into())),
        (
            "background-size",
            CssValue::List(vec![
                CssValue::Length(624.0, Unit::Px),
                CssValue::Length(325.0, Unit::Px),
            ]),
        ),
        ("background-position", CssValue::Keyword("right".into())),
    ] {
        apply_declaration(
            name,
            &value,
            &mut style,
            &mut container,
            &mut text,
            &mut text_flow,
            &Style::default(),
            &ContainerStyle::default(),
            &TextStyle::default(),
            &TextFlowStyle::default(),
            &mut overflow,
            ColorScheme::Light,
        )
        .expect("Scratch background declaration is accepted");
    }
    assert_eq!(container.background_repeat, BackgroundRepeat::NoRepeat);
    assert_eq!(
        container.background_size,
        BackgroundSize::Explicit {
            width: BackgroundDimension::Length(624.0),
            height: BackgroundDimension::Length(325.0),
        }
    );
    assert_eq!(
        container.background_position,
        BackgroundPosition {
            x: BackgroundPositionAxis::End(BackgroundOffset::Zero),
            y: BackgroundPositionAxis::Center(BackgroundOffset::Zero),
        }
    );
}

#[test]
fn scratch_responsive_background_position_is_parsed() {
    let container = apply_container_property(
        "background-position",
        CssValue::List(vec![
            CssValue::Keyword("bottom".into()),
            CssValue::Length(32.0, Unit::Px),
            CssValue::Keyword("right".into()),
            CssValue::Length(50.0, Unit::Percent),
        ]),
    );
    assert_eq!(
        container.background_position,
        BackgroundPosition {
            x: BackgroundPositionAxis::End(BackgroundOffset::Percent(0.5)),
            y: BackgroundPositionAxis::End(BackgroundOffset::Length(32.0)),
        }
    );

    let container = apply_container_property("background-size", CssValue::Length(40.0, Unit::Rem));
    assert_eq!(
        container.background_size,
        BackgroundSize::Explicit {
            width: BackgroundDimension::Length(640.0),
            height: BackgroundDimension::Auto,
        }
    );
}

#[test]
fn linear_gradient_calc_position_resolves() {
    let args = vec![
        CssValue::Keyword("red".into()),
        CssValue::Length(0.0, Unit::Percent),
        CssValue::Keyword("blue".into()),
        CssValue::Function(
            "calc".into(),
            vec![vec![
                CssValue::Length(50.0, Unit::Percent),
                CssValue::Keyword("+".into()),
                CssValue::Length(10.0, Unit::Percent),
            ]],
        ),
        CssValue::Keyword("green".into()),
        CssValue::Length(100.0, Unit::Percent),
    ];
    let gradient = parse_gradient(
        "linear-gradient",
        &args,
        &TextStyle::default(),
        &TextFlowStyle::default(),
        ColorScheme::Light,
    )
    .expect("gradient parses");
    assert_eq!(gradient.stops[0].position, Some(0.0));
    assert_eq!(gradient.stops[1].position, Some(0.6));
    assert_eq!(gradient.stops[2].position, Some(1.0));
}

#[test]
fn linear_gradient_calc_negative_position_clamps() {
    let args = vec![
        CssValue::Keyword("red".into()),
        CssValue::Function(
            "calc".into(),
            vec![vec![
                CssValue::Length(50.0, Unit::Percent),
                CssValue::Keyword("*".into()),
                CssValue::Number(-1.0),
            ]],
        ),
        CssValue::Keyword("blue".into()),
        CssValue::Length(100.0, Unit::Percent),
    ];
    let gradient = parse_gradient(
        "linear-gradient",
        &args,
        &TextStyle::default(),
        &TextFlowStyle::default(),
        ColorScheme::Light,
    )
    .expect("gradient parses");
    assert_eq!(gradient.stops[0].position, Some(0.0));
}

#[test]
fn linear_gradient_calc_zero_position() {
    let args = vec![
        CssValue::Keyword("red".into()),
        CssValue::Function(
            "calc".into(),
            vec![vec![CssValue::Length(0.0, Unit::Percent)]],
        ),
        CssValue::Keyword("blue".into()),
        CssValue::Length(100.0, Unit::Percent),
    ];
    let gradient = parse_gradient(
        "linear-gradient",
        &args,
        &TextStyle::default(),
        &TextFlowStyle::default(),
        ColorScheme::Light,
    )
    .expect("gradient parses");
    assert_eq!(gradient.stops[0].position, Some(0.0));
}

#[test]
fn repeating_gradient_parses_through_background_shorthand() {
    let container = apply_container_property(
        "background",
        CssValue::Function(
            "repeating-linear-gradient".into(),
            vec![
                vec![CssValue::Length(90.0, Unit::Deg)],
                vec![CssValue::Keyword("red".into())],
                vec![CssValue::Length(0.0, Unit::Percent)],
                vec![CssValue::Length(10.0, Unit::Percent)],
                vec![CssValue::Keyword("blue".into())],
                vec![CssValue::Length(10.0, Unit::Percent)],
                vec![CssValue::Length(20.0, Unit::Percent)],
            ],
        ),
    );
    let Background::Gradient(gradient) = container.background else {
        panic!("expected gradient background");
    };
    assert!(matches!(
        gradient.kind,
        GradientKind::Linear { angle: 90.0 }
    ));
    assert_eq!(gradient.stops.len(), 4);
}

#[test]
fn gradient_current_color_stop_resolves_to_text_color() {
    let args = vec![
        CssValue::Keyword("currentColor".into()),
        CssValue::Keyword("white".into()),
        CssValue::Keyword("black".into()),
    ];
    let mut text_style = TextStyle::default();
    text_style.color = Color(255, 0, 0, 255);
    let gradient = parse_gradient(
        "linear-gradient",
        &args,
        &text_style,
        &TextFlowStyle::default(),
        ColorScheme::Light,
    )
    .expect("gradient parses");
    assert_eq!(gradient.stops[0].color, Color(255, 0, 0, 255));
}

#[test]
fn normalize_whitespace_collapses_css_whitespace_only() {
    let normal = |s| normalize_whitespace(s, WhiteSpace::Normal);
    assert_eq!(normal("a  b\tc\nd"), "a b c d");
    assert_eq!(normal("a\u{a0}b"), "a\u{a0}b");
    assert_eq!(normal("\u{2007}\u{2007}"), "\u{2007}\u{2007}");
    assert_eq!(normal(" a\n\t b "), " a b ");
}

#[test]
fn normalize_whitespace_normalizes_segment_breaks() {
    let normal = |s| normalize_whitespace(s, WhiteSpace::Normal);
    let pre = |s| normalize_whitespace(s, WhiteSpace::Pre);
    assert_eq!(normal("a\r\nb"), "a b");
    assert_eq!(normal("a\rb"), "a b");
    assert_eq!(normal("a\u{c}b"), "a b");
    assert_eq!(pre("a\r\nb"), "a\nb");
    assert_eq!(pre("a\rb"), "a\nb");
    assert_eq!(pre("a\u{c}b"), "a\nb");
}

#[test]
fn normalize_whitespace_pre_line_drops_trailing_newline() {
    let pre_line = |s| normalize_whitespace(s, WhiteSpace::PreLine);
    assert_eq!(pre_line("a\nb\n"), "a\nb");
    assert_eq!(pre_line("a\nb\n\n"), "a\nb");
    assert_eq!(pre_line("\n"), "");
    assert_eq!(pre_line("a  b\nc\n"), "a b\nc");
}

#[test]
fn normalize_whitespace_pre_wrap_preserves_newlines() {
    assert_eq!(
        normalize_whitespace("a\nb\n", WhiteSpace::PreWrap),
        "a\nb\n"
    );
    assert_eq!(
        normalize_whitespace("a  b\tc", WhiteSpace::BreakSpaces),
        "a  b\tc"
    );
    assert_eq!(normalize_whitespace("a\n\nb", WhiteSpace::Nowrap), "a b");
}

// ═════════════════════════════════════════════════════════════════════════════
// CSS Color Function Tests — full pipeline (Parser → Resolver → Color)
// ═════════════════════════════════════════════════════════════════════════════

/// Parse a CSS rule, resolve it, and return the resolved `Color` for the
/// given property (defaults to `"color"`).
fn resolved_color(css: &str, prop: &str) -> Color {
    let stylesheet = CssParser::new(css).parse().unwrap();
    let resolved = CssResolver::resolve(&stylesheet);
    let decl = resolved
        .iter()
        .find(|d| d.name == prop)
        .unwrap_or_else(|| panic!("no `{prop}` declaration in `{css}`"));
    resolve_css_color(&decl.name, &decl.value, ColorScheme::Light)
        .unwrap_or_else(|| panic!("`{prop}` did not resolve to a color: {:?}", decl.value))
}

/// Same as `resolved_color` but uses a specific `ColorScheme`.
fn resolved_color_scheme(css: &str, prop: &str, scheme: ColorScheme) -> Color {
    let stylesheet = CssParser::new(css).parse().unwrap();
    let resolved = CssResolver::resolve(&stylesheet);
    let decl = resolved
        .iter()
        .find(|d| d.name == prop)
        .unwrap_or_else(|| panic!("no `{prop}` declaration in `{css}`"));
    resolve_css_color(&decl.name, &decl.value, scheme)
        .unwrap_or_else(|| panic!("`{prop}` did not resolve to a color: {:?}", decl.value))
}

/// Convenience: resolve `color` property from a CSS rule.
fn rc(css: &str) -> Color {
    resolved_color(css, "color")
}

#[test]
fn rgb_pure_red() {
    assert_eq!(rc("div { color: rgb(255, 0, 0); }"), Color(255, 0, 0, 255));
}

#[test]
fn rgb_pure_green() {
    assert_eq!(rc("div { color: rgb(0, 255, 0); }"), Color(0, 255, 0, 255));
}

#[test]
fn rgb_pure_blue() {
    assert_eq!(rc("div { color: rgb(0, 0, 255); }"), Color(0, 0, 255, 255));
}

#[test]
fn rgb_black() {
    assert_eq!(rc("div { color: rgb(0, 0, 0); }"), Color(0, 0, 0, 255));
}

#[test]
fn rgb_white() {
    assert_eq!(
        rc("div { color: rgb(255, 255, 255); }"),
        Color(255, 255, 255, 255)
    );
}

#[test]
fn rgb_gray() {
    assert_eq!(
        rc("div { color: rgb(128, 128, 128); }"),
        Color(128, 128, 128, 255)
    );
}

#[test]
fn rgb_mixed() {
    assert_eq!(
        rc("div { color: rgb(12, 34, 56); }"),
        Color(12, 34, 56, 255)
    );
}

#[test]
fn rgb_alpha_zero() {
    assert_eq!(rc("div { color: rgb(255, 0, 0, 0); }"), Color(255, 0, 0, 0));
}

#[test]
fn rgb_alpha_half() {
    assert_eq!(
        rc("div { color: rgb(255, 0, 0, 0.5); }"),
        Color(255, 0, 0, 128)
    );
}

#[test]
fn rgb_alpha_one() {
    assert_eq!(
        rc("div { color: rgb(255, 0, 0, 1); }"),
        Color(255, 0, 0, 255)
    );
}

#[test]
fn rgba_pure_red() {
    assert_eq!(
        rc("div { color: rgba(255, 0, 0, 1); }"),
        Color(255, 0, 0, 255)
    );
}

#[test]
fn rgba_transparent() {
    assert_eq!(
        rc("div { color: rgba(255, 0, 0, 0); }"),
        Color(255, 0, 0, 0)
    );
}

#[test]
fn rgba_half_alpha() {
    assert_eq!(
        rc("div { color: rgba(10, 20, 30, 0.5); }"),
        Color(10, 20, 30, 128)
    );
}

#[test]
fn rgb_percentage_red() {
    assert_eq!(
        rc("div { color: rgb(100%, 0%, 0%); }"),
        Color(255, 0, 0, 255)
    );
}

#[test]
fn rgb_percentage_green() {
    assert_eq!(
        rc("div { color: rgb(0%, 100%, 0%); }"),
        Color(0, 255, 0, 255)
    );
}

#[test]
fn rgb_percentage_blue() {
    assert_eq!(
        rc("div { color: rgb(0%, 0%, 100%); }"),
        Color(0, 0, 255, 255)
    );
}

#[test]
fn rgb_percentage_gray() {
    assert_eq!(
        rc("div { color: rgb(50%, 50%, 50%); }"),
        Color(128, 128, 128, 255)
    );
}

#[test]
fn rgb_space_separated() {
    assert_eq!(rc("div { color: rgb(255 0 0); }"), Color(255, 0, 0, 255));
}

#[test]
fn rgb_space_separated_alpha() {
    assert_eq!(
        rc("div { color: rgb(255 0 0 / 0.5); }"),
        Color(255, 0, 0, 128)
    );
}

#[test]
fn rgba_space_separated_alpha() {
    assert_eq!(
        rc("div { color: rgba(255 0 0 / 0.5); }"),
        Color(255, 0, 0, 128)
    );
}

#[test]
fn hex_black() {
    assert_eq!(rc("div { color: #000000; }"), Color(0, 0, 0, 255));
}

#[test]
fn hex_white() {
    assert_eq!(rc("div { color: #ffffff; }"), Color(255, 255, 255, 255));
}

#[test]
fn hex_red() {
    assert_eq!(rc("div { color: #ff0000; }"), Color(255, 0, 0, 255));
}

#[test]
fn hex_green() {
    assert_eq!(rc("div { color: #00ff00; }"), Color(0, 255, 0, 255));
}

#[test]
fn hex_blue() {
    assert_eq!(rc("div { color: #0000ff; }"), Color(0, 0, 255, 255));
}

#[test]
fn hex_shorthand_black() {
    assert_eq!(rc("div { color: #000; }"), Color(0, 0, 0, 255));
}

#[test]
fn hex_shorthand_white() {
    assert_eq!(rc("div { color: #fff; }"), Color(255, 255, 255, 255));
}

#[test]
fn hex_shorthand_red() {
    assert_eq!(rc("div { color: #f00; }"), Color(255, 0, 0, 255));
}

#[test]
fn hex_shorthand_mixed() {
    assert_eq!(rc("div { color: #123; }"), Color(17, 34, 51, 255));
}

#[test]
fn hex_with_alpha_zero() {
    assert_eq!(rc("div { color: #ff000000; }"), Color(255, 0, 0, 0));
}

#[test]
fn hex_with_alpha_half() {
    assert_eq!(rc("div { color: #ff000080; }"), Color(255, 0, 0, 128));
}

#[test]
fn hex_with_alpha_full() {
    assert_eq!(rc("div { color: #ff0000ff; }"), Color(255, 0, 0, 255));
}

#[test]
fn hex_shorthand_with_alpha_zero() {
    assert_eq!(rc("div { color: #f000; }"), Color(255, 0, 0, 0));
}

#[test]
fn hex_shorthand_with_alpha_full() {
    assert_eq!(rc("div { color: #f00f; }"), Color(255, 0, 0, 255));
}

#[test]
fn named_red() {
    assert_eq!(rc("div { color: red; }"), Color(255, 0, 0, 255));
}

#[test]
fn named_green() {
    assert_eq!(rc("div { color: green; }"), Color(0, 128, 0, 255));
}

#[test]
fn named_blue() {
    assert_eq!(rc("div { color: blue; }"), Color(0, 0, 255, 255));
}

#[test]
fn named_black() {
    assert_eq!(rc("div { color: black; }"), Color(0, 0, 0, 255));
}

#[test]
fn named_white() {
    assert_eq!(rc("div { color: white; }"), Color(255, 255, 255, 255));
}

#[test]
fn named_transparent() {
    assert_eq!(rc("div { color: transparent; }"), Color(0, 0, 0, 0));
}

#[test]
fn named_rebeccapurple() {
    assert_eq!(
        rc("div { color: rebeccapurple; }"),
        Color(102, 51, 153, 255)
    );
}

#[test]
fn hsl_red() {
    assert_eq!(
        rc("div { color: hsl(0, 100%, 50%); }"),
        Color(255, 0, 0, 255)
    );
}

#[test]
fn hsl_green() {
    assert_eq!(
        rc("div { color: hsl(120, 100%, 50%); }"),
        Color(0, 255, 0, 255)
    );
}

#[test]
fn hsl_blue() {
    assert_eq!(
        rc("div { color: hsl(240, 100%, 50%); }"),
        Color(0, 0, 255, 255)
    );
}

#[test]
fn hsl_white() {
    assert_eq!(
        rc("div { color: hsl(0, 0%, 100%); }"),
        Color(255, 255, 255, 255)
    );
}

#[test]
fn hsl_black() {
    assert_eq!(rc("div { color: hsl(0, 0%, 0%); }"), Color(0, 0, 0, 255));
}

#[test]
fn hsl_gray() {
    assert_eq!(
        rc("div { color: hsl(0, 0%, 50%); }"),
        Color(128, 128, 128, 255)
    );
}

#[test]
fn hsl_half_saturation() {
    assert_eq!(
        rc("div { color: hsl(0, 50%, 50%); }"),
        Color(191, 64, 64, 255)
    );
}

#[test]
fn hsla_red() {
    assert_eq!(
        rc("div { color: hsla(0, 100%, 50%, 1); }"),
        Color(255, 0, 0, 255)
    );
}

#[test]
fn hsla_transparent() {
    assert_eq!(
        rc("div { color: hsla(0, 100%, 50%, 0); }"),
        Color(255, 0, 0, 0)
    );
}

#[test]
fn hsla_half_alpha() {
    assert_eq!(
        rc("div { color: hsla(120, 100%, 50%, 0.5); }"),
        Color(0, 255, 0, 128)
    );
}

#[test]
fn hsl_space_separated() {
    assert_eq!(rc("div { color: hsl(0 100% 50%); }"), Color(255, 0, 0, 255));
}

#[test]
fn hsl_space_separated_alpha() {
    assert_eq!(
        rc("div { color: hsl(0 100% 50% / 0.5); }"),
        Color(255, 0, 0, 128)
    );
}

#[test]
fn hwb_red() {
    assert_eq!(rc("div { color: hwb(0 0% 0%); }"), Color(255, 0, 0, 255));
}

#[test]
fn hwb_white() {
    assert_eq!(
        rc("div { color: hwb(0 100% 0%); }"),
        Color(255, 255, 255, 255)
    );
}

#[test]
fn hwb_black() {
    assert_eq!(rc("div { color: hwb(0 0% 100%); }"), Color(0, 0, 0, 255));
}

#[test]
fn hwb_gray() {
    assert_eq!(
        rc("div { color: hwb(0 50% 50%); }"),
        Color(128, 128, 128, 255)
    );
}

#[test]
fn hwb_with_alpha() {
    assert_eq!(
        rc("div { color: hwb(0 0% 0% / 0.5); }"),
        Color(255, 0, 0, 128)
    );
}

#[test]
fn hwb_green() {
    assert_eq!(rc("div { color: hwb(120 0% 0%); }"), Color(0, 255, 0, 255));
}

#[test]
fn hwb_blue() {
    assert_eq!(rc("div { color: hwb(240 0% 0%); }"), Color(0, 0, 255, 255));
}

#[test]
fn hwb_yellow() {
    assert_eq!(rc("div { color: hwb(60 0% 0%); }"), Color(255, 255, 0, 255));
}

#[test]
fn hwb_white_plus_black_is_gray() {
    // w + b >= 1 → gray = w / (w + b)
    assert_eq!(rc("div { color: hwb(0 25% 75%); }"), Color(64, 64, 64, 255));
}

#[test]
fn hwb_white_plus_black_half_half() {
    assert_eq!(
        rc("div { color: hwb(0 50% 50%); }"),
        Color(128, 128, 128, 255)
    );
}

#[test]
fn hwb_white_plus_black_all_gray() {
    // w + b == 1 → gray
    assert_eq!(
        rc("div { color: hwb(0 100% 0%); }"),
        Color(255, 255, 255, 255)
    );
    assert_eq!(rc("div { color: hwb(0 0% 100%); }"), Color(0, 0, 0, 255));
    assert_eq!(
        rc("div { color: hwb(0 75% 25%); }"),
        Color(191, 191, 191, 255)
    );
}

#[test]
fn hwb_tint() {
    // Adding whiteness to a hue tints it
    let c = rc("div { color: hwb(0 30% 0%); }");
    // Pure red tinted 30% white: R stays high, G and B increase
    assert!(c.0 > 200, "red dominant: {c:?}");
    assert!(c.1 > 50, "green raised by white: {c:?}");
    assert!(c.2 > 50, "blue raised by white: {c:?}");
}

#[test]
fn hwb_shade() {
    // Adding blackness to a hue shades it
    let c = rc("div { color: hwb(0 0% 30%); }");
    // Pure red shaded 30% black: all channels decrease
    assert!(c.0 > 100, "red still visible: {c:?}");
    assert!(c.1 < 50, "green stays low: {c:?}");
    assert!(c.2 < 50, "blue stays low: {c:?}");
}

#[test]
fn hwb_zero_alpha() {
    assert_eq!(
        rc("div { color: hwb(120 0% 0% / 0); }"),
        Color(0, 255, 0, 0)
    );
}

#[test]
fn hwb_full_alpha() {
    assert_eq!(
        rc("div { color: hwb(240 0% 0% / 1); }"),
        Color(0, 0, 255, 255)
    );
}

#[test]
fn hwb_hue_wraps() {
    // 360 == 0 (red)
    assert_eq!(rc("div { color: hwb(360 0% 0%); }"), Color(255, 0, 0, 255));
    // 420 == 60 (yellow)
    assert_eq!(
        rc("div { color: hwb(420 0% 0%); }"),
        Color(255, 255, 0, 255)
    );
    // -60 == 300 (magenta)
    assert_eq!(
        rc("div { color: hwb(-60 0% 0%); }"),
        Color(255, 0, 255, 255)
    );
}

#[test]
fn hwb_without_percent_syntax() {
    // hwb can also accept 0.0..1.0 without percent signs
    assert_eq!(rc("div { color: hwb(0 0.3 0); }"), Color(255, 77, 77, 255));
}

// ─── light-dark() ──────────────────────────────────────────────────────────

#[test]
fn light_dark_picks_light_in_light_scheme() {
    let c = resolved_color_scheme(
        "div { color: light-dark(white, black); }",
        "color",
        ColorScheme::Light,
    );
    assert_eq!(c, Color(255, 255, 255, 255));
}

#[test]
fn light_dark_picks_dark_in_dark_scheme() {
    let c = resolved_color_scheme(
        "div { color: light-dark(white, black); }",
        "color",
        ColorScheme::Dark,
    );
    assert_eq!(c, Color(0, 0, 0, 255));
}

#[test]
fn light_dark_with_hex_colors() {
    let c_light = resolved_color_scheme(
        "div { color: light-dark(#ffffff, #333333); }",
        "color",
        ColorScheme::Light,
    );
    assert_eq!(c_light, Color(255, 255, 255, 255));

    let c_dark = resolved_color_scheme(
        "div { color: light-dark(#ffffff, #333333); }",
        "color",
        ColorScheme::Dark,
    );
    assert_eq!(c_dark, Color(0x33, 0x33, 0x33, 255));
}

#[test]
fn light_dark_with_named_colors() {
    let c = resolved_color_scheme(
        "div { color: light-dark(red, blue); }",
        "color",
        ColorScheme::Dark,
    );
    assert_eq!(c, Color(0, 0, 255, 255));
}

// ─── color-mix() ───────────────────────────────────────────────────────────

#[test]
fn color_mix_equal_weights_srgb() {
    let c = rc("div { color: color-mix(in srgb, red, blue); }");
    assert_eq!(c, Color(188, 0, 188, 255));
}

#[test]
fn color_mix_white_black() {
    let c = rc("div { color: color-mix(in srgb, white, black); }");
    assert!(c.0 > 100 && c.0 < 200, "gray expected: {c:?}");
    assert_eq!(c.0, c.1, "R == G for neutral gray");
    assert_eq!(c.1, c.2, "G == B for neutral gray");
    assert_eq!(c.3, 255);
}

#[test]
fn color_mix_explicit_percentages() {
    let c = rc("div { color: color-mix(in srgb, red 25%, blue); }");
    // red 25% + blue (missing weight takes remaining 75%)
    assert_eq!(c, Color(137, 0, 225, 255));
}

#[test]
fn color_mix_transparent_alpha() {
    let c = rc("div { color: color-mix(in srgb, transparent, blue); }");
    assert_eq!(c.3, 128, "alpha should be ~50%");
}

#[test]
fn color_mix_lch_space() {
    let c = rc("div { color: color-mix(in lch, red, blue); }");
    assert!(c.0 > 0, "has red component: {c:?}");
    assert!(c.2 > 0, "has blue component: {c:?}");
    assert!(c.1 < 50, "low green (purple): {c:?}");
    assert_eq!(c.3, 255);
}

#[test]
fn color_mix_both_percentages() {
    let c = rc("div { color: color-mix(in srgb, red 70%, blue 30%); }");
    assert!(c.0 > c.2, "more red than blue: {c:?}");
    assert_eq!(c.3, 255);
}

// ─── Boundary and edge-case tests ───────────────────────────────────────────

#[test]
fn rgb_clamps_out_of_range_values() {
    assert_eq!(
        rc("div { color: rgb(300, 300, 300); }"),
        Color(255, 255, 255, 255)
    );
}

#[test]
fn rgb_negative_values_clamp_to_zero() {
    assert_eq!(rc("div { color: rgb(-10, 0, 0); }"), Color(0, 0, 0, 255));
}

#[test]
fn rgb_zero_alpha() {
    assert_eq!(
        rc("div { color: rgb(128, 128, 128, 0); }"),
        Color(128, 128, 128, 0)
    );
}

#[test]
fn rgb_full_alpha() {
    assert_eq!(
        rc("div { color: rgb(64, 128, 192, 1); }"),
        Color(64, 128, 192, 255)
    );
}

// ─── Math functions inside color functions ─────────────────────────────────

#[test]
fn rgb_calc_addition() {
    assert_eq!(
        rc("div { color: rgb(calc(10 + 3), 5, 6); }"),
        Color(13, 5, 6, 255)
    );
}

#[test]
fn rgb_calc_subtraction() {
    assert_eq!(
        rc("div { color: rgb(calc(255 - 55), 100, 50); }"),
        Color(200, 100, 50, 255)
    );
}

#[test]
fn rgb_calc_multiplication() {
    assert_eq!(
        rc("div { color: rgb(calc(50 * 2), 25, 10); }"),
        Color(100, 25, 10, 255)
    );
}

#[test]
fn rgb_calc_division() {
    assert_eq!(
        rc("div { color: rgb(calc(200 / 2), 100, 50); }"),
        Color(100, 100, 50, 255)
    );
}

#[test]
fn rgb_calc_nested() {
    assert_eq!(
        rc("div { color: rgb(calc(10 + calc(5 * 2)), 0, 0); }"),
        Color(20, 0, 0, 255)
    );
}

#[test]
fn rgb_calc_all_channels() {
    assert_eq!(
        rc("div { color: rgb(calc(100 + 55), calc(200 - 100), calc(25 * 4)); }"),
        Color(155, 100, 100, 255)
    );
}

#[test]
fn rgb_calc_with_alpha() {
    assert_eq!(
        rc("div { color: rgb(calc(200 + 55), 0, 0, calc(0.5 + 0.5)); }"),
        Color(255, 0, 0, 255)
    );
}

#[test]
fn rgb_calc_alpha_only() {
    assert_eq!(
        rc("div { color: rgb(255, 0, 0, calc(0.25 * 2)); }"),
        Color(255, 0, 0, 128)
    );
}

#[test]
fn rgb_min_function() {
    assert_eq!(
        rc("div { color: rgb(min(100, 200), 50, 30); }"),
        Color(100, 50, 30, 255)
    );
}

#[test]
fn rgb_max_function() {
    assert_eq!(
        rc("div { color: rgb(max(100, 200), 50, 30); }"),
        Color(200, 50, 30, 255)
    );
}

#[test]
fn rgb_clamp_function() {
    assert_eq!(
        rc("div { color: rgb(clamp(0, 300, 255), 50, 30); }"),
        Color(255, 50, 30, 255)
    );
}

#[test]
fn hsl_calc_hue() {
    assert_eq!(
        rc("div { color: hsl(calc(120 + 0)deg, 100%, 50%); }"),
        Color(0, 255, 0, 255)
    );
}

#[test]
fn hsl_calc_saturation() {
    assert_eq!(
        rc("div { color: hsl(0deg, calc(50 + 50)%, 50%); }"),
        Color(255, 0, 0, 255)
    );
}

#[test]
fn hsl_calc_lightness() {
    assert_eq!(
        rc("div { color: hsl(0deg, 100%, calc(100 / 2)%); }"),
        Color(255, 0, 0, 255)
    );
}

#[test]
fn hwb_calc_whiteness() {
    assert_eq!(
        rc("div { color: hwb(0 calc(0 + 0)% 0%); }"),
        Color(255, 0, 0, 255)
    );
}

#[test]
fn hwb_calc_blackness() {
    assert_eq!(
        rc("div { color: hwb(0 0% calc(50 + 50)%); }"),
        Color(0, 0, 0, 255)
    );
}

#[test]
fn rgb_calc_with_percent_values() {
    assert_eq!(
        rc("div { color: rgb(calc(50 + 50)%, 0%, 0%); }"),
        Color(255, 0, 0, 255)
    );
}

#[test]
fn hsl_min_max_in_channels() {
    assert_eq!(
        rc("div { color: hsl(min(0, 360)deg, max(50, 100)%, 50%); }"),
        Color(255, 0, 0, 255)
    );
}

#[test]
fn rgb_calc_complex_expression() {
    // calc(255 * 0.5 + 127) = 254.5 → 255
    assert_eq!(
        rc("div { color: rgb(calc(255 * 0.5 + 127), 0, 0); }"),
        Color(255, 0, 0, 255)
    );
}

// ─── hsl boundary hues (continued) ────────────────────────────────────────

#[test]
fn hsl_boundary_hues() {
    let hues = [
        (0, Color(255, 0, 0, 255)),
        (60, Color(255, 255, 0, 255)),
        (120, Color(0, 255, 0, 255)),
        (180, Color(0, 255, 255, 255)),
        (240, Color(0, 0, 255, 255)),
        (300, Color(255, 0, 255, 255)),
    ];
    for (hue, expected) in hues {
        let c = rc(&format!("div {{ color: hsl({hue}deg, 100%, 50%); }}"));
        assert_eq!(c, expected, "hue={hue}");
    }
}

#[test]
fn hsl_intermediate_hues() {
    let c30 = rc("div { color: hsl(30deg, 100%, 50%); }");
    assert!(c30.0 > 200, "red should be high at 30 deg: {c30:?}");
    assert!(c30.1 > 50, "green should be moderate: {c30:?}");
    assert_eq!(c30.2, 0, "blue should be zero at 30 deg");

    let c90 = rc("div { color: hsl(90deg, 100%, 50%); }");
    assert!(c90.1 > 200, "green should be high at 90 deg: {c90:?}");
    assert!(c90.0 > 50, "red should be moderate: {c90:?}");
    assert_eq!(c90.2, 0, "blue should be zero at 90 deg");
}

#[test]
fn hsl_pastel_colors() {
    let pastel = rc("div { color: hsl(0deg, 100%, 87.5%); }");
    assert!(pastel.0 > 200, "red channel high: {pastel:?}");
    assert!(pastel.1 > 100, "green channel moderate: {pastel:?}");
    assert!(pastel.2 > 100, "blue channel moderate: {pastel:?}");
}

#[test]
fn hsl_dark_colors() {
    let dark = rc("div { color: hsl(240deg, 100%, 25%); }");
    assert!(dark.2 > 0, "blue channel present: {dark:?}");
    assert!(dark.0 < 100, "red low in dark blue: {dark:?}");
    assert!(dark.1 < 100, "green low in dark blue: {dark:?}");
}

// ─── Mixed selector contexts ───────────────────────────────────────────────

#[test]
fn color_in_multiple_selectors() {
    let css = r#"
        .a { color: red; }
        .b { color: rgb(0, 0, 255); }
        .c { color: hsl(120deg, 100%, 50%); }
    "#;
    assert_eq!(resolved_color(css, "color"), Color(255, 0, 0, 255));
    // The last rule wins for the same element — but different selectors.
    // Let's test each selector's color independently.
    let stylesheet = CssParser::new(css).parse().unwrap();
    let resolved = CssResolver::resolve(&stylesheet);
    let colors: Vec<_> = resolved
        .iter()
        .filter(|d| d.name == "color")
        .map(|d| resolve_css_color(&d.name, &d.value, ColorScheme::Light).unwrap())
        .collect();
    assert_eq!(colors.len(), 3);
    assert_eq!(colors[0], Color(255, 0, 0, 255)); // .a: red
    assert_eq!(colors[1], Color(0, 0, 255, 255)); // .b: rgb blue
    assert_eq!(colors[2], Color(0, 255, 0, 255)); // .c: hsl green
}

#[test]
fn color_with_important() {
    let css = ".a { color: red !important; }";
    let stylesheet = CssParser::new(css).parse().unwrap();
    let resolved = CssResolver::resolve(&stylesheet);
    let decl = resolved.iter().find(|d| d.name == "color").unwrap();
    assert!(decl.important);
    assert_eq!(
        resolve_css_color(&decl.name, &decl.value, ColorScheme::Light).unwrap(),
        Color(255, 0, 0, 255)
    );
}

#[test]
fn background_color_property() {
    assert_eq!(
        resolved_color("div { background-color: #ff8000; }", "background-color"),
        Color(255, 128, 0, 255)
    );
}

#[test]
fn border_color_property() {
    assert_eq!(
        resolved_color("div { border-color: teal; }", "border-color"),
        Color(0, 128, 128, 255)
    );
}

// ─── var() with color values ───────────────────────────────────────────────
// NOTE: var() resolution happens in the layout builder, not in CssResolver.
// These tests verify that var() values survive the resolver as-is.

#[test]
fn var_preserved_in_resolver() {
    let stylesheet = CssParser::new(":root { --main: red; } div { color: var(--main); }")
        .parse()
        .unwrap();
    let resolved = CssResolver::resolve(&stylesheet);
    let decl = resolved.iter().find(|d| d.name == "color").unwrap();
    // var() is preserved as a function, not resolved at this stage
    assert!(matches!(decl.value, CssValue::Function(ref name, _) if name == "var"));
}

// ─── Inheritance through the resolver ───────────────────────────────────────

#[test]
fn color_not_inherited_by_default_in_resolver() {
    // The resolver produces declarations, not computed styles.
    // Each declaration is independent — no inheritance at this stage.
    let css = "div { color: red; } span { }";
    let stylesheet = CssParser::new(css).parse().unwrap();
    let resolved = CssResolver::resolve(&stylesheet);
    let span_decls: Vec<_> = resolved
        .iter()
        .filter(|d| d.selector.to_string().contains("span"))
        .collect();
    assert!(span_decls.is_empty(), "span should have no declarations");
}

// ─── Display rendering of color functions (parser round-trip) ───────────────

#[test]
fn parser_roundtrip_rgb() {
    let css = "div { color: rgb(255, 128, 0); }";
    let stylesheet = CssParser::new(css).parse().unwrap();
    let resolved = CssResolver::resolve(&stylesheet);
    let decl = resolved.iter().find(|d| d.name == "color").unwrap();
    assert_eq!(decl.value.to_string(), "rgb(255, 128, 0)");
}

#[test]
fn parser_roundtrip_hsl() {
    let css = "div { color: hsl(120deg, 100%, 50%); }";
    let stylesheet = CssParser::new(css).parse().unwrap();
    let resolved = CssResolver::resolve(&stylesheet);
    let decl = resolved.iter().find(|d| d.name == "color").unwrap();
    assert_eq!(decl.value.to_string(), "hsl(120deg, 100%, 50%)");
}

#[test]
fn parser_roundtrip_hex() {
    let css = "div { color: #ff8000; }";
    let stylesheet = CssParser::new(css).parse().unwrap();
    let resolved = CssResolver::resolve(&stylesheet);
    let decl = resolved.iter().find(|d| d.name == "color").unwrap();
    assert_eq!(decl.value.to_string(), "#ff8000");
}

#[test]
fn parser_roundtrip_named_color() {
    let css = "div { color: rebeccapurple; }";
    let stylesheet = CssParser::new(css).parse().unwrap();
    let resolved = CssResolver::resolve(&stylesheet);
    let decl = resolved.iter().find(|d| d.name == "color").unwrap();
    // Named colors are stored as CssValue::Keyword, display is the name itself
    assert_eq!(decl.value.to_string(), "rebeccapurple");
}
