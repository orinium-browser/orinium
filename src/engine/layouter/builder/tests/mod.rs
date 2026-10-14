#![allow(unused_imports, dead_code)]

use super::*;
use crate::engine::bridge::audio::NullAudioSinkFactory;
use crate::engine::bridge::text::FallbackTextMeasurer;
use crate::engine::css::parser::{CssNodeType, Parser as CssParser};
use crate::engine::html::parser::Parser as HtmlParser;
use crate::engine::layouter::css_resolver::CssResolver;
use crate::engine::layouter::types::ClipPath;
use crate::engine::renderer_model::generate_draw_commands;
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

/// Apply declarations in order to one style so that shorthand resets are
/// observable. Returns `None` when a declaration is rejected.
fn apply_layout_declarations(declarations: &[(&str, CssValue)]) -> Option<Style> {
    let mut style = Style::default();
    let mut container_style = ContainerStyle::default();
    let mut text_style = TextStyle::default();
    let mut text_flow_style = TextFlowStyle::default();
    let mut overflow = Overflow::default();
    for (name, value) in declarations {
        apply_declaration(
            name,
            value,
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
        )?;
    }
    Some(style)
}

/// Apply a `grid-template` shorthand through the stylesheet parser, so tests
/// see the same token stream a real stylesheet produces. Returns `None` when
/// the declaration is invalid.
fn grid_template_style(declaration: &str) -> Option<Style> {
    let sheet = CssParser::new(&format!(".grid {{ grid-template: {declaration} }}"))
        .parse()
        .unwrap();
    let node = sheet.children()[0].children()[0].node();
    let CssNodeType::Declaration { value, .. } = node else {
        panic!("expected a declaration");
    };
    apply_layout_declarations(&[("grid-template", value.clone())])
}

/// Parse a single declaration through the stylesheet parser and return its
/// value, so tests exercise the same token stream a real stylesheet produces.
fn declaration_value(declaration: &str) -> CssValue {
    let sheet = CssParser::new(&format!(".t {{ {declaration} }}"))
        .parse()
        .unwrap();
    let node = sheet.children()[0].children()[0].node();
    let CssNodeType::Declaration { value, .. } = node else {
        panic!("expected a declaration");
    };
    value.clone()
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
        Arc::new(NullAudioSinkFactory),
        InheritedCss::default(),
        ElementChain::default(),
        ColorScheme::Light,
        ScriptingMode::default(),
        (0.0, 0.0),
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

fn find_iframe(info: &InfoNode) -> Option<&InfoNode> {
    if matches!(
        info.kind,
        NodeKind::Container {
            scroll_x: true,
            scroll_y: true,
            ..
        }
    ) {
        return Some(info);
    }
    info.children.iter().find_map(find_iframe)
}

fn is_within(root: &InfoNode, needle: &InfoNode) -> bool {
    if std::ptr::eq(root, needle) {
        return true;
    }
    root.children.iter().any(|child| is_within(child, needle))
}

fn scrolled_containers<'a>(info: &'a InfoNode, out: &mut Vec<&'a InfoNode>) {
    match &info.kind {
        NodeKind::Container {
            scroll_offset_y, ..
        }
        | NodeKind::Custom {
            scroll_offset_y, ..
        } if scroll_offset_y.abs() > 0.001 => {
            out.push(info);
        }
        _ => {}
    }
    for child in &info.children {
        scrolled_containers(child, out);
    }
}

fn body_layout_children(mut node: &LayoutNode) -> &[LayoutChild] {
    loop {
        match node.children.as_slice() {
            [LayoutChild::Node(child)] => node = child,
            [LayoutChild::Node(a), LayoutChild::Node(b)]
                if a.children.is_empty() && !b.children.is_empty() =>
            {
                node = b;
            }
            [LayoutChild::Node(a), LayoutChild::Node(b)]
                if b.children.is_empty() && !a.children.is_empty() =>
            {
                node = a;
            }
            _ => return &node.children,
        }
    }
}

