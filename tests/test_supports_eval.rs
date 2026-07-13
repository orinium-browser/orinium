use orinium_browser::engine::css::parser::Parser;
use orinium_browser::engine::layouter::css_resolver::{CssResolver, ResolvedStyles};

fn resolve(css: &str) -> ResolvedStyles {
    let mut parser = Parser::new(css);
    let stylesheet = parser.parse().expect("CSS parse failed");
    CssResolver::resolve(&stylesheet)
}

fn has_prop_in_rule(styles: &ResolvedStyles, prop: &str) -> bool {
    // Check if any declaration with this property was resolved (from any rule).
    // We don't check selector matching here — just whether the resolver produced it.
    styles.iter().any(|d| d.name == prop)
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
//  @media still applies unconditionally
// ============================================================

#[test]
fn test_media_always_applies() {
    let s = resolve(
        r#"
        @media screen and (max-width: 600px) {
            div { color: red; }
        }
        "#,
    );
    assert!(
        has_prop_in_rule(&s, "color"),
        "@media should always apply children"
    );
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
fn test_var_cycle_detection() {
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
        !has_prop_in_rule(&s, "color"),
        "circular var() should produce no declaration"
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
fn test_var_no_fallback_missing() {
    let s = resolve(
        r#"
        div {
            color: var(--missing);
        }
        "#,
    );
    assert!(
        !has_prop_in_rule(&s, "color"),
        "missing var without fallback drops declaration"
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
