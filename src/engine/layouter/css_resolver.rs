//! A CSS resolver that handles selector matching and value resolution.

use crate::engine::css::parser::{AtQuery, ComplexSelector, CssNode, CssNodeType};
use crate::engine::css::values::{CssIdent, CssValue, Unit};
use crate::engine::layouter::types::ColorScheme;

use std::collections::{HashMap, HashSet};

type CustomProperties = HashMap<CssIdent, CssValue>;

struct Declaration {
    name: String,
    value: CssValue,
    important: bool,
}

/// Origin of a declaration in the CSS cascade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StyleOrigin {
    /// Browser-provided default styling.
    UserAgent,
    /// Styling supplied by the loaded document.
    Author,
}

/// A single CSS declaration after selector resolution and value processing.
///
/// `ResolvedDeclaration` represents one property-value pair that has been
/// fully associated with a selector and enriched with all information
/// required for CSS cascade resolution.
///
/// This structure is produced after:
/// - Parsing selectors
/// - Resolving `var()` using custom properties
/// - Computing selector specificity
///
/// During the cascade phase, multiple `ResolvedDeclaration`s with the same
/// property name may compete. The winner is determined by comparing:
///
/// 1. `important` declarations
/// 2. cascade `origin`
/// 3. `specificity` (higher specificity wins)
/// 4. `order` (later declarations win)
#[derive(Debug, Clone)]
pub struct ResolvedDeclaration {
    pub selector: ComplexSelector,
    pub name: String,
    pub value: CssValue,
    pub specificity: (u32, u32, u32),
    pub order: usize,
    pub important: bool,
    pub origin: StyleOrigin,
    /// Nested `@media` conditions which must all match before this declaration applies.
    pub media_queries: Vec<AtQuery>,
}

pub type ResolvedStyles = Vec<ResolvedDeclaration>;

impl ResolvedDeclaration {
    /// Returns whether this declaration wins over another matching declaration.
    pub fn outranks(&self, other: &Self) -> bool {
        (self.important, self.origin, self.specificity, self.order)
            > (
                other.important,
                other.origin,
                other.specificity,
                other.order,
            )
    }

    /// Returns whether every enclosing `@media` rule matches `environment`.
    pub fn matches_media(&self, environment: &MediaEnvironment) -> bool {
        self.media_queries
            .iter()
            .all(|query| MediaEvaluator::evaluate(query, environment))
    }
}

/// Values used to evaluate media queries for the current page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MediaEnvironment {
    /// Width of the page viewport in CSS pixels.
    pub viewport_width: f32,
    /// Height of the page viewport in CSS pixels.
    pub viewport_height: f32,
    /// Operating-system color preference used by `prefers-color-scheme`.
    pub color_scheme: ColorScheme,
}

impl MediaEnvironment {
    pub fn new(viewport: (f32, f32), color_scheme: ColorScheme) -> Self {
        Self {
            viewport_width: viewport.0,
            viewport_height: viewport.1,
            color_scheme,
        }
    }
}

/// Keeps only declarations whose enclosing media queries currently match.
pub fn filter_media(styles: &ResolvedStyles, environment: &MediaEnvironment) -> ResolvedStyles {
    styles
        .iter()
        .filter(|declaration| declaration.matches_media(environment))
        .cloned()
        .collect()
}

/// Appends resolved declarations while preserving source order across stylesheets.
pub fn append_resolved_styles(target: &mut ResolvedStyles, mut incoming: ResolvedStyles) {
    let next_order = target
        .iter()
        .map(|declaration| declaration.order)
        .max()
        .map_or(0, |order| order + 1);
    for declaration in &mut incoming {
        declaration.order += next_order;
    }
    target.extend(incoming);
}

/// Resolves a `style` attribute's declarations into (name, value, important)
/// triples, applying `var()` and `!important` handling like a rule block.
///
/// Inline styles participate in the cascade as author-origin declarations with
/// the highest specificity, so callers should apply them after stylesheet
/// declarations for the same element.
pub fn resolve_inline_style(style_attr: &str) -> Vec<(String, CssValue, bool)> {
    let mut parser = crate::engine::css::parser::Parser::new(style_attr);
    let Ok(nodes) = parser.parse_declarations() else {
        return Vec::new();
    };

    DeclarationResolver::collect(&nodes)
        .into_iter()
        .map(|declaration| (declaration.name, declaration.value, declaration.important))
        .collect()
}

