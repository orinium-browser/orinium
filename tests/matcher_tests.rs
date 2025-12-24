use orinium_browser::engine::html::HtmlNodeType;
use orinium_browser::engine::html::tokenizer::Attribute;
use orinium_browser::engine::styler::matcher::selector_matches_on_node;
use orinium_browser::engine::tree::TreeNode;
use std::rc::Rc;

#[test]
fn simple_selectors_match() {
    let node = TreeNode::new(HtmlNodeType::Element {
        tag_name: "div".to_string(),
        attributes: vec![
            Attribute {
                name: "class".to_string(),
                value: "foo bar".to_string(),
            },
            Attribute {
                name: "id".to_string(),
                value: "main".to_string(),
            },
        ],
    });

    assert!(selector_matches_on_node("div", &node));
    assert!(selector_matches_on_node(".foo", &node));
    assert!(selector_matches_on_node(".bar", &node));
    assert!(selector_matches_on_node("#main", &node));
    assert!(selector_matches_on_node("div.foo#main", &node));
    assert!(!selector_matches_on_node(".baz", &node));
}

#[test]
fn descendant_selector_matches_with_ancestor() {
    // ancestor: <div>
    let ancestor = TreeNode::new(HtmlNodeType::Element {
        tag_name: "div".to_string(),
        attributes: vec![],
    });

    // child: <span class="foo">
    let child = TreeNode::new(HtmlNodeType::Element {
        tag_name: "span".to_string(),
        attributes: vec![Attribute {
            name: "class".to_string(),
            value: "foo".to_string(),
        }],
    });

    TreeNode::add_child(&ancestor, Rc::clone(&child));

    assert!(selector_matches_on_node(".foo", &child));
    assert!(selector_matches_on_node("div .foo", &child));
    assert!(selector_matches_on_node("div span", &child));
    // Non-matching ancestor
    assert!(!selector_matches_on_node("section .foo", &child));
}
