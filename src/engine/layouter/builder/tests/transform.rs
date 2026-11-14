use super::*;

use crate::engine::renderer_model::AffineTransform;

#[test]
fn transform_none_yields_no_transform() {
    let cs = apply_container_property("transform", CssValue::Keyword("none".into()));
    assert_eq!(cs.transform, None);
}

#[test]
fn transform_rotate_degrees() {
    let cs = apply_container_property("transform", declaration_value("transform: rotate(45deg)"));
    let Some(t) = cs.transform else {
        panic!("expected a transform, got None");
    };
    let expected = AffineTransform::rotate(std::f32::consts::PI / 4.0);
    assert!((t.m11 - expected.m11).abs() < 1e-4);
    assert!((t.m12 - expected.m12).abs() < 1e-4);
    assert!((t.m21 - expected.m21).abs() < 1e-4);
    assert!((t.m22 - expected.m22).abs() < 1e-4);
    assert_eq!((t.dx, t.dy), (0.0, 0.0));
}

#[test]
fn transform_rotate_radians_and_turns() {
    let rad = apply_container_property(
        "transform",
        declaration_value("transform: rotate(3.14159rad)"),
    );
    let Some(t) = rad.transform else {
        panic!("expected a transform, got None");
    };
    assert!((t.m11 - (-1.0)).abs() < 1e-3);

    let turn =
        apply_container_property("transform", declaration_value("transform: rotate(0.5turn)"));
    let Some(t) = turn.transform else {
        panic!("expected a transform, got None");
    };
    assert!((t.m11 - (-1.0)).abs() < 1e-4);
}

#[test]
fn transform_scale_uniform_and_xy() {
    let cs = apply_container_property("transform", declaration_value("transform: scale(2)"));
    let Some(t) = cs.transform else {
        panic!("expected a transform, got None");
    };
    assert_eq!((t.m11, t.m22), (2.0, 2.0));
    assert_eq!((t.m12, t.m21, t.dx, t.dy), (0.0, 0.0, 0.0, 0.0));

    let cs = apply_container_property("transform", declaration_value("transform: scale(1.5, 2)"));
    let Some(t) = cs.transform else {
        panic!("expected a transform, got None");
    };
    assert_eq!((t.m11, t.m22), (1.5, 2.0));
}

#[test]
fn transform_translate() {
    let cs = apply_container_property(
        "transform",
        declaration_value("transform: translate(10px, 20px)"),
    );
    let Some(t) = cs.transform else {
        panic!("expected a transform, got None");
    };
    assert_eq!((t.dx, t.dy), (10.0, 20.0));
    assert_eq!((t.m11, t.m22), (1.0, 1.0));

    let cs = apply_container_property("transform", declaration_value("transform: translate(30px)"));
    let Some(t) = cs.transform else {
        panic!("expected a transform, got None");
    };
    assert_eq!((t.dx, t.dy), (30.0, 0.0));
}

#[test]
fn transform_translatex() {
    let cs = apply_container_property(
        "transform",
        declaration_value("transform: translateX(40px)"),
    );
    let Some(t) = cs.transform else {
        panic!("expected a transform, got None");
    };
    assert_eq!((t.dx, t.dy), (40.0, 0.0));
    assert_eq!((t.m11, t.m22), (1.0, 1.0));
}

#[test]
fn transform_translatey() {
    let cs = apply_container_property(
        "transform",
        declaration_value("transform: translateY(-15px)"),
    );
    let Some(t) = cs.transform else {
        panic!("expected a transform, got None");
    };
    assert_eq!((t.dx, t.dy), (0.0, -15.0));
    assert_eq!((t.m11, t.m22), (1.0, 1.0));
}

#[test]
fn transform_list_composes_left_to_right() {
    let cs = apply_container_property(
        "transform",
        declaration_value("transform: rotate(90deg) scale(2)"),
    );
    let Some(t) = cs.transform else {
        panic!("expected a transform, got None");
    };
    // scale(2).then(rotate(90deg)) applied to (1,0) => rotate first => (0,1), then scale => (0,2).
    let (x, y) = t.apply(1.0, 0.0);
    assert!((x - 0.0).abs() < 1e-4);
    assert!((y - 2.0).abs() < 1e-4);
}

#[test]
fn transform_unsupported_function_yields_none() {
    let cs = apply_container_property("transform", declaration_value("transform: skew(10deg)"));
    assert_eq!(cs.transform, None);
}
