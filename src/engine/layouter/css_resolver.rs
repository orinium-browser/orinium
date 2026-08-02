//! A CSS resolver that handles selector matching and value resolution.

use crate::engine::css::parser::{AtQuery, ComplexSelector, CssNode, CssNodeType};
use crate::engine::css::values::{CssIdent, CssValue};

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
        Self::walk(stylesheet, &mut styles, &mut order, origin);
        styles
    }

    fn walk(node: &CssNode, styles: &mut ResolvedStyles, order: &mut usize, origin: StyleOrigin) {
        Self::resolve_rule(node, styles, order, origin);

        if Self::should_recurse(node) {
            for child in node.children() {
                Self::walk(child, styles, order, origin);
            }
        }
    }

    fn resolve_rule(
        node: &CssNode,
        styles: &mut ResolvedStyles,
        order: &mut usize,
        origin: StyleOrigin,
    ) {
        let CssNodeType::Rule { selectors } = node.node() else {
            return;
        };

        let declarations = DeclarationResolver::collect(node);

        for selector in selectors {
            Self::push_resolved(selector, &declarations, styles, order, origin);
        }
    }

    fn push_resolved(
        selector: &ComplexSelector,
        declarations: &[Declaration],
        styles: &mut ResolvedStyles,
        order: &mut usize,
        origin: StyleOrigin,
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
            });
            *order += 1;
        }
    }

    fn should_recurse(node: &CssNode) -> bool {
        if let CssNodeType::AtRule { name, params } = node.node() {
            Self::evaluate_at_rule(name, params)
        } else {
            true
        }
    }

    fn evaluate_at_rule(name: &str, params: &AtQuery) -> bool {
        if name.eq_ignore_ascii_case("supports") {
            SupportsEvaluator::evaluate(params)
        } else {
            true
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
    fn collect(rule_node: &CssNode) -> Vec<Declaration> {
        let mut custom_props: CustomProperties = HashMap::new();
        let mut result = Vec::new();

        for child in rule_node.children() {
            if let CssNodeType::Declaration { name, value } = &child.node()
                && name.starts_with("--")
            {
                custom_props.insert(name.into(), value.clone());
            }
        }

        for child in rule_node.children() {
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