fn count_whitespace_text_info(info: &InfoNode) -> usize {
    let mut n = 0;
    if let NodeKind::Text { text, .. } = &info.kind
        && text.chars().all(is_css_whitespace)
    {
        n += 1;
    }
    n + info
        .children
        .iter()
        .map(count_whitespace_text_info)
        .sum::<usize>()
}

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

fn resolve_color(value: CssValue) -> Color {
    resolve_css_color("test", &value, ColorScheme::Light).expect("color resolves")
}

fn math_result(name: &str, args: Vec<Vec<CssValue>>) -> Option<CalcValue> {
    let text_flow_style = TextFlowStyle::default();
    resolve_calc_value(
        "math",
        &CssValue::Function(name.into(), args),
        &text_flow_style,
    )
}

fn math_deg(v: f32) -> Vec<Vec<CssValue>> {
    vec![vec![CssValue::Length(v, Unit::Deg)]]
}

fn math_num(args: &[f32]) -> Vec<Vec<CssValue>> {
    args.iter().map(|n| vec![CssValue::Number(*n)]).collect()
}

fn expect_number(actual: Option<CalcValue>, expected: f32) {
    let Some(CalcValue::Number(n)) = actual else {
        panic!("expected a number, got {actual:?}");
    };
    assert!((n - expected).abs() <= 1e-4, "expected {expected}, got {n}");
}

fn expect_angle(actual: Option<CalcValue>, expected_deg: f32) {
    let Some(CalcValue::Angle(deg)) = actual else {
        panic!("expected an angle, got {actual:?}");
    };
    assert!(
        (deg - expected_deg).abs() <= 1e-4,
        "expected {expected_deg}deg, got {deg}deg"
    );
}

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

fn rc(css: &str) -> Color {
    resolved_color(css, "color")
}

fn rc_opt(css: &str) -> Option<Color> {
    let stylesheet = CssParser::new(css).parse().unwrap();
    let resolved = CssResolver::resolve(&stylesheet);
    let decl = resolved.iter().find(|d| d.name == "color")?;
    resolve_css_color(&decl.name, &decl.value, ColorScheme::Light)
}

fn resolve_env_function(value: &CssValue) -> Option<CssValue> {
    DeclarationResolver::resolve_var(value, &Properties::new(), &mut HashSet::new())
}

fn env_function(args: Vec<Vec<CssValue>>) -> CssValue {
    CssValue::Function("env".into(), args)
}

fn find_boxes_by_style<'a>(
    layout: &'a LayoutNode,
    info: &'a InfoNode,
    pred: &dyn Fn(&ContainerStyle) -> bool,
) -> Vec<&'a ui_layout::LayoutBox> {
    let mut out = Vec::new();
    if let NodeKind::Container { style, .. } = &info.kind
        && pred(style)
    {
        out.push(&layout.layout_box);
    }
    for (child, child_info) in layout.children.iter().zip(info.children.iter()) {
        if let LayoutChild::Node(n) = child {
            out.extend(find_boxes_by_style(n, child_info, pred));
        }
    }
    out
}

fn box_for(
    layout: &LayoutNode,
    info: &InfoNode,
    pred: &dyn Fn(&ContainerStyle) -> bool,
) -> ui_layout::Rect {
    find_boxes_by_style(layout, info, pred)
        .into_iter()
        .next()
        .map(|b| b.iter().next().unwrap().border_box)
        .expect("expected exactly one matching box")
}

fn anchor_border_boxes(layout: &LayoutNode, info: &InfoNode) -> Vec<ui_layout::Rect> {
    let mut rects: Vec<_> = find_boxes_by_style(layout, info, &|s| {
        s.anchor.anchors.iter().any(|a| a == "--a")
    })
    .into_iter()
    .map(|b| b.iter().next().unwrap().border_box)
    .collect();
    rects.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap());
    rects
}

mod anchors;
mod cascade_colors;
mod display_flow;
#[cfg(test)]
mod flex;
mod grid;
mod iframe;
mod images_gradients;
mod misc;
mod overflow;
mod position_calc;
mod pseudo;
