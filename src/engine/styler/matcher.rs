use crate::engine::css::cssom::{CssNodeType, CssValue};
use crate::engine::html::tokenizer::Attribute;
use crate::engine::tree::{Tree, TreeNode};

fn selector_matches(selector: &str, tag: &str, attrs: &[Attribute]) -> bool {
    if selector == tag {
        return true;
    }

    if let Some(class) = selector.strip_prefix('.') {
        return attrs
            .iter()
            .any(|a| a.name == "class" && a.value.split_whitespace().any(|c| c == class));
    }

    if let Some(id) = selector.strip_prefix('#') {
        return attrs.iter().any(|a| a.name == "id" && a.value == id);
    }

    false
}
