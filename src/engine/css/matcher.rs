use super::parser::{Combinator, ComplexSelector, Selector};

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

    pub fn specificity(&self) -> (u32, u32, u32) {
        let mut a = 0; // id
        let mut b = 0; // class / attr / pseudo-class
        let mut c = 0; // tag / pseudo-element

        for part in &self.parts {
            let sel = &part.selector;

            if sel.id.is_some() {
                a += 1;
            }
            b += sel.classes.len() as u32;
            if sel.tag.is_some() {
                c += 1;
            }
        }

        (a, b, c)
    }
}
