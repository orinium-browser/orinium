use std::collections::HashMap;
use std::rc::Rc
use crate::engine::css::cssom::{CssNodeType, CssValue};
use crate::engine::tree::{Tree, TreeNode};

/// Map: selector (e.g. "body", "p") -> declarations
pub type ResolvedStyles = HashMap<String, Vec<(String, CssValue)>>;

/// CSS resolver
pub struct CssResolver;

impl CssResolver {
    /// Resolve a CSS tree into selector-based styles
    pub fn resolve(css_tree: &Tree<CssNodeType>) -> ResolvedStyles {
        let mut styles = HashMap::new();
        Self::walk(&css_tree.root, &mut styles);
        styles
    }

    /// Walk CSS AST recursively
    fn walk(
        node: &Rc<std::cell::RefCell<TreeNode<CssNodeType>>>,
        styles: &mut ResolvedStyles,
    ) {
        let node_ref = node.borrow();

        match &node_ref.value {
            CssNodeType::Rule { selectors } => {
                let declarations = Self::collect_declarations(node);

                for selector in selectors {
                    styles
                        .entry(selector.clone())
                        .or_default()
                        .extend(declarations.clone());
                }
            }
            _ => {}
        }

        for child in node_ref.children() {
            Self::walk(child, styles);
        }
    }

    /// Collect declarations under a rule node
    fn collect_declarations(
        rule_node: &std::rc::Rc<std::cell::RefCell<TreeNode<CssNodeType>>>,
    ) -> Vec<(String, CssValue)> {
        let mut result = Vec::new();

        for child in rule_node.borrow().children() {
            if let CssNodeType::Declaration { name, value } = &child.borrow().value {
                result.push((name.clone(), value.clone()));
            }
        }

        result
    }
}
