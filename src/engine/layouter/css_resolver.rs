use crate::engine::css::parser::{ComplexSelector, CssNode, CssNodeType};
use crate::engine::css::values::CssValue;

use std::collections::HashMap;

type CustomProperties = HashMap<String, CssValue>;

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
/// 1. `specificity` (higher specificity wins)
/// 2. `order` (later declarations win)
#[derive(Debug, Clone)]
pub struct ResolvedDeclaration {
    /// The selector this declaration originates from.
    pub selector: ComplexSelector,

    /// The CSS property name (e.g. `"color"`, `"margin-top"`).
    pub name: String,

    /// The resolved CSS value for the property.
    /// This value has already had `var()` functions expanded.
    pub value: CssValue,

    /// The specificity of the selector, represented as (a, b, c).
    /// - a: ID selectors
    /// - b: class, attribute, and pseudo-class selectors
    /// - c: type and pseudo-element selectors
    pub specificity: (u32, u32, u32),

    /// The source order of the declaration.
    /// Higher values indicate declarations that appear later in the stylesheet.
    pub order: usize,

    /// Whether this declaration is marked as `!important`.
    pub important: bool,
}

pub type ResolvedStyles = Vec<ResolvedDeclaration>;

pub struct CssResolver;

impl CssResolver {
    pub fn resolve(stylesheet: &CssNode) -> ResolvedStyles {
        let mut styles = Vec::new();
        let mut order = 0;
        Self::walk(stylesheet, &mut styles, &mut order);
        styles
    }

    fn walk(node: &CssNode, styles: &mut ResolvedStyles, order: &mut usize) {
        if let CssNodeType::Rule { selectors } = &node.node() {
            let declarations = Self::collect_declarations(node);

            for selector in selectors {
                let specificity = selector.specificity();

                for (name, value, important) in &declarations {
                    styles.push(ResolvedDeclaration {
                        selector: selector.clone(),
                        name: name.clone(),
                        value: value.clone(),
                        specificity,
                        order: *order,
                        important: *important,
                    });
                    *order += 1;
                }
            }
        }

        for child in node.children() {
            Self::walk(child, styles, order);
        }
    }

    fn collect_declarations(rule_node: &CssNode) -> Vec<(String, CssValue, bool)> {
        let mut result = Vec::new();
        let mut custom_props: CustomProperties = HashMap::new();

        // 1. custom property を先に集める
        for child in rule_node.children() {
            if let CssNodeType::Declaration { name, value } = &child.node()
                && name.starts_with("--")
            {
                custom_props.insert(name.clone(), value.clone());
            }
        }

        // 2. 通常の declaration を var 解決して追加
        for child in rule_node.children() {
            if let CssNodeType::Declaration { name, value } = &child.node() {
                let (raw_value, important) = Self::extract_important(value);

                if name.starts_with("--") {
                    result.push((name.clone(), raw_value, important));
                } else if let Some(resolved) = Self::resolve_var(&raw_value, &custom_props) {
                    result.push((name.clone(), resolved, important));
                }
            }
        }

        result
    }

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
                    return (value, true);
                }

                (value.clone(), false)
            }
            _ => (value.clone(), false),
        }
    }

    fn resolve_var(value: &CssValue, custom_props: &CustomProperties) -> Option<CssValue> {
        match value {
            CssValue::Function(name, args) if name == "var" => {
                // var(--x [, fallback])
                let var_name = match args.first() {
                    Some(CssValue::Keyword(name)) => name,
                    _ => return None,
                };

                if let Some(v) = custom_props.get(var_name) {
                    Self::resolve_var(v, custom_props)
                } else if let Some(fallback) = args.get(1) {
                    Self::resolve_var(fallback, custom_props)
                } else {
                    None
                }
            }

            CssValue::Function(name, args) => {
                let resolved_args = args
                    .iter()
                    .map(|v| Self::resolve_var(v, custom_props))
                    .collect::<Option<Vec<_>>>()?;
                Some(CssValue::Function(name.clone(), resolved_args))
            }

            CssValue::List(list) => {
                let resolved = list
                    .iter()
                    .map(|v| Self::resolve_var(v, custom_props))
                    .collect::<Option<Vec<_>>>()?;
                Some(CssValue::List(resolved))
            }

            _ => Some(value.clone()),
        }
    }
}
