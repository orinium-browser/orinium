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

/// One link of an [`ElementChain`].
#[derive(Debug)]
struct ChainLink {
    info: ElementInfo,
    /// Ancestors (parent, grandparent, …), shared with the parent's chain.
    next: Option<Arc<ChainLink>>,
}

/// 右（自分）→ 左（祖先）
///
/// Cloning is O(1) and prepending an element is O(1): descendant chains share
/// their ancestor links through `Arc`.
#[derive(Debug, Clone, Default)]
pub struct ElementChain {
    head: Option<Arc<ChainLink>>,
}

impl ElementChain {
    /// Returns this chain extended with `info` as the innermost element.
    ///
    /// Passing `None` yields an equivalent chain without allocating.
    pub fn prepend(&self, info: Option<ElementInfo>) -> Self {
        match info {
            Some(info) => Self {
                head: Some(Arc::new(ChainLink {
                    info,
                    next: self.head.clone(),
                })),
            },
            None => self.clone(),
        }
    }

    /// The innermost (current) element.
    pub fn first(&self) -> Option<&ElementInfo> {
        self.head.as_deref().map(|link| &link.info)
    }

    pub fn is_empty(&self) -> bool {
        self.head.is_none()
    }

    /// Builds a chain from elements ordered innermost-first.
    pub fn from_vec(elements: Vec<ElementInfo>) -> Self {
        let mut chain = Self::default();
        for info in elements.into_iter().rev() {
            chain = chain.prepend(Some(info));
        }
        chain
    }
}

