use crate::engine::css::parser::{ComplexSelector, CssNode, CssNodeType};
use crate::engine::css::values::CssValue;

use std::collections::HashMap;

type CustomProperties = HashMap<String, CssValue>;

/// complex selector -> declarations
pub type ResolvedStyles = Vec<(ComplexSelector, Vec<(String, CssValue)>)>;

pub struct CssResolver;

impl CssResolver {
    pub fn resolve(stylecheet: &CssNode) -> ResolvedStyles {
        let mut styles = Vec::new();
        Self::walk(stylecheet, &mut styles);
        styles
    }

    fn walk(node: &CssNode, styles: &mut ResolvedStyles) {
        if let CssNodeType::Rule { selectors } = &node.node() {
            let declarations = Self::collect_declarations(node);

            for selector in selectors {
                styles.push((selector.clone(), declarations.clone()));
            }
        }

        for child in node.children() {
            Self::walk(child, styles);
        }
    }

    fn collect_declarations(rule_node: &CssNode) -> Vec<(String, CssValue)> {
        let mut result = Vec::new();
        let mut custom_props: CustomProperties = HashMap::new();

        // 1. custom property を先に集める
        for child in rule_node.children() {
            if let CssNodeType::Declaration { name, value } = &child.node() {
                if name.starts_with("--") {
                    custom_props.insert(name.clone(), value.clone());
                }
            }
        }

        // 2. 通常の declaration を var 解決して追加
        for child in rule_node.children() {
            if let CssNodeType::Declaration { name, value } = &child.node() {
                if name.starts_with("--") {
                    // custom property 自体も残す（後続フェーズ用）
                    result.push((name.clone(), value.clone()));
                } else if let Some(resolved) = Self::resolve_var(value, &custom_props) {
                    result.push((name.clone(), resolved));
                }
            }
        }

        result
    }

    fn resolve_var(value: &CssValue, custom_props: &CustomProperties) -> Option<CssValue> {
        match value {
            CssValue::Function(name, args) if name == "var" => {
                // var(--x [, fallback])
                let var_name = match args.get(0) {
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