pub fn resolve_inline_value(value: &str) -> Option<CssValue> {
    let mut tokenizer = crate::engine::css::tokenizer::Tokenizer::new(value);
    let mut tokens = Vec::new();

    loop {
        let token = tokenizer.next_token();
        if token == crate::engine::css::tokenizer::Token::EOF {
            break;
        }

        tokens.push(token);
    }

    let Ok(value) = crate::engine::css::parser::Parser::parse_tokens_to_css_value(tokens) else {
        return None;
    };

    Some(value)
}

// ============================================================
//  CssResolver — tree walk + rule resolution
// ============================================================

pub struct CssResolver;

impl CssResolver {
    /// Resolves an author stylesheet into declarations used by layout.
    pub fn resolve(stylesheet: &CssNode) -> ResolvedStyles {
        Self::resolve_with_origin(stylesheet, StyleOrigin::Author)
    }

    /// Resolves a stylesheet using the supplied cascade origin.
    pub fn resolve_with_origin(stylesheet: &CssNode, origin: StyleOrigin) -> ResolvedStyles {
        let mut styles = Vec::new();
        let mut order = 0;
        Self::walk(stylesheet, &mut styles, &mut order, origin, &mut Vec::new());
        styles
    }

    /// Parses declarations from an HTML `style` attribute.
    ///
    /// Inline declarations use author origin but outrank selector-based author
    /// rules of the same importance. Author `!important` declarations still
    /// outrank normal inline declarations, as required by the cascade.
    pub fn resolve_inline_style(style: &str) -> ResolvedStyles {
        let source = format!("* {{ {style} }}");
        let Ok(stylesheet) = crate::engine::css::parser::Parser::new(&source).parse() else {
            return Vec::new();
        };
        let mut declarations = Self::resolve_with_origin(&stylesheet, StyleOrigin::Author);
        for declaration in &mut declarations {
            declaration.specificity = (u32::MAX, u32::MAX, u32::MAX);
            declaration.order = usize::MAX;
        }
        declarations
    }

    fn walk(
        node: &CssNode,
        styles: &mut ResolvedStyles,
        order: &mut usize,
        origin: StyleOrigin,
        media_queries: &mut Vec<AtQuery>,
    ) {
        if let CssNodeType::AtRule { name, params } = node.node() {
            if name.eq_ignore_ascii_case("supports") && !SupportsEvaluator::evaluate(params) {
                return;
            }

            let is_media = name.eq_ignore_ascii_case("media");
            if is_media {
                media_queries.push(params.clone());
            }
            for child in node.children() {
                Self::walk(child, styles, order, origin, media_queries);
            }
            if is_media {
                media_queries.pop();
            }
            return;
        }

        Self::resolve_rule(node, styles, order, origin, media_queries);
        for child in node.children() {
            Self::walk(child, styles, order, origin, media_queries);
        }
    }

    fn resolve_rule(
        node: &CssNode,
        styles: &mut ResolvedStyles,
        order: &mut usize,
        origin: StyleOrigin,
        media_queries: &[AtQuery],
    ) {
        let CssNodeType::Rule { selectors } = node.node() else {
            return;
        };

        let declarations = DeclarationResolver::collect(node.children());

        for selector in selectors {
            Self::push_resolved(
                selector,
                &declarations,
                styles,
                order,
                origin,
                media_queries,
            );
        }
    }

    fn push_resolved(
        selector: &ComplexSelector,
        declarations: &[Declaration],
        styles: &mut ResolvedStyles,
        order: &mut usize,
        origin: StyleOrigin,
        media_queries: &[AtQuery],
    ) {
        let specificity = selector.specificity();

        for decl in declarations {
            styles.push(ResolvedDeclaration {
                selector: selector.clone(),
                name: decl.name.clone(),
                value: decl.value.clone(),
                specificity,
                order: *order,
                important: decl.important,
                origin,
                media_queries: media_queries.to_vec(),
            });
            *order += 1;
        }
    }
}

struct MediaEvaluator;

impl MediaEvaluator {
    fn evaluate(query: &AtQuery, environment: &MediaEnvironment) -> bool {
        match query {
            AtQuery::Group(items) => Self::evaluate_group(items, environment),
            item => Self::evaluate_clause(std::slice::from_ref(item), environment),
        }
    }

    fn evaluate_group(items: &[AtQuery], environment: &MediaEnvironment) -> bool {
        items
            .split(|item| matches!(item, AtQuery::Keyword(keyword) if keyword == ","))
            .any(|clause| Self::evaluate_clause(clause, environment))
    }

