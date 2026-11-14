use super::*;

use crate::engine::layouter::types::{
    Background, BackgroundDimension, BackgroundImage, BackgroundLayer, BackgroundPosition,
    BackgroundPositionAxis, BackgroundRepeat, BackgroundSize, Color, Gradient, GradientKind,
};

fn gradient_layer(color1: &str, color2: &str, angle_deg: f32) -> CssValue {
    CssValue::Function(
        "linear-gradient".into(),
        vec![vec![
            CssValue::Length(angle_deg, Unit::Deg),
            CssValue::Color(color1.into()),
            CssValue::Color(color2.into()),
        ]],
    )
}

#[test]
fn parse_multiple_background_layers_are_preserved() {
    let css = declaration_value(
        "background: linear-gradient(45deg, #ff0000, #0000ff), linear-gradient(135deg, #00ff00, #000000);",
    );
    let cs = apply_container_property("background", css);
    assert_eq!(
        cs.background.layers.len(),
        2,
        "expected 2 background layers"
    );
    match &cs.background.layers[0].image {
        Some(BackgroundImage::Gradient(g)) => match &g.kind {
            GradientKind::Linear { angle } => {
                assert!((angle - 45.0).abs() < 1e-4);
            }
            _ => panic!("expected linear gradient"),
        },
        _ => panic!("expected gradient"),
    }
    match &cs.background.layers[1].image {
        Some(BackgroundImage::Gradient(g)) => match &g.kind {
            GradientKind::Linear { angle } => {
                assert!((angle - 135.0).abs() < 1e-4);
            }
            _ => panic!("expected linear gradient"),
        },
        _ => panic!("expected gradient"),
    }
}

#[test]
fn parse_transparent_gradient_stops() {
    let css = declaration_value(
        "background: linear-gradient(45deg, #1b2330 25%, transparent 25%, transparent 75%, #1b2330 75%);",
    );
    let cs = apply_container_property("background", css);
    assert_eq!(cs.background.layers.len(), 1);
    match &cs.background.layers[0].image {
        Some(BackgroundImage::Gradient(g)) => {
            assert_eq!(g.stops.len(), 4);
            // First stop: #1b2330 at 25%
            assert_eq!(
                g.stops[0].color,
                resolve_color(CssValue::Color("1b2330".into()))
            );
            assert_eq!(g.stops[0].position, Some(0.25));
            // Second stop: transparent at 25%
            assert_eq!(g.stops[1].color.3, 0);
            assert_eq!(g.stops[1].position, Some(0.25));
        }
        _ => panic!("expected gradient"),
    }
}

#[test]
fn background_position_applies_per_layer() {
    // First set up layers via background-image, then apply position
    let image_css = declaration_value(
        "background-image: linear-gradient(45deg, red, blue), linear-gradient(135deg, green, black);",
    );
    let pos_css = declaration_value("background-position: 0 0, 8px 8px;");
    let cs = apply_container_declarations(&[
        ("background-image", image_css),
        ("background-position", pos_css),
    ])
    .expect("declarations apply");
    assert_eq!(cs.background.layers.len(), 2);
    match cs.background.layers[0].position.x {
        BackgroundPositionAxis::Start(BackgroundOffset::Length(v)) => v,
        BackgroundPositionAxis::Start(BackgroundOffset::Zero) => 0.0,
        _ => panic!("expected x=0px"),
    };
    match cs.background.layers[1].position.x {
        BackgroundPositionAxis::Start(BackgroundOffset::Length(v)) => assert_eq!(v, 8.0),
        _ => panic!("expected x=8px"),
    }
    match cs.background.layers[1].position.y {
        BackgroundPositionAxis::Start(BackgroundOffset::Length(v)) => assert_eq!(v, 8.0),
        _ => panic!("expected y=8px"),
    }
}

#[test]
fn background_size_single_value_replicated_to_all_layers() {
    let image_css = declaration_value(
        "background-image: linear-gradient(45deg, red, blue), linear-gradient(135deg, green, black);",
    );
    let size_css = declaration_value("background-size: 16px 16px;");
    let cs = apply_container_declarations(&[
        ("background-image", image_css),
        ("background-size", size_css),
    ])
    .expect("declarations apply");
    assert_eq!(cs.background.layers.len(), 2);
    for layer in &cs.background.layers {
        match layer.size {
            BackgroundSize::Explicit {
                width: BackgroundDimension::Length(w),
                height: BackgroundDimension::Length(h),
            } => {
                assert_eq!(w, 16.0);
                assert_eq!(h, 16.0);
            }
            _ => panic!("expected explicit 16px 16px"),
        }
    }
}

