use orinium_browser::engine::css::parser::Parser;
use orinium_browser::engine::layouter::css_resolver::{
    CssResolver, MediaEnvironment, ResolvedStyles, filter_media,
};
use orinium_browser::engine::layouter::types::ColorScheme;

fn resolve(css: &str) -> ResolvedStyles {
    let mut parser = Parser::new(css);
    let stylesheet = parser.parse().expect("CSS parse failed");
    CssResolver::resolve(&stylesheet)
}

fn has_prop_in_rule<'a>(styles: impl IntoIterator<Item = &'a orinium_browser::engine::layouter::css_resolver::ResolvedDeclaration>, prop: &str) -> bool {
    // Check if any declaration with this property was resolved (from any rule).
    // We don't check selector matching here — just whether the resolver produced it.
    styles.into_iter().any(|d| d.name == prop)
}

// ============================================================
//  @supports — supported / unsupported property detection
// ============================================================
//
// `apply_declaration` returns `None` (unsupported) for some known-property
// + unknown-value combos, e.g. `flex-direction: banana`.
// It returns `Some(())` (silently accepted) for truly unknown property names
// and for most display values — so we use the former pattern here.

#[test]
fn test_supports_supported_applies() {
    let s = resolve(
        r#"
        @supports (flex-direction: row) {
            div { color: red; }
        }
        "#,
    );
    assert!(
        has_prop_in_rule(&s, "color"),
        "flex-direction: row is supported"
    );
}

#[test]
fn test_supports_unsupported_skipped() {
    let s = resolve(
        r#"
        @supports (flex-direction: banana) {
            div { color: red; }
        }
        "#,
    );
    assert!(
        !has_prop_in_rule(&s, "color"),
        "flex-direction: banana is not supported"
    );
}

#[test]
fn test_supports_and_both_supported() {
    let s = resolve(
        r#"
        @supports (flex-direction: row) and (justify-content: center) {
            div { color: red; }
        }
        "#,
    );
    assert!(has_prop_in_rule(&s, "color"));
}

#[test]
fn test_supports_and_one_unsupported() {
    let s = resolve(
        r#"
        @supports (flex-direction: row) and (justify-content: banana) {
            div { color: red; }
        }
        "#,
    );
    assert!(!has_prop_in_rule(&s, "color"), "AND requires both to pass");
}

#[test]
fn test_supports_or_one_supported() {
    let s = resolve(
        r#"
        @supports (flex-direction: row) or (justify-content: banana) {
            div { color: red; }
        }
        "#,
    );
    assert!(
        has_prop_in_rule(&s, "color"),
        "OR passes if one is supported"
    );
}

#[test]
fn test_supports_or_none_supported() {
    let s = resolve(
        r#"
        @supports (flex-direction: banana) or (justify-content: banana) {
            div { color: red; }
        }
        "#,
    );
    assert!(
        !has_prop_in_rule(&s, "color"),
        "OR fails if both are unsupported"
    );
}

#[test]
fn test_supports_not_supported_property() {
    // flex-direction: row IS supported → not → should NOT apply
    let s = resolve(
        r#"
        @supports not (flex-direction: row) {
            div { color: red; }
        }
        "#,
    );
    assert!(!has_prop_in_rule(&s, "color"));
}

#[test]
fn test_supports_not_unsupported_property() {
    // flex-direction: banana is NOT supported → not → SHOULD apply
    let s = resolve(
        r#"
        @supports not (flex-direction: banana) {
            div { color: red; }
        }
        "#,
    );
    assert!(has_prop_in_rule(&s, "color"));
}

// ============================================================
//  @media
// ============================================================

#[test]
fn test_media_applies_only_when_environment_matches() {
    let s = resolve(
        r#"
        @media screen and (max-width: 600px) {
            div { color: red; }
        }
        "#,
    );
    let narrow = MediaEnvironment::new((600.0, 800.0), ColorScheme::Light);
    let wide = MediaEnvironment::new((800.0, 600.0), ColorScheme::Light);
    assert!(has_prop_in_rule(filter_media(&s, &narrow), "color"));
    assert!(!has_prop_in_rule(filter_media(&s, &wide), "color"));
}

// ============================================================
//  var() resolution
// ============================================================

#[test]
fn test_var_resolution_in_same_rule() {
    let s = resolve(
        r#"
        div {
            --accent: blue;
            color: var(--accent);
        }
        "#,
    );
    assert!(has_prop_in_rule(&s, "color"));
}

#[test]
fn test_var_cycle_is_deferred_to_the_element_cascade() {
    let s = resolve(
        r#"
        div {
            --a: var(--b);
            --b: var(--a);
            color: var(--a);
        }
        "#,
    );
    assert!(
        has_prop_in_rule(&s, "color"),
        "var() validity is determined after inherited and selector custom properties cascade"
    );
}

#[test]
fn test_var_fallback() {
    let s = resolve(
        r#"
        div {
            color: var(--missing, red);
        }
        "#,
    );
    assert!(
        has_prop_in_rule(&s, "color"),
        "fallback should be used when var is undefined"
    );
}

#[test]
fn test_var_no_fallback_is_deferred_to_the_element_cascade() {
    let s = resolve(
        r#"
        div {
            color: var(--missing);
        }
        "#,
    );
    assert!(
        has_prop_in_rule(&s, "color"),
        "a custom property missing from this rule may be supplied by the element cascade"
    );
}

