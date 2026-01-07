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
    /// Simple selector matcher (tag / class / id-ready)
    pub fn matches(&self, tag_name: &str, id: Option<&str>, class_list: &[String]) -> bool {
        // tag
        if let Some(tag) = &self.tag {
            if tag != tag_name {
                return false;
            }
        }

        // class
        for class in &self.classes {
            if !class_list.iter().any(|c| c == class) {
                return false;
            }
        }

        // TODO: id
        let _ = id;

        true
    }
}

impl ComplexSelector {
    pub fn matches(&self, chain: &[ElementInfo]) -> bool {
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

        // セレクタが尽きた
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