#[test]
fn background_size_multiple_values_per_layer() {
    // `16px, cover` → 2 items, not both dimensions → 2 separate layer sizes
    let image_css = declaration_value(
        "background-image: linear-gradient(45deg, red, blue), linear-gradient(135deg, green, black);",
    );
    let size_css = declaration_value("background-size: 16px, cover;");
    let cs = apply_container_declarations(&[
        ("background-image", image_css),
        ("background-size", size_css),
    ])
    .expect("declarations apply");
    assert_eq!(cs.background.layers.len(), 2);
    match cs.background.layers[0].size {
        BackgroundSize::Explicit {
            width: BackgroundDimension::Length(w),
            height: BackgroundDimension::Auto,
        } => {
            assert_eq!(w, 16.0);
        }
        _ => panic!("expected explicit 16px auto"),
    }
    assert_eq!(cs.background.layers[1].size, BackgroundSize::Cover);
}

#[test]
fn background_repeat_per_layer() {
    let image_css = declaration_value(
        "background-image: linear-gradient(45deg, red, blue), linear-gradient(135deg, green, black);",
    );
    let repeat_css = declaration_value("background-repeat: repeat, no-repeat;");
    let cs = apply_container_declarations(&[
        ("background-image", image_css),
        ("background-repeat", repeat_css),
    ])
    .expect("declarations apply");
    assert_eq!(cs.background.layers.len(), 2);
    assert_eq!(cs.background.layers[0].repeat, BackgroundRepeat::Repeat);
    assert_eq!(cs.background.layers[1].repeat, BackgroundRepeat::NoRepeat);
}

#[test]
fn background_color_only_replaces_solid_layers() {
    let css = declaration_value("background-color: #ff0000;");
    let cs = apply_container_property("background-color", css);
    assert_eq!(
        cs.background.color,
        Some(Color(255, 0, 0, 255)),
        "expected solid red color"
    );
    assert!(cs.background.layers.is_empty());
}

#[test]
fn background_color_preserves_gradient_layers() {
    let image = declaration_value(
        "background-image: linear-gradient(45deg, red, blue), linear-gradient(135deg, green, black);",
    );
    let color = declaration_value("background-color: #ff0000;");
    let cs =
        apply_container_declarations(&[("background-image", image), ("background-color", color)])
            .expect("declarations apply");
    assert_eq!(cs.background.layers.len(), 2);
    assert_eq!(
        cs.background.color,
        Some(Color(255, 0, 0, 255)),
        "color should be independent bottom color"
    );
}

#[test]
fn background_image_preserves_existing_color_layer() {
    let color = declaration_value("background-color: #00ff00;");
    let image = declaration_value("background-image: url(\"pixel.png\");");
    let cs =
        apply_container_declarations(&[("background-color", color), ("background-image", image)])
            .expect("declarations apply");
    assert_eq!(cs.background.layers.len(), 1);
    assert_eq!(
        cs.background.color,
        Some(Color(0, 255, 0, 255)),
        "existing color should be preserved"
    );
}

#[test]
fn full_checkerboard_css_parses_as_two_layers() {
    let css = declaration_value(
        "background: linear-gradient(45deg, #1b2330 25%, transparent 25%, transparent 75%, #1b2330 75%), linear-gradient(45deg, #1b2330 25%, transparent 25%, transparent 75%, #1b2330 75%);",
    );
    let cs = apply_container_property("background", css);
    assert_eq!(
        cs.background.layers.len(),
        2,
        "checkerboard should have 2 layers"
    );
    for layer in &cs.background.layers {
        match &layer.image {
            Some(BackgroundImage::Gradient(g)) => {
                assert_eq!(g.stops.len(), 4);
            }
            _ => panic!("expected gradient"),
        }
    }

    let pos_css = declaration_value("background-position: 0 0, 8px 8px;");
    let pos_cs = apply_container_property("background-position", pos_css);
    assert_eq!(pos_cs.background.layers.len(), 0);
    // Note: background-position alone doesn't create layers; it needs existing layers to apply to.
    // This test verifies no panic and no spurious layers created.
}

