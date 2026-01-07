use crate::engine::css::cssom::{ComplexSelector, CssNodeType, CssValue};
use crate::engine::tree::{Tree, TreeNode};
use std::rc::Rc;

/// complex selector -> declarations
pub type ResolvedStyles = Vec<(ComplexSelector, Vec<(String, CssValue)>)>;

pub struct CssResolver;

impl CssResolver {
    pub fn resolve(css_tree: &Tree<CssNodeType>) -> ResolvedStyles {
        let mut styles = Vec::new();
        Self::walk(&css_tree.root, &mut styles);
        styles
    }

    fn walk(node: &Rc<std::cell::RefCell<TreeNode<CssNodeType>>>, styles: &mut ResolvedStyles) {
        let node_ref = node.borrow();

        if let CssNodeType::Rule { selectors } = &node_ref.value {
            let declarations = Self::collect_declarations(node);

            for selector in selectors {
                styles.push((selector.clone(), declarations.clone()));
            }
        }

        for child in node_ref.children() {
            Self::walk(child, styles);
        }
    }

    fn collect_declarations(
        rule_node: &Rc<std::cell::RefCell<TreeNode<CssNodeType>>>,
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
