//! CSSセレクターマッチング処理。DOM要素とセレクターの照合を行う。

use super::parser::{Combinator, ComplexSelector, Selector};

#[derive(Debug, Clone)]
pub struct ElementInfo {
    pub tag_name: String,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attributes: Vec<(String, String)>,
}

/// 右（自分）→ 左（祖先）
pub type ElementChain = Vec<ElementInfo>;

impl Selector {
    /// Matches this simple selector against one element.
    pub fn matches(
        &self,
        tag_name: &str,
        id: Option<&str>,
        class_list: &[String],
        attributes: &[(String, String)],
    ) -> bool {
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

        for expected in &self.attributes {
            let actual = attributes
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(&expected.name));
            match (&expected.value, actual) {
                (None, Some(_)) => {}
                (Some(expected_value), Some((_, actual_value)))
                    if actual_value == expected_value => {}
                _ => return false,
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

        if !part.selector.matches(
            &element.tag_name,
            element.id.as_deref(),
            &element.classes,
            &element.attributes,
        ) {
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
            Some(Combinator::Child) => {
                chain_index + 1 < chain.len()
                    && self.match_from(chain, chain_index + 1, selector_index + 1)
            }
            None => false,
        }
    }

    pub fn specificity(&self) -> (u32, u32, u32) {
        let mut a = 0; // id
        let mut b = 0; // class / attr / pseudo-class
        let mut c = 0; // tag / pseudo-element

        for part in &self.parts {
            let sel = &part.selector;

            if sel.id.is_some() {
                a += 1;
            }
            b += (sel.classes.len() + sel.attributes.len()) as u32;
            if sel.tag.is_some() {
                c += 1;
            }
        }

        (a, b, c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::css::parser::{AttributeSelector, Parser, SelectorPart};

    fn input_selector(value: Option<&str>) -> ComplexSelector {
        ComplexSelector {
            parts: vec![SelectorPart {
                selector: Selector {
                    tag: Some("input".into()),
                    id: None,
                    classes: Vec::new(),
                    attributes: vec![AttributeSelector {
                        name: "type".into(),
                        value: value.map(Into::into),
                    }],
                    pseudo_class: None,
                    pseudo_element: None,
                },
                combinator: None,
            }],
        }
    }

    fn input(attributes: &[(&str, &str)]) -> ElementInfo {
        ElementInfo {
            tag_name: "input".into(),
            id: None,
            classes: Vec::new(),
            attributes: attributes
                .iter()
                .map(|(name, value)| ((*name).into(), (*value).into()))
                .collect(),
        }
    }

    #[test]
    fn exact_attribute_selector_requires_matching_value() {
        let selector = input_selector(Some("hidden"));

        assert!(selector.matches(&[input(&[("type", "hidden")])]));
        assert!(!selector.matches(&[input(&[])]));
        assert!(!selector.matches(&[input(&[("type", "text")])]));
    }

    #[test]
    fn presence_attribute_selector_requires_attribute() {
        let selector = input_selector(None);

        assert!(selector.matches(&[input(&[("type", "text")])]));
        assert!(!selector.matches(&[input(&[])]));
    }

    #[test]
    fn child_combinator_requires_direct_parent() {
        let stylesheet = Parser::new("main > p { color: red; }").parse().unwrap();
        let selector = match stylesheet.children().first().unwrap().node() {
            crate::engine::css::parser::CssNodeType::Rule { selectors } => &selectors[0],
            _ => panic!("expected CSS rule"),
        };
        let paragraph = ElementInfo {
            tag_name: "p".into(),
            id: None,
            classes: Vec::new(),
            attributes: Vec::new(),
        };
        let section = ElementInfo {
            tag_name: "section".into(),
            id: None,
            classes: Vec::new(),
            attributes: Vec::new(),
        };
        let main = ElementInfo {
            tag_name: "main".into(),
            id: None,
            classes: Vec::new(),
            attributes: Vec::new(),
        };

        assert!(selector.matches(&[paragraph.clone(), main.clone()]));
        assert!(!selector.matches(&[paragraph, section, main]));
    }
}
