use super::{Combinator, ComplexSelector, Selector};

#[derive(Debug, Clone)]
pub struct ElementInfo {
    pub tag_name: String,
    pub id: Option<String>,
    pub classes: Vec<String>,
}

/// 右（自分）→ 左（祖先）
pub type ElementChain = Vec<ElementInfo>;

impl Selector {
    /// Simple selector matcher (tag / class / id)
    pub fn matches(&self, tag_name: &str, id: Option<&str>, class_list: &[String]) -> bool {
        // tag
        if let Some(tag) = &self.tag
            && tag != tag_name
        {
            return false;
        }

        // id
        if let Some(expected_id) = &self.id {
            match id {
                Some(actual_id) if actual_id == expected_id => {}
                _ => return false,
            }
        }

        // class
        for class in &self.classes {
            if !class_list.iter().any(|c| c == class) {
                return false;
            }
        }

        if let Some(_pseudo) = &self.pseudo_class {
            // TODO
            return false;
        }

        if let Some(_pseudo) = &self.pseudo_element {
            // TODO
            return false;
        }

        true
    }
}

impl ComplexSelector {
    pub fn matches(&self, chain: &[ElementInfo]) -> bool {
        if chain.is_empty() || self.parts.is_empty() {
            return false;
        }
        self.match_from(chain, 0, 0)
    }

    fn match_from(&self, chain: &[ElementInfo], chain_index: usize, selector_index: usize) -> bool {
        let element = &chain[chain_index];
        let part = &self.parts[selector_index];

        if !part
            .selector
            .matches(&element.tag_name, element.id.as_deref(), &element.classes)
        {
            return false;
        }

        // セレクタが尽きた → 完全一致
        if selector_index + 1 == self.parts.len() {
            return true;
        }

        match part.combinator {
            Some(Combinator::Descendant) => {
                for next in (chain_index + 1)..chain.len() {
                    if self.match_from(chain, next, selector_index + 1) {
                        return true;
                    }
                }
                false
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::css::cssom::SelectorPart;

    fn el(tag: &str, id: Option<&str>, classes: &[&str]) -> ElementInfo {
        ElementInfo {
            tag_name: tag.to_string(),
            id: id.map(|s| s.to_string()),
            classes: classes.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn match_single_selector() {
        let selector = ComplexSelector {
            parts: vec![SelectorPart {
                selector: Selector {
                    tag: Some("div".into()),
                    id: None,
                    classes: vec![],
                    pseudo_class: None,
                    pseudo_element: None,
                },
                combinator: None,
            }],
        };

        let chain = vec![el("div", None, &[])];

        assert!(selector.matches(&chain));
    }

    #[test]
    fn match_descendant_selector_simple() {
        // .main-nav ul
        let selector = ComplexSelector {
            parts: vec![
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

        let chain = vec![
            el("ul", None, &[]),
            el("nav", None, &["main-nav"]),
            el("body", None, &[]),
        ];

        assert!(selector.matches(&chain));
    }

    #[test]
    fn descendant_selector_fails_if_no_ancestor_match() {
        let selector = ComplexSelector {
            parts: vec![
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

        let chain = vec![el("ul", None, &[]), el("div", None, &["content"])];

        assert!(!selector.matches(&chain));
    }

    #[test]
    fn deep_descendant_match() {
        // div .a span
        let selector = ComplexSelector {
            parts: vec![
                SelectorPart {
                    selector: Selector {
                        tag: Some("span".into()),
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
                        classes: vec!["a".into()],
                        pseudo_class: None,
                        pseudo_element: None,
                    },
                    combinator: Some(Combinator::Descendant),
                },
                SelectorPart {
                    selector: Selector {
                        tag: Some("div".into()),
                        id: None,
                        classes: vec![],
                        pseudo_class: None,
                        pseudo_element: None,
                    },
                    combinator: None,
                },
            ],
        };

        let chain = vec![
            el("span", None, &[]),
            el("p", None, &[]),
            el("section", None, &["a"]),
            el("div", None, &[]),
        ];

        assert!(selector.matches(&chain));
    }

    #[test]
    fn class_and_tag_both_required() {
        let selector = ComplexSelector {
            parts: vec![SelectorPart {
                selector: Selector {
                    tag: Some("div".into()),
                    id: None,
                    classes: vec!["container".into()],
                    pseudo_class: None,
                    pseudo_element: None,
                },
                combinator: None,
            }],
        };

        let chain_ok = vec![el("div", None, &["container"])];
        let chain_ng = vec![el("div", None, &[])];

        assert!(selector.matches(&chain_ok));
        assert!(!selector.matches(&chain_ng));
    }
}