    fn evaluate_clause(items: &[AtQuery], environment: &MediaEnvironment) -> bool {
        if items.is_empty() {
            return false;
        }
        let negate = matches!(items.first(), Some(AtQuery::Keyword(keyword)) if keyword.eq_ignore_ascii_case("not"));
        let mut saw_operand = false;
        let matches = items
            .iter()
            .skip(usize::from(negate))
            .all(|item| match item {
                AtQuery::Keyword(keyword)
                    if keyword.eq_ignore_ascii_case("and")
                        || keyword.eq_ignore_ascii_case("only") =>
                {
                    true
                }
                AtQuery::Keyword(keyword)
                    if keyword.eq_ignore_ascii_case("all")
                        || keyword.eq_ignore_ascii_case("screen") =>
                {
                    saw_operand = true;
                    true
                }
                AtQuery::Keyword(keyword) if keyword.eq_ignore_ascii_case("print") => {
                    saw_operand = true;
                    false
                }
                AtQuery::Keyword(_) => {
                    saw_operand = true;
                    false
                }
                AtQuery::Condition { name, value } => {
                    saw_operand = true;
                    Self::evaluate_condition(name, value, environment)
                }
                AtQuery::Group(group) => {
                    saw_operand = true;
                    Self::evaluate_group(group, environment)
                }
            });
        if !saw_operand {
            return false;
        }
        if negate { !matches } else { matches }
    }

