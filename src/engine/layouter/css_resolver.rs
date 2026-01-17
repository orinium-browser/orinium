use crate::engine::css::parser::{ComplexSelector, CssNode, CssNodeType};

/// complex selector -> declarations
pub type ResolvedStyles = Vec<(ComplexSelector, Vec<(String, Vec<Token>)>)>;

pub struct CssResolver;

impl CssResolver {
    pub fn resolve(stylecheet: &CssNode) -> ResolvedStyles {
        let mut styles = Vec::new();
        Self::walk(&stylecheet, &mut styles);
        styles
    }

    fn walk(node: &CssNode, styles: &mut ResolvedStyles) {
        if let CssNodeType::Rule { selectors } = &node {
            let declarations = Self::collect_declarations(node);

            for selector in selectors {
                styles.push((selector.clone(), declarations.clone()));
            }
        }

        for child in node_ref.children() {
            Self::walk(child, styles);
        }
    }

    fn collect_declarations(rule_node: &CssNode) -> Vec<(String, Token)> {
        let mut result = Vec::new();

        for child in rule_node.children() {
            if let CssNodeType::Declaration { name, value } = &child {
                result.push((name.clone(), value.clone()));
            }
        }

        result
    }
}
