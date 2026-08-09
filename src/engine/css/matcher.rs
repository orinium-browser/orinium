//! CSSセレクターマッチング処理。DOM要素とセレクターの照合を行う。

use std::sync::Arc;

use super::parser::{Combinator, ComplexSelector, PseudoClass, Selector};

#[derive(Debug, Clone, Default)]
pub struct ElementInfo {
    pub tag_name: String,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attributes: Vec<(String, String)>,
    /// One-based index among element siblings.
    pub element_index: usize,
    /// Number of element siblings including this element.
    pub element_count: usize,
    /// One-based index among siblings with the same tag name.
    pub type_index: usize,
    /// Number of siblings with the same tag name.
    pub type_count: usize,
    /// Element siblings preceding this element in document order.
    pub previous_siblings: Arc<[ElementInfo]>,
}

/// 右（自分）→ 左（祖先）
pub type ElementChain = Vec<ElementInfo>;

#[derive(Clone, Copy)]
struct MatchCursor {
    chain_index: usize,
    sibling_index: Option<usize>,
}

fn matches_an_plus_b(index: usize, a: i32, b: i32) -> bool {
    let index = index as i32;
    if a == 0 {
        return index == b;
    }
    let delta = index - b;
    delta % a == 0 && delta / a >= 0
}

impl Selector {
    /// Matches the non-structural portion of this selector against one element.
    fn matches_base(&self, element: &ElementInfo) -> bool {
        // tag
        if let Some(tag) = &self.tag
            && tag != &element.tag_name
        {
            return false;
        }

        // id
        if let Some(expected_id) = &self.id {
            match element.id.as_deref() {
                Some(actual_id) if actual_id == expected_id => {}
                _ => return false,
            }
        }

        // class
        for class in &self.classes {
            if !element.classes.iter().any(|c| c == class) {
                return false;
            }
        }

        for expected in &self.attributes {
            let actual = element
                .attributes
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(&expected.name));
            match (&expected.value, actual) {
                (None, Some(_)) => {}
                (Some(expected_value), Some((_, actual_value)))
                    if actual_value == expected_value => {}
                _ => return false,
            }
        }

