use crate::engine::css::parser::{ComplexSelector, CssNode, CssNodeType};
use crate::engine::css::values::CssValue;

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

        for child in rule_node.children() {
            if let CssNodeType::Declaration { name, value } = &child.node() {
                result.push((name.clone(), value.clone()));
            }
        }

        result
    }
}
