use crate::engine::css::cssom::{CssNodeType, CssValue};
use crate::engine::html::tokenizer::Attribute;
use crate::engine::tree::{Tree, TreeNode};

fn selector_matches(selector: &str, tag: &str, attrs: &Vec<Attribute>) -> bool {
    if selector == tag {
        return true;
    }

    if selector.starts_with('.') {
        let class = &selector[1..];
        return attrs
            .iter()
            .any(|a| a.name == "class" && a.value.split_whitespace().any(|c| c == class));
    }

    if selector.starts_with('#') {
        let id = &selector[1..];
        return attrs.iter().any(|a| a.name == "id" && a.value == id);
    }

    false
}