        true
    }

    fn matches_pseudo_classes(
        &self,
        chain: &[ElementInfo],
        cursor: MatchCursor,
        is_root: bool,
    ) -> bool {
        let element = ComplexSelector::element_at(chain, cursor);
        self.pseudo_classes.iter().all(|pseudo| match pseudo {
            PseudoClass::Simple(pseudo) => {
                let has_attribute = |name: &str| {
                    element
                        .attributes
                        .iter()
                        .any(|(attribute, _)| attribute.eq_ignore_ascii_case(name))
                };
                let is_form_control = matches!(
                    element.tag_name.as_str(),
                    "button" | "fieldset" | "input" | "optgroup" | "option" | "select" | "textarea"
                );
                match pseudo.to_ascii_lowercase().as_str() {
                    "root" => is_root,
                    "link" | "any-link" => {
                        matches!(element.tag_name.as_str(), "a" | "area") && has_attribute("href")
                    }
                    "disabled" => is_form_control && has_attribute("disabled"),
                    "enabled" => is_form_control && !has_attribute("disabled"),
                    "checked" => {
                        (element.tag_name == "input" && has_attribute("checked"))
                            || (element.tag_name == "option" && has_attribute("selected"))
                    }
                    "required" => is_form_control && has_attribute("required"),
                    "optional" => is_form_control && !has_attribute("required"),
                    "first-child" => element.element_index == 1,
                    "last-child" => element.element_index == element.element_count,
                    "only-child" => element.element_count == 1,
                    "first-of-type" => element.type_index == 1,
                    "last-of-type" => element.type_index == element.type_count,
                    "only-of-type" => element.type_count == 1,
                    // These require browsing history or live interaction state.
                    "visited" | "active" | "focus" | "focus-visible" | "focus-within" | "hover" => {
                        false
                    }
                    _ => false,
                }
            }
            PseudoClass::SelectorList { name, selectors } => {
                if selectors.is_empty() {
                    return false;
                }
                let any_matches = selectors
                    .iter()
                    .any(|selector| selector.matches_from(chain, cursor, 0));
                match name.as_str() {
                    "is" | "where" => any_matches,
                    "not" => !any_matches,
                    _ => false,
                }
            }
            PseudoClass::Nth { name, a, b } => {
                let index = match name.as_str() {
                    "nth-child" => element.element_index,
                    "nth-last-child" => element
                        .element_count
                        .saturating_add(1)
                        .saturating_sub(element.element_index),
                    "nth-of-type" => element.type_index,
                    "nth-last-of-type" => element
                        .type_count
                        .saturating_add(1)
                        .saturating_sub(element.type_index),
                    _ => return false,
                };
                matches_an_plus_b(index, *a, *b)
            }
        })
    }

    fn matches_at(&self, chain: &[ElementInfo], cursor: MatchCursor) -> bool {
        let element = ComplexSelector::element_at(chain, cursor);
        let is_root = cursor.sibling_index.is_none() && cursor.chain_index + 1 == chain.len();
        if !self.matches_base(element) || !self.matches_pseudo_classes(chain, cursor, is_root) {
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
        self.matches_from(
            chain,
            MatchCursor {
                chain_index: 0,
                sibling_index: None,
            },
            0,
        )
    }

    fn element_at(chain: &[ElementInfo], cursor: MatchCursor) -> &ElementInfo {
        match cursor.sibling_index {
            Some(index) => &chain[cursor.chain_index].previous_siblings[index],
            None => &chain[cursor.chain_index],
        }
    }

    fn previous_sibling_cursor(chain: &[ElementInfo], cursor: MatchCursor) -> Option<MatchCursor> {
        let position = cursor
            .sibling_index
            .unwrap_or(chain[cursor.chain_index].previous_siblings.len());
        position.checked_sub(1).map(|sibling_index| MatchCursor {
            chain_index: cursor.chain_index,
            sibling_index: Some(sibling_index),
        })
    }

    fn matches_from(
        &self,
        chain: &[ElementInfo],
        cursor: MatchCursor,
        selector_index: usize,
    ) -> bool {
        let part = &self.parts[selector_index];

        if !part.selector.matches_at(chain, cursor) {
            return false;
        }

        // セレクタが尽きた → 完全一致
        if selector_index + 1 == self.parts.len() {
            return true;
        }

        match part.combinator {
            Some(Combinator::Descendant) => {
                for next in (cursor.chain_index + 1)..chain.len() {
                    if self.matches_from(
                        chain,
                        MatchCursor {
                            chain_index: next,
                            sibling_index: None,
                        },
                        selector_index + 1,
                    ) {
                        return true;
                    }
                }
                false
            }
            Some(Combinator::Child) => {
                cursor.chain_index + 1 < chain.len()
                    && self.matches_from(
                        chain,
                        MatchCursor {
                            chain_index: cursor.chain_index + 1,
                            sibling_index: None,
                        },
                        selector_index + 1,
                    )
            }
            Some(Combinator::NextSibling) => Self::previous_sibling_cursor(chain, cursor)
                .is_some_and(|previous| self.matches_from(chain, previous, selector_index + 1)),
            Some(Combinator::SubsequentSibling) => {
                let mut previous = Self::previous_sibling_cursor(chain, cursor);
                while let Some(candidate) = previous {
                    if self.matches_from(chain, candidate, selector_index + 1) {
                        return true;
                    }
                    previous = Self::previous_sibling_cursor(chain, candidate);
                }
                false
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
            for pseudo in &sel.pseudo_classes {
                match pseudo {
                    PseudoClass::Simple(_) | PseudoClass::Nth { .. } => b += 1,
                    PseudoClass::SelectorList { name, selectors } if name == "where" => {}
                    PseudoClass::SelectorList { selectors, .. } => {
                        let nested = selectors
                            .iter()
                            .map(ComplexSelector::specificity)
                            .max()
                            .unwrap_or_default();
                        a += nested.0;
                        b += nested.1;
                        c += nested.2;
                    }
                }
            }
            if sel.tag.is_some() {
                c += 1;
            }
            c += u32::from(sel.pseudo_element.is_some());
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
                    pseudo_classes: Vec::new(),
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
            ..ElementInfo::default()
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
            ..ElementInfo::default()
        };
        let section = ElementInfo {
            tag_name: "section".into(),
            id: None,
            classes: Vec::new(),
            attributes: Vec::new(),
            ..ElementInfo::default()
        };
        let main = ElementInfo {
            tag_name: "main".into(),
            id: None,
            classes: Vec::new(),
            attributes: Vec::new(),
            ..ElementInfo::default()
        };

        assert!(selector.matches(&[paragraph.clone(), main.clone()]));
        assert!(!selector.matches(&[paragraph, section, main]));
    }

    fn parse_selector(source: &str) -> ComplexSelector {
        let stylesheet = Parser::new(&format!("{source} {{ color: red; }}"))
            .parse()
            .unwrap();
        match stylesheet.children().first().unwrap().node() {
            crate::engine::css::parser::CssNodeType::Rule { selectors } => selectors[0].clone(),
            _ => panic!("expected CSS rule"),
        }
    }

    #[test]
    fn root_pseudo_class_only_matches_the_root_element() {
        let selector = parse_selector(":root");
        let html = ElementInfo {
            tag_name: "html".into(),
            id: None,
            classes: Vec::new(),
            attributes: Vec::new(),
            ..ElementInfo::default()
        };
        let body = ElementInfo {
            tag_name: "body".into(),
            id: None,
            classes: Vec::new(),
            attributes: Vec::new(),
            ..ElementInfo::default()
        };

        assert!(selector.matches(std::slice::from_ref(&html)));
        assert!(!selector.matches(&[body, html]));
    }

    #[test]
    fn link_pseudo_class_requires_an_href() {
        let selector = parse_selector("a:link");

        assert!(selector.matches(&[ElementInfo {
            tag_name: "a".into(),
            id: None,
            classes: Vec::new(),
            attributes: vec![("href".into(), "/next".into())],
            ..ElementInfo::default()
        }]));
        assert!(!selector.matches(&[ElementInfo {
            tag_name: "a".into(),
            id: None,
            classes: Vec::new(),
            attributes: Vec::new(),
            ..ElementInfo::default()
        }]));
    }

    fn element(
        tag_name: &str,
        classes: &[&str],
        element_index: usize,
        element_count: usize,
        type_index: usize,
        type_count: usize,
    ) -> ElementInfo {
        ElementInfo {
            tag_name: tag_name.into(),
            classes: classes.iter().map(|class| (*class).into()).collect(),
            element_index,
            element_count,
            type_index,
            type_count,
            ..ElementInfo::default()
        }
    }

    #[test]
    fn sibling_combinators_match_preceding_elements() {
        let heading = element("h2", &[], 1, 3, 1, 1);
        let aside = element("aside", &[], 2, 3, 1, 1);
        let mut paragraph = element("p", &[], 3, 3, 1, 1);
        paragraph.previous_siblings = vec![heading, aside].into();

        assert!(parse_selector("aside + p").matches(std::slice::from_ref(&paragraph)));
        assert!(!parse_selector("h2 + p").matches(std::slice::from_ref(&paragraph)));
        assert!(parse_selector("h2 ~ p").matches(std::slice::from_ref(&paragraph)));
        assert!(!parse_selector("nav ~ p").matches(std::slice::from_ref(&paragraph)));
    }

    #[test]
    fn structural_pseudo_classes_use_element_and_type_positions() {
        let second_paragraph = element("p", &[], 3, 5, 2, 3);

        assert!(
            parse_selector("p:nth-child(2n+1)").matches(std::slice::from_ref(&second_paragraph))
        );
        assert!(
            parse_selector("p:nth-of-type(even)").matches(std::slice::from_ref(&second_paragraph))
        );
        assert!(
            parse_selector("p:nth-last-child(3)").matches(std::slice::from_ref(&second_paragraph))
        );
        assert!(
            parse_selector("p:nth-last-of-type(2)")
                .matches(std::slice::from_ref(&second_paragraph))
        );
        assert!(
            parse_selector("p:nth-child(-n+3)").matches(std::slice::from_ref(&second_paragraph))
        );
        assert!(!parse_selector("p:first-child").matches(std::slice::from_ref(&second_paragraph)));
        assert!(!parse_selector("p:last-of-type").matches(std::slice::from_ref(&second_paragraph)));
    }

    #[test]
    fn selector_list_pseudo_classes_and_multiple_pseudos_match() {
        let visible_first = element("li", &["item"], 1, 3, 1, 3);
        let hidden_first = element("li", &["item", "hidden"], 1, 3, 1, 3);
        let selector = parse_selector("li.item:first-child:not(.hidden)");

        assert!(selector.matches(std::slice::from_ref(&visible_first)));
        assert!(!selector.matches(std::slice::from_ref(&hidden_first)));
        assert!(
            parse_selector(":is(article, li.item)").matches(std::slice::from_ref(&visible_first))
        );
        assert!(
            parse_selector(":where(.item, .card)").matches(std::slice::from_ref(&visible_first))
        );
    }

    #[test]
    fn selector_list_pseudo_classes_follow_specificity_rules() {
        assert_eq!(
            parse_selector(":where(#main, .item)").specificity(),
            (0, 0, 0)
        );
        assert_eq!(parse_selector(":is(#main, .item)").specificity(), (1, 0, 0));
        assert_eq!(
            parse_selector("li:not(.hidden):first-child").specificity(),
            (0, 2, 1)
        );
    }
}