// ============================================================
//  @supports + var() together
// ============================================================

#[test]
fn test_supports_condition_with_var() {
    // var(--accent) resolves to blue → color: blue is supported
    let s = resolve(
        r#"
        div {
            --accent: blue;
            color: var(--accent);
        }
        "#,
    );
    assert!(has_prop_in_rule(&s, "color"));
}

#[test]
fn test_supports_blocks_inner_rule_with_var() {
    let s = resolve(
        r#"
        @supports (flex-direction: row) {
            div {
                --accent: blue;
                color: var(--accent);
            }
        }
        "#,
    );
    assert!(
        has_prop_in_rule(&s, "color"),
        "inner rule var() should resolve within same rule"
    );
}

#[test]
fn test_unsupported_blocks_inner_var_declaration() {
    let s = resolve(
        r#"
        @supports (flex-direction: banana) {
            div {
                --accent: blue;
                color: var(--accent);
            }
        }
        "#,
    );
    assert!(
        !has_prop_in_rule(&s, "color"),
        "unsupported @supports should block inner declarations"
    );
}

#[test]
fn test_supports_flex_shrink() {
    let s = resolve(
        r#"
        @supports (flex-shrink: 0) {
            div {
                --val: supported;
            }
        }
        "#,
    );
    assert!(
        has_prop_in_rule(&s, "--val"),
        "flex-shrink should be recognized"
    );
}

#[test]
fn test_supports_align_self() {
    let s = resolve(
        r#"
        @supports (align-self: center) {
            div {
                --val: supported;
            }
        }
        "#,
    );
    assert!(
        has_prop_in_rule(&s, "--val"),
        "align-self should be recognized"
    );
}

#[test]
fn test_supports_column_gap() {
    let s = resolve(
        r#"
        @supports (column-gap: 10px) {
            div {
                --val: supported;
            }
        }
        "#,
    );
    assert!(
        has_prop_in_rule(&s, "--val"),
        "column-gap should be recognized"
    );
}

#[test]
fn test_supports_row_gap() {
    let s = resolve(
        r#"
        @supports (row-gap: 1em) {
            div {
                --val: supported;
            }
        }
        "#,
    );
    assert!(
        has_prop_in_rule(&s, "--val"),
        "row-gap should be recognized"
    );
}

#[test]
fn test_supports_display_contents_unsupported() {
    let s = resolve(
        r#"
        @supports (display: contents) {
            div {
                --val: supported;
            }
        }
        "#,
    );
    assert!(
        !has_prop_in_rule(&s, "--val"),
        "display: contents is not supported by layout engine"
    );
}

#[test]
fn test_supports_display_inline_block_supported() {
    let s = resolve(
        r#"
        @supports (display: inline-block) {
            div {
                --val: supported;
            }
        }
        "#,
    );
    assert!(
        has_prop_in_rule(&s, "--val"),
        "display: inline-block should be supported by layout engine"
    );
}

#[test]
fn test_supports_min_function() {
    let s = resolve(
        r#"
        @supports (width: min(100px, 200px)) {
            div {
                --val: supported;
            }
        }
        "#,
    );
    assert!(
        has_prop_in_rule(&s, "--val"),
        "min() function should be recognized"
    );
}

#[test]
fn test_supports_max_function() {
    let s = resolve(
        r#"
        @supports (width: max(100px, 200px)) {
            div {
                --val: supported;
            }
        }
        "#,
    );
    assert!(
        has_prop_in_rule(&s, "--val"),
        "max() function should be recognized"
    );
}

#[test]
fn test_supports_min_function_with_percent() {
    let s = resolve(
        r#"
        @supports (width: min(100%, 500px)) {
            div {
                --val: supported;
            }
        }
        "#,
    );
    assert!(
        has_prop_in_rule(&s, "--val"),
        "min() with percent should be recognized"
    );
}

#[test]
fn test_supports_max_function_with_vw() {
    let s = resolve(
        r#"
        @supports (width: max(200px, 50vw)) {
            div {
                --val: supported;
            }
        }
        "#,
    );
    assert!(
        has_prop_in_rule(&s, "--val"),
        "max() with vw should be recognized"
    );
}

#[test]
fn test_supports_min_function_three_args() {
    let s = resolve(
        r#"
        @supports (width: min(100px, 50vw, 500px)) {
            div {
                --val: supported;
            }
        }
        "#,
    );
    assert!(
        has_prop_in_rule(&s, "--val"),
        "min() with three args should be recognized"
    );
}

#[test]
fn test_supports_max_function_single_arg_unsupported() {
    let s = resolve(
        r#"
        @supports (width: max(100px)) {
            div {
                --val: supported;
            }
        }
        "#,
    );
    assert!(
        !has_prop_in_rule(&s, "--val"),
        "max() with single arg should not be recognized"
    );
}

#[test]
fn test_supports_calc_in_min() {
    let s = resolve(
        r#"
        @supports (width: min(calc(100% - 20px), 500px)) {
            div {
                --val: supported;
            }
        }
        "#,
    );
    assert!(
        has_prop_in_rule(&s, "--val"),
        "calc() inside min() should be recognized"
    );
}