#[derive(Clone, Copy)]
struct MatchCursor<'a> {
    link: &'a ChainLink,
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

    fn matches_pseudo_classes(&self, cursor: MatchCursor<'_>, is_root: bool) -> bool {
        let element = ComplexSelector::element_at(cursor);
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
                if pseudo.eq_ignore_ascii_case("root") {
                    is_root
                } else if pseudo.eq_ignore_ascii_case("link")
                    || pseudo.eq_ignore_ascii_case("any-link")
                {
                    matches!(element.tag_name.as_str(), "a" | "area") && has_attribute("href")
                } else if pseudo.eq_ignore_ascii_case("disabled") {
                    is_form_control && has_attribute("disabled")
                } else if pseudo.eq_ignore_ascii_case("enabled") {
                    is_form_control && !has_attribute("disabled")
                } else if pseudo.eq_ignore_ascii_case("checked") {
                    (element.tag_name == "input" && has_attribute("checked"))
                        || (element.tag_name == "option" && has_attribute("selected"))
                } else if pseudo.eq_ignore_ascii_case("required") {
                    is_form_control && has_attribute("required")
                } else if pseudo.eq_ignore_ascii_case("optional") {
                    is_form_control && !has_attribute("required")
                } else if pseudo.eq_ignore_ascii_case("first-child") {
                    element.element_index == 1
                } else if pseudo.eq_ignore_ascii_case("last-child") {
                    element.element_index == element.element_count
                } else if pseudo.eq_ignore_ascii_case("only-child") {
                    element.element_count == 1
                } else if pseudo.eq_ignore_ascii_case("first-of-type") {
                    element.type_index == 1
                } else if pseudo.eq_ignore_ascii_case("last-of-type") {
                    element.type_index == element.type_count
                } else if pseudo.eq_ignore_ascii_case("only-of-type") {
                    element.type_count == 1
                } else {
                    false
                }
            }
            PseudoClass::SelectorList { name, selectors } => {
                if selectors.is_empty() {
                    return false;
                }
                let any_matches = selectors
                    .iter()
                    .any(|selector| selector.matches_from(cursor, 0));
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

    fn matches_at(&self, cursor: MatchCursor<'_>) -> bool {
        let element = ComplexSelector::element_at(cursor);
        let is_root = cursor.sibling_index.is_none() && cursor.link.next.is_none();
        if !self.matches_base(element) || !self.matches_pseudo_classes(cursor, is_root) {
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
    pub fn matches(&self, chain: &ElementChain) -> bool {
        if self.parts.is_empty() {
            return false;
        }
        let Some(link) = chain.head.as_deref() else {
            return false;
        };
        self.matches_from(
            MatchCursor {
                link,
                sibling_index: None,
            },
            0,
        )
    }

    fn element_at(cursor: MatchCursor<'_>) -> &ElementInfo {
        match cursor.sibling_index {
            Some(index) => &cursor.link.info.previous_siblings[index],
            None => &cursor.link.info,
        }
    }

    fn previous_sibling_cursor(cursor: MatchCursor<'_>) -> Option<MatchCursor<'_>> {
        let position = cursor
            .sibling_index
            .unwrap_or(cursor.link.info.previous_siblings.len());
        position.checked_sub(1).map(|sibling_index| MatchCursor {
            link: cursor.link,
            sibling_index: Some(sibling_index),
        })
    }

    fn matches_from(&self, cursor: MatchCursor<'_>, selector_index: usize) -> bool {
        let part = &self.parts[selector_index];

        if !part.selector.matches_at(cursor) {
            return false;
        }

        // セレクタが尽きた → 完全一致
        if selector_index + 1 == self.parts.len() {
            return true;
        }

        match part.combinator {
            Some(Combinator::Descendant) => {
                let mut ancestor = cursor.link.next.as_deref();
                while let Some(link) = ancestor {
                    if self.matches_from(
                        MatchCursor {
                            link,
                            sibling_index: None,
                        },
                        selector_index + 1,
                    ) {
                        return true;
                    }
                    ancestor = link.next.as_deref();
                }
                false
            }
            Some(Combinator::Child) => cursor.link.next.as_deref().is_some_and(|parent| {
                self.matches_from(
                    MatchCursor {
                        link: parent,
                        sibling_index: None,
                    },
                    selector_index + 1,
                )
            }),
            Some(Combinator::NextSibling) => Self::previous_sibling_cursor(cursor)
                .is_some_and(|previous| self.matches_from(previous, selector_index + 1)),
            Some(Combinator::SubsequentSibling) => {
                let mut previous = Self::previous_sibling_cursor(cursor);
                while let Some(candidate) = previous {
                    if self.matches_from(candidate, selector_index + 1) {
                        return true;
                    }
                    previous = Self::previous_sibling_cursor(candidate);
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

    fn chain(elements: impl IntoIterator<Item = ElementInfo>) -> ElementChain {
        ElementChain::from_vec(elements.into_iter().collect())
    }

    #[test]
    fn exact_attribute_selector_requires_matching_value() {
        let selector = input_selector(Some("hidden"));

        assert!(selector.matches(&chain([input(&[("type", "hidden")])])));
        assert!(!selector.matches(&chain([input(&[])])));
        assert!(!selector.matches(&chain([input(&[("type", "text")])])));
    }

    #[test]
    fn presence_attribute_selector_requires_attribute() {
        let selector = input_selector(None);

        assert!(selector.matches(&chain([input(&[("type", "text")])])));
        assert!(!selector.matches(&chain([input(&[])])));
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

        assert!(selector.matches(&chain([paragraph.clone(), main.clone()])));
        assert!(!selector.matches(&chain([paragraph, section, main])));
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

        assert!(selector.matches(&chain([html.clone()])));
        assert!(!selector.matches(&chain([body, html])));
    }

    #[test]
    fn link_pseudo_class_requires_an_href() {
        let selector = parse_selector("a:link");

        assert!(selector.matches(&chain([ElementInfo {
            tag_name: "a".into(),
            id: None,
            classes: Vec::new(),
            attributes: vec![("href".into(), "/next".into())],
            ..ElementInfo::default()
        }])));
        assert!(!selector.matches(&chain([ElementInfo {
            tag_name: "a".into(),
            id: None,
            classes: Vec::new(),
            attributes: Vec::new(),
            ..ElementInfo::default()
        }])));
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

        assert!(parse_selector("aside + p").matches(&chain([paragraph.clone()])));
        assert!(!parse_selector("h2 + p").matches(&chain([paragraph.clone()])));
        assert!(parse_selector("h2 ~ p").matches(&chain([paragraph.clone()])));
        assert!(!parse_selector("nav ~ p").matches(&chain([paragraph])));
    }

    #[test]
    fn structural_pseudo_classes_use_element_and_type_positions() {
        let second_paragraph = element("p", &[], 3, 5, 2, 3);

        assert!(parse_selector("p:nth-child(2n+1)").matches(&chain([second_paragraph.clone()])));
        assert!(parse_selector("p:nth-of-type(even)").matches(&chain([second_paragraph.clone()])));
        assert!(parse_selector("p:nth-last-child(3)").matches(&chain([second_paragraph.clone()])));
        assert!(
            parse_selector("p:nth-last-of-type(2)").matches(&chain([second_paragraph.clone()]))
        );
        assert!(parse_selector("p:nth-child(-n+3)").matches(&chain([second_paragraph.clone()])));
        assert!(!parse_selector("p:first-child").matches(&chain([second_paragraph.clone()])));
        assert!(!parse_selector("p:last-of-type").matches(&chain([second_paragraph])));
    }

    #[test]
    fn selector_list_pseudo_classes_and_multiple_pseudos_match() {
        let visible_first = element("li", &["item"], 1, 3, 1, 3);
        let hidden_first = element("li", &["item", "hidden"], 1, 3, 1, 3);
        let selector = parse_selector("li.item:first-child:not(.hidden)");

        assert!(selector.matches(&chain([visible_first.clone()])));
        assert!(!selector.matches(&chain([hidden_first])));
        assert!(parse_selector(":is(article, li.item)").matches(&chain([visible_first.clone()])));
        assert!(parse_selector(":where(.item, .card)").matches(&chain([visible_first])));
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