    fn evaluate_condition(name: &str, value: &CssValue, environment: &MediaEnvironment) -> bool {
        let name = name.to_ascii_lowercase();
        match name.as_str() {
            "width" | "min-width" | "max-width" => {
                Self::compare_length(&name, value, environment.viewport_width, environment)
            }
            "height" | "min-height" | "max-height" => {
                Self::compare_length(&name, value, environment.viewport_height, environment)
            }
            "orientation" => match value {
                CssValue::Keyword(keyword) if keyword.eq_ignore_ascii_case("portrait") => {
                    environment.viewport_height >= environment.viewport_width
                }
                CssValue::Keyword(keyword) if keyword.eq_ignore_ascii_case("landscape") => {
                    environment.viewport_width > environment.viewport_height
                }
                _ => false,
            },
            "prefers-color-scheme" => match value {
                CssValue::Keyword(keyword) if keyword.eq_ignore_ascii_case("dark") => {
                    environment.color_scheme == ColorScheme::Dark
                }
                CssValue::Keyword(keyword) if keyword.eq_ignore_ascii_case("light") => {
                    environment.color_scheme == ColorScheme::Light
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn compare_length(
        name: &str,
        value: &CssValue,
        actual: f32,
        environment: &MediaEnvironment,
    ) -> bool {
        let Some(expected) = Self::length_px(value, environment) else {
            return false;
        };
        if name.starts_with("min-") {
            actual >= expected
        } else if name.starts_with("max-") {
            actual <= expected
        } else {
            (actual - expected).abs() <= f32::EPSILON
        }
    }

    fn length_px(value: &CssValue, environment: &MediaEnvironment) -> Option<f32> {
        match value {
            CssValue::Length(value, Unit::Px) => Some(*value),
            CssValue::Length(value, Unit::Em) | CssValue::Length(value, Unit::Rem) => {
                Some(*value * 16.0)
            }
            CssValue::Length(value, Unit::Vw) => Some(*value * environment.viewport_width / 100.0),
            CssValue::Length(value, Unit::Vh) => Some(*value * environment.viewport_height / 100.0),
            CssValue::Number(0.0) => Some(0.0),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::css::parser::Parser;

    fn resolve(css: &str, origin: StyleOrigin) -> ResolvedStyles {
        let stylesheet = Parser::new(css).parse().unwrap();
        CssResolver::resolve_with_origin(&stylesheet, origin)
    }

    #[test]
    fn author_declaration_outranks_more_specific_user_agent_declaration() {
        let user_agent = resolve(
            r#"input[type="text"] { display: inline-block; }"#,
            StyleOrigin::UserAgent,
        );
        let author = resolve("input { display: block; }", StyleOrigin::Author);

        assert!(author[0].outranks(&user_agent[0]));
    }

    #[test]
    fn append_rebases_order_across_stylesheets() {
        let mut styles = resolve("input { display: inline; }", StyleOrigin::Author);
        let later = resolve("input { display: block; }", StyleOrigin::Author);
        append_resolved_styles(&mut styles, later);

        assert!(styles[1].order > styles[0].order);
        assert!(styles[1].outranks(&styles[0]));
    }

    #[test]
    fn resolve_inline_style_extracts_declarations_and_important() {
        let decls = resolve_inline_style("color: red; margin: 4px 8px !important; width: 10px");
        assert_eq!(decls.len(), 3);

        assert_eq!(decls[0].0, "color");
        assert!(!decls[0].2);

        assert_eq!(decls[1].0, "margin");
        assert!(decls[1].2);

        assert_eq!(decls[2].0, "width");
    }

    #[test]
    fn resolve_inline_style_tolerates_empty_and_malformed_input() {
        assert!(resolve_inline_style("").is_empty());
        assert!(resolve_inline_style(";;;").is_empty());
    }

    #[test]
    fn resolve_inline_style_resolves_var() {
        let decls = resolve_inline_style("--accent: blue; color: var(--accent)");
        assert_eq!(decls.len(), 2);

        let (_, color_value, _) = decls.iter().find(|(n, _, _)| n == "color").unwrap();
        assert_eq!(color_value, &CssValue::Keyword("blue".into()));
    }

    #[test]
    fn media_width_conditions_follow_viewport() {
        let styles = resolve(
            "@media screen and (max-width: 600px) { div { color: red; } }",
            StyleOrigin::Author,
        );
        let narrow = MediaEnvironment::new((600.0, 800.0), ColorScheme::Light);
        let wide = MediaEnvironment::new((601.0, 800.0), ColorScheme::Light);

        assert_eq!(filter_media(&styles, &narrow).len(), 1);
        assert!(filter_media(&styles, &wide).is_empty());
    }

    #[test]
    fn media_query_lists_use_or_semantics() {
        let styles = resolve(
            "@media print, (orientation: landscape) { div { color: red; } }",
            StyleOrigin::Author,
        );
        let landscape = MediaEnvironment::new((800.0, 600.0), ColorScheme::Light);
        let portrait = MediaEnvironment::new((600.0, 800.0), ColorScheme::Light);

        assert_eq!(filter_media(&styles, &landscape).len(), 1);
        assert!(filter_media(&styles, &portrait).is_empty());
    }

    #[test]
    fn media_color_scheme_matches_system_preference() {
        let styles = resolve(
            "@media (prefers-color-scheme: dark) { div { color: white; } }",
            StyleOrigin::Author,
        );
        let light = MediaEnvironment::new((800.0, 600.0), ColorScheme::Light);
        let dark = MediaEnvironment::new((800.0, 600.0), ColorScheme::Dark);

        assert!(filter_media(&styles, &light).is_empty());
        assert_eq!(filter_media(&styles, &dark).len(), 1);
    }

    #[test]
    fn empty_media_query_does_not_match() {
        let styles = resolve("@media { div { color: red; } }", StyleOrigin::Author);
        let environment = MediaEnvironment::new((800.0, 600.0), ColorScheme::Light);

        assert!(filter_media(&styles, &environment).is_empty());
    }
}

// ============================================================
//  SupportsEvaluator — `@supports` condition evaluation
// ============================================================

struct SupportsEvaluator;

impl SupportsEvaluator {
    /// Dispatch on the `AtQuery` AST variant.
    fn evaluate(query: &AtQuery) -> bool {
        match query {
            // `@supports (display: grid)` — a parenthesised group
            AtQuery::Group(items) => Self::evaluate_group(items),
            // `@supports (display: grid)` — the inner condition after unwrapping
            AtQuery::Condition { name, value } => Self::is_supported(name, value),
            // Stray keyword outside a group (malformed input)
            AtQuery::Keyword(_) => false,
        }
    }

    /// Evaluate a group of `@supports` items.
    ///
    /// The parser produces flat groups like:
    /// - `(display: grid)` → `[Group([Condition])]`
    /// - `(A) and (B)` → `[Group, Keyword("and"), Group]`
    /// - `not (A)` → `[Keyword("not"), Group]`
    /// - `(A) or (B)` → `[Group, Keyword("or"), Group]`
    fn evaluate_group(items: &[AtQuery]) -> bool {
        if items.is_empty() {
            return false;
        }

        // `not (display: grid)` — negate
        if matches!(items.first(), Some(AtQuery::Keyword(k)) if k.eq_ignore_ascii_case("not")) {
            return items.len() > 1 && !Self::evaluate(&AtQuery::Group(items[1..].to_vec()));
        }

        // `(display: flex) and (gap: 10px)` — all operands must be supported
        if let Some(operands) = Self::split_by_keyword(items, "and") {
            operands.iter().all(|g| Self::evaluate(g))
        // `(display: flex) or (display: grid)` — at least one must be supported
        } else if let Some(operands) = Self::split_by_keyword(items, "or") {
            operands.iter().any(|g| Self::evaluate(g))
        // Single group — unwrap one level
        } else if items.len() == 1 {
            Self::evaluate(&items[0])
        } else {
            items.iter().all(Self::evaluate)
        }
    }

    fn is_supported(name: &str, value: &CssValue) -> bool {
        super::builder::apply_declaration(
            name,
            value,
            &mut ui_layout::Style::default(),
            &mut super::types::ContainerStyle::default(),
            &mut super::types::TextStyle::default(),
            &mut super::types::Overflow::default(),
            super::types::ColorScheme::Light,
        )
        .is_some()
    }

    fn split_by_keyword<'a>(items: &'a [AtQuery], keyword: &str) -> Option<Vec<&'a AtQuery>> {
        let has = items
            .iter()
            .any(|item| matches!(item, AtQuery::Keyword(k) if k.eq_ignore_ascii_case(keyword)));
        if !has {
            return None;
        }
        Some(
            items
                .iter()
                .filter(
                    |item| !matches!(item, AtQuery::Keyword(k) if k.eq_ignore_ascii_case(keyword)),
                )
                .collect(),
        )
    }
}

// ============================================================
//  DeclarationResolver — `!important` extraction, `var()` resolution
// ============================================================

struct DeclarationResolver;

impl DeclarationResolver {
    fn collect(children: &[CssNode]) -> Vec<Declaration> {
        let mut custom_props: CustomProperties = HashMap::new();
        let mut result = Vec::new();

        for child in children {
            if let CssNodeType::Declaration { name, value } = &child.node()
                && name.starts_with("--")
            {
                custom_props.insert(name.into(), value.clone());
            }
        }

        for child in children {
            if let CssNodeType::Declaration { name, value } = &child.node() {
                let (raw_value, important) = Self::extract_important(value);

                if name.starts_with("--") {
                    // `--accent: blue` — custom property, keep as-is
                    result.push(Declaration {
                        name: name.clone(),
                        value: raw_value,
                        important,
                    });
                } else if let Some(resolved) =
                    Self::resolve_var(&raw_value, &custom_props, &mut HashSet::new())
                {
                    // `color: var(--accent)` — resolve var() then emit
                    result.push(Declaration {
                        name: name.clone(),
                        value: resolved,
                        important,
                    });
                }
            }
        }

        result
    }

    /// Extract `!important` from a CSS value.
    ///
    /// `border: 1px solid black !important` is parsed as a `List` where the
    /// last two items are `Keyword("!")` and `Keyword("important")`.
    fn extract_important(value: &CssValue) -> (CssValue, bool) {
        match value {
            CssValue::List(list) if list.len() >= 2 => {
                let len = list.len();
                let is_important = matches!(
                    (&list[len - 2], &list[len - 1]),
                    (
                        CssValue::Keyword(bang),
                        CssValue::Keyword(ident)
                    )
                    if bang == "!" && ident.eq_ignore_ascii_case("important")
                );

                if is_important {
                    let value = if len - 2 == 1 {
                        list.iter().next().unwrap().clone()
                    } else {
                        CssValue::List(list[..len - 2].to_vec())
                    };
                    (value, true)
                } else {
                    (value.clone(), false)
                }
            }
            _ => (value.clone(), false),
        }
    }

    fn resolve_var(
        value: &CssValue,
        custom_props: &CustomProperties,
        visited: &mut HashSet<CssIdent>,
    ) -> Option<CssValue> {
        match value {
            // `var(--accent)` / `var(--missing, red)` — resolve the custom property
            CssValue::Function(name, args) if name == "var" => {
                let var_name = match args.first() {
                    Some(CssValue::Keyword(name)) => name,
                    _ => return None,
                };

                if !visited.insert(var_name.clone()) {
                    return None;
                }

                let result = if let Some(v) = custom_props.get(var_name) {
                    Self::resolve_var(v, custom_props, visited)
                } else if let Some(fallback) = args.get(1) {
                    Self::resolve_var(fallback, custom_props, visited)
                } else {
                    None
                };

                visited.remove(var_name);
                result
            }

            // `rgb(var(--r), var(--g), var(--b))` — resolve args independently
            CssValue::Function(name, args) => {
                let resolved_args = args
                    .iter()
                    .map(|v| Self::resolve_var(v, custom_props, &mut visited.clone()))
                    .collect::<Option<Vec<_>>>()?;
                Some(CssValue::Function(name.clone(), resolved_args))
            }

            // `1px solid var(--color)` — resolve each item independently
            CssValue::List(list) => {
                let resolved = list
                    .iter()
                    .map(|v| Self::resolve_var(v, custom_props, &mut visited.clone()))
                    .collect::<Option<Vec<_>>>()?;
                Some(CssValue::List(resolved))
            }

            // `10px` / `"hello"` / `#fff` — already concrete, pass through
            _ => Some(value.clone()),
        }
    }
}
