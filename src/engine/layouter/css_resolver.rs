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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::css::cssom::{matcher::ElementInfo, *};
    use crate::engine::css::values::Color;
    use crate::engine::tree::{Tree, TreeNode};

    #[test]
    fn css_resolver_collects_rules_and_declarations() {
        // Stylesheet
        let tree = Tree::new(CssNodeType::Stylesheet);
        let root = &tree.root;

        // html { padding: 0 40px }
        let html_rule = TreeNode::new(CssNodeType::Rule {
            selectors: vec![ComplexSelector {
                parts: vec![SelectorPart {
                    selector: Selector {
                        tag: Some("html".into()),
                        id: None,
                        classes: vec![],
                        pseudo_class: None,
                        pseudo_element: None,
                    },
                    combinator: None,
                }],
            }],
        });
        TreeNode::add_child(
            &html_rule,
            TreeNode::new(CssNodeType::Declaration {
                name: "padding".into(),
                value: CssValue::Keyword("0 40px".into()),
            }),
        );
        TreeNode::add_child(root, html_rule);

        // body { margin: 0; padding: 0; color: ... }
        let body_rule = TreeNode::new(CssNodeType::Rule {
            selectors: vec![ComplexSelector {
                parts: vec![SelectorPart {
                    selector: Selector {
                        tag: Some("body".into()),
                        id: None,
                        classes: vec![],
                        pseudo_class: None,
                        pseudo_element: None,
                    },
                    combinator: None,
                }],
            }],
        });

        TreeNode::add_child(
            &body_rule,
            TreeNode::new(CssNodeType::Declaration {
                name: "margin".into(),
                value: CssValue::Keyword("0".into()),
            }),
        );
        TreeNode::add_child(
            &body_rule,
            TreeNode::new(CssNodeType::Declaration {
                name: "padding".into(),
                value: CssValue::Keyword("0".into()),
            }),
        );
        TreeNode::add_child(
            &body_rule,
            TreeNode::new(CssNodeType::Declaration {
                name: "color".into(),
                value: CssValue::Color(Color::Rgba(240, 240, 240, 1.0)),
            }),
        );

        TreeNode::add_child(root, body_rule);

        // 実行
        let styles = CssResolver::resolve(&tree);

        // Rule 数
        assert_eq!(styles.len(), 2);

        // html
        let (selector, decls) = &styles[0];
        assert_eq!(selector.parts[0].selector.tag.as_deref(), Some("html"));
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].0, "padding");

        // body
        let (selector, decls) = &styles[1];
        assert_eq!(selector.parts[0].selector.tag.as_deref(), Some("body"));
        assert_eq!(decls.len(), 3);
        assert_eq!(decls[0].0, "margin");
        assert_eq!(decls[1].0, "padding");
        assert_eq!(decls[2].0, "color");
    }

    #[test]
    fn matches_descendant_chain_main_nav_ul_li_a() {
        // .main-nav ul li a
        let selector = ComplexSelector {
            parts: vec![
                SelectorPart {
                    selector: Selector {
                        tag: Some("a".into()),
                        id: None,
                        classes: vec![],
                        pseudo_class: None,
                        pseudo_element: None,
                    },
                    combinator: Some(Combinator::Descendant),
                },
                SelectorPart {
                    selector: Selector {
                        tag: Some("li".into()),
                        id: None,
                        classes: vec![],
                        pseudo_class: None,
                        pseudo_element: None,
                    },
                    combinator: Some(Combinator::Descendant),
                },
                SelectorPart {
                    selector: Selector {
                        tag: Some("ul".into()),
                        id: None,
                        classes: vec![],
                        pseudo_class: None,
                        pseudo_element: None,
                    },
                    combinator: Some(Combinator::Descendant),
                },
                SelectorPart {
                    selector: Selector {
                        tag: None,
                        id: None,
                        classes: vec!["main-nav".into()],
                        pseudo_class: None,
                        pseudo_element: None,
                    },
                    combinator: None,
                },
            ],
        };

        // DOM chain: a <- li <- ul <- div.main-nav
        let chain = vec![
            ElementInfo {
                tag_name: "a".into(),
                id: None,
                classes: vec![],
            },
            ElementInfo {
                tag_name: "li".into(),
                id: None,
                classes: vec![],
            },
            ElementInfo {
                tag_name: "ul".into(),
                id: None,
                classes: vec![],
            },
            ElementInfo {
                tag_name: "div".into(),
                id: None,
                classes: vec!["main-nav".into()],
            },
        ];

        assert!(selector.matches(&chain));
    }

    #[test]
    fn does_not_match_when_class_is_missing() {
        // .main-nav ul li a
        let selector = ComplexSelector {
            parts: vec![
                SelectorPart {
                    selector: Selector {
                        tag: Some("a".into()),
                        id: None,
                        classes: vec![],
                        pseudo_class: None,
                        pseudo_element: None,
                    },
                    combinator: Some(Combinator::Descendant),
                },
                SelectorPart {
                    selector: Selector {
                        tag: Some("li".into()),
                        id: None,
                        classes: vec![],
                        pseudo_class: None,
                        pseudo_element: None,
                    },
                    combinator: Some(Combinator::Descendant),
                },
                SelectorPart {
                    selector: Selector {
                        tag: Some("ul".into()),
                        id: None,
                        classes: vec![],
                        pseudo_class: None,
                        pseudo_element: None,
                    },
                    combinator: Some(Combinator::Descendant),
                },
                SelectorPart {
                    selector: Selector {
                        tag: None,
                        id: None,
                        classes: vec!["main-nav".into()],
                        pseudo_class: None,
                        pseudo_element: None,
                    },
                    combinator: None,
                },
            ],
        };

        // class が違う
        let chain = vec![
            ElementInfo {
                tag_name: "a".into(),
                id: None,
                classes: vec![],
            },
            ElementInfo {
                tag_name: "li".into(),
                id: None,
                classes: vec![],
            },
            ElementInfo {
                tag_name: "ul".into(),
                id: None,
                classes: vec![],
            },
            ElementInfo {
                tag_name: "div".into(),
                id: None,
                classes: vec!["header".into()],
            },
        ];

        assert!(!selector.matches(&chain));
    }
}