#[test]
fn background_image_layers_with_transparent_stops() {
    let css = declaration_value(
        "background-image: linear-gradient(45deg, red 25%, transparent 25%), linear-gradient(135deg, blue 50%, transparent 50%);",
    );
    let cs = apply_container_property("background-image", css);
    assert_eq!(cs.background.layers.len(), 2);
    match &cs.background.layers[0].image {
        Some(BackgroundImage::Gradient(g)) => {
            assert_eq!(g.stops.len(), 2);
        }
        _ => panic!("expected gradient"),
    }
    match &cs.background.layers[1].image {
        Some(BackgroundImage::Gradient(g)) => {
            assert_eq!(g.stops.len(), 2);
        }
        _ => panic!("expected gradient"),
    }
}

#[test]
fn default_container_style_has_empty_background() {
    let cs = ContainerStyle::default();
    assert_eq!(cs.background.color, None);
    assert!(cs.background.layers.is_empty());
}

#[test]
fn background_shorthand_color_and_image_one_layer() {
    // `background: #fff url("pixel.png")` → color is bottom color, image is one layer
    let css = declaration_value(r#"background: #fff url("pixel.png");"#);
    let cs = apply_container_property("background", css);
    assert_eq!(
        cs.background.color,
        Some(Color(255, 255, 255, 255)),
        "color should be bottom color"
    );
    assert_eq!(cs.background.layers.len(), 1);
    match &cs.background.layers[0].image {
        Some(crate::engine::layouter::types::BackgroundImage::Image { source, .. }) => {
            assert_eq!(source, "pixel.png");
        }
        _ => panic!("expected image layer"),
    }
}

#[test]
fn background_shorthand_color_after_image() {
    // `background: url("pixel.png") #fff` → color still bottom color
    let css = declaration_value(r#"background: url("pixel.png") #fff;"#);
    let cs = apply_container_property("background", css);
    assert_eq!(
        cs.background.color,
        Some(Color(255, 255, 255, 255)),
        "color should be bottom color regardless of order"
    );
    assert_eq!(cs.background.layers.len(), 1);
}

#[test]
fn background_shorthand_per_layer_repeat() {
    // `url("a.png") no-repeat, url("b.png") repeat` → two layers with different repeat
    let css = declaration_value(r#"background: url("a.png") no-repeat, url("b.png") repeat;"#);
    let cs = apply_container_property("background", css);
    assert_eq!(cs.background.layers.len(), 2);
    assert_eq!(cs.background.layers[0].repeat, BackgroundRepeat::NoRepeat);
    assert_eq!(cs.background.layers[1].repeat, BackgroundRepeat::Repeat);
}

#[test]
fn css_parser_preserves_comma_in_shorthand_value() {
    use crate::engine::css::parser::{CssNodeType, Parser as CssParser};
    use crate::engine::css::values::CssValue;

    // `url("a.png") no-repeat, url("b.png") repeat` → Comma groups are preserved
    // as separate argument sublists within the url() Function.
    let sheet =
        CssParser::new(".t { background: url(\"a.png\") no-repeat, url(\"b.png\") repeat; }")
            .parse()
            .unwrap();
    let node = sheet.children()[0].children()[0].node();
    let CssNodeType::Declaration { value, .. } = node else {
        panic!("expected a declaration");
    };
    // The value starts with url(), so it's a Function with comma-separated arg groups.
    let CssValue::Function(_, args) = value else {
        panic!("expected a Function, got {value:?}");
    };
    // Two comma-separated groups: [String("a.png"), Keyword("no-repeat")] and
    // [Function("url", ...), Keyword("repeat")]
    assert_eq!(args.len(), 2);
    assert!(
        args[0]
            .iter()
            .any(|v| matches!(v, CssValue::Keyword(k) if k == "no-repeat"))
    );
    assert!(
        args[1]
            .iter()
            .any(|v| matches!(v, CssValue::Keyword(k) if k == "repeat"))
    );
}

#[test]
fn css_parser_preserves_comma_separated_groups() {
    use crate::engine::css::parser::{CssNodeType, Parser as CssParser};
    use crate::engine::css::values::CssValue;

    // `10px, 20px, 30px` → List with two Separators
    let sheet = CssParser::new(".t { background-size: 10px, 20px, 30px; }")
        .parse()
        .unwrap();
    let node = sheet.children()[0].children()[0].node();
    let CssNodeType::Declaration { value, .. } = node else {
        panic!("expected a declaration");
    };
    let CssValue::List(items) = value else {
        panic!("expected a List, got {value:?}");
    };
    // Expect: [Length(10px), Separator, Length(20px), Separator, Length(30px)]
    assert_eq!(items.len(), 5);
    assert!(matches!(items[1], CssValue::Separator));
    assert!(matches!(items[3], CssValue::Separator));
}

#[test]
fn background_multi_layer_differing_sizes() {
    // `background-image: url("a.png"), url("b.png"); background-size: 10px 10px, 20px 20px;`
    let image_css = declaration_value(r#"background-image: url("a.png"), url("b.png");"#);
    let size_css = declaration_value("background-size: 10px 10px, 20px 20px;");
    let cs = apply_container_declarations(&[
        ("background-image", image_css),
        ("background-size", size_css),
    ])
    .expect("declarations apply");
    assert_eq!(cs.background.layers.len(), 2);
    match cs.background.layers[0].size {
        crate::engine::layouter::types::BackgroundSize::Explicit {
            width: crate::engine::layouter::types::BackgroundDimension::Length(w),
            height: crate::engine::layouter::types::BackgroundDimension::Length(h),
        } => {
            assert_eq!(w, 10.0);
            assert_eq!(h, 10.0);
        }
        _ => panic!("expected 10px 10px for first layer"),
    }
    match cs.background.layers[1].size {
        crate::engine::layouter::types::BackgroundSize::Explicit {
            width: crate::engine::layouter::types::BackgroundDimension::Length(w),
            height: crate::engine::layouter::types::BackgroundDimension::Length(h),
        } => {
            assert_eq!(w, 20.0);
            assert_eq!(h, 20.0);
        }
        _ => panic!("expected 20px 20px for second layer"),
    }
}

#[test]
fn background_multi_layer_differing_positions() {
    // `background-image: url("a.png"), url("b.png"); background-position: 0 0, 10px 10px;`
    let image_css = declaration_value(r#"background-image: url("a.png"), url("b.png");"#);
    let pos_css = declaration_value("background-position: 0 0, 10px 10px;");
    let cs = apply_container_declarations(&[
        ("background-image", image_css),
        ("background-position", pos_css),
    ])
    .expect("declarations apply");
    assert_eq!(cs.background.layers.len(), 2);
    match cs.background.layers[0].position.x {
        crate::engine::layouter::types::BackgroundPositionAxis::Start(
            crate::engine::layouter::types::BackgroundOffset::Length(v),
        ) => assert_eq!(v, 0.0),
        _ => panic!("expected x=0px for first layer"),
    }
    match cs.background.layers[1].position.x {
        crate::engine::layouter::types::BackgroundPositionAxis::Start(
            crate::engine::layouter::types::BackgroundOffset::Length(v),
        ) => assert_eq!(v, 10.0),
        _ => panic!("expected x=10px for second layer"),
    }
}

#[test]
fn background_multi_layer_differing_repeats() {
    // `background-image: url("a.png"), url("b.png"); background-repeat: no-repeat, repeat;`
    let image_css = declaration_value(r#"background-image: url("a.png"), url("b.png");"#);
    let repeat_css = declaration_value("background-repeat: no-repeat, repeat;");
    let cs = apply_container_declarations(&[
        ("background-image", image_css),
        ("background-repeat", repeat_css),
    ])
    .expect("declarations apply");
    assert_eq!(cs.background.layers.len(), 2);
    assert_eq!(cs.background.layers[0].repeat, BackgroundRepeat::NoRepeat);
    assert_eq!(cs.background.layers[1].repeat, BackgroundRepeat::Repeat);
}

#[test]
fn background_shorthand_multi_layer_full() {
    // `background: url("a.png") no-repeat / 10px 10px, url("b.png") repeat / cover`
    let css = declaration_value(
        r#"background: url("a.png") no-repeat / 10px 10px, url("b.png") repeat / cover;"#,
    );
    let cs = apply_container_property("background", css);
    assert_eq!(cs.background.layers.len(), 2);
    assert_eq!(cs.background.layers[0].repeat, BackgroundRepeat::NoRepeat);
    assert_eq!(cs.background.layers[1].repeat, BackgroundRepeat::Repeat);
    match cs.background.layers[0].size {
        crate::engine::layouter::types::BackgroundSize::Explicit {
            width: crate::engine::layouter::types::BackgroundDimension::Length(w),
            height: crate::engine::layouter::types::BackgroundDimension::Length(h),
        } => {
            assert_eq!(w, 10.0);
            assert_eq!(h, 10.0);
        }
        _ => panic!("expected 10px 10px for first layer size"),
    }
    match cs.background.layers[1].size {
        crate::engine::layouter::types::BackgroundSize::Cover => {}
        _ => panic!("expected cover for second layer size"),
    }
}
