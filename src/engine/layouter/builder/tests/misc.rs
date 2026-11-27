use super::*;
use crate::engine::html::parser::ShadowRootMode;
use crate::engine::layouter::types::ColorScheme;
use crate::engine::tree::TreeNode;
use std::borrow::Cow;
use std::rc::Rc;

#[test]
fn normalize_whitespace_collapses_css_whitespace_only() {
    let normal = |s| normalize_whitespace(s, WhiteSpace::Normal);
    assert_eq!(normal("a  b\tc\nd"), "a b c d");
    assert_eq!(normal("a\u{a0}b"), "a\u{a0}b");
    assert_eq!(normal("\u{2007}\u{2007}"), "\u{2007}\u{2007}");
    assert_eq!(normal(" a\n\t b "), " a b ");
}

#[test]
fn normalize_whitespace_normalizes_segment_breaks() {
    let normal = |s| normalize_whitespace(s, WhiteSpace::Normal);
    let pre = |s| normalize_whitespace(s, WhiteSpace::Pre);
    assert_eq!(normal("a\r\nb"), "a b");
    assert_eq!(normal("a\rb"), "a b");
    assert_eq!(normal("a\u{c}b"), "a b");
    assert_eq!(pre("a\r\nb"), "a\nb");
    assert_eq!(pre("a\rb"), "a\nb");
    assert_eq!(pre("a\u{c}b"), "a\nb");
}

#[test]
fn normalize_whitespace_pre_line_keeps_trailing_newline() {
    let pre_line = |s| normalize_whitespace(s, WhiteSpace::PreLine);
    assert_eq!(pre_line("a\nb\n"), "a\nb\n");
    assert_eq!(pre_line("a\nb\n\n"), "a\nb\n\n");
    assert_eq!(pre_line("\n"), "\n");
    assert_eq!(pre_line("a  b\nc\n"), "a b\nc\n");
}

#[test]
fn normalize_whitespace_pre_wrap_preserves_newlines() {
    assert_eq!(
        normalize_whitespace("a\nb\n", WhiteSpace::PreWrap),
        "a\nb\n"
    );
    assert_eq!(
        normalize_whitespace("a  b\tc", WhiteSpace::BreakSpaces),
        "a  b\tc"
    );
    assert_eq!(normalize_whitespace("a\n\nb", WhiteSpace::Nowrap), "a b");
}

#[test]
fn normalize_whitespace_borrows_text_that_needs_no_rewrite() {
    // Already-collapsed text is returned without allocating.
    assert!(matches!(
        normalize_whitespace("plain text", WhiteSpace::Normal),
        Cow::Borrowed(_)
    ));
    assert!(matches!(
        normalize_whitespace("a b c", WhiteSpace::Nowrap),
        Cow::Borrowed(_)
    ));
    assert!(matches!(
        normalize_whitespace("a\nb\n", WhiteSpace::PreWrap),
        Cow::Borrowed(_)
    ));
    // A single leading/trailing space survives the original algorithm, so it
    // is not considered a rewrite either.
    assert!(matches!(
        normalize_whitespace(" a b ", WhiteSpace::Normal),
        Cow::Borrowed(_)
    ));
    assert!(matches!(
        normalize_whitespace("", WhiteSpace::Normal),
        Cow::Borrowed(_)
    ));
}

#[test]
fn normalize_whitespace_owns_text_that_needs_a_rewrite() {
    assert!(matches!(
        normalize_whitespace("a  b", WhiteSpace::Normal),
        Cow::Owned(_)
    ));
    assert!(matches!(
        normalize_whitespace("a\tb", WhiteSpace::Normal),
        Cow::Owned(_)
    ));
    assert!(matches!(
        normalize_whitespace("a\r\nb", WhiteSpace::Normal),
        Cow::Owned(_)
    ));
    assert!(matches!(
        normalize_whitespace("a\r\nb", WhiteSpace::Pre),
        Cow::Owned(_)
    ));
    assert!(matches!(
        normalize_whitespace("a\nb\n", WhiteSpace::PreLine),
        Cow::Owned(_)
    ));
}

#[test]
fn normalize_whitespace_fast_path_matches_the_full_pass() {
    let samples = [
        "",
        " ",
        "  ",
        "a",
        "a b",
        " a b ",
        "a  b",
        "a\tb",
        "a\nb",
        "a\r\nb",
        "a\rb",
        "a\x0cb",
        "a\n\nb",
        "a \t\n b",
        "\n",
        "\r\n",
        "a\u{a0}b",
        "a\u{2007}b",
        "a b\nc d",
    ];
    let modes = [
        WhiteSpace::Normal,
        WhiteSpace::Nowrap,
        WhiteSpace::Pre,
        WhiteSpace::PreWrap,
        WhiteSpace::PreLine,
        WhiteSpace::BreakSpaces,
    ];
    for &mode in &modes {
        for sample in samples {
            let optimized = normalize_whitespace(sample, mode);
            let reference = reference_normalize_whitespace(sample, mode);
            assert_eq!(
                optimized, reference,
                "mismatch for {sample:?} under {mode:?}"
            );
        }
    }
}

/// The pre-optimization implementation, kept as the oracle for the fast path.
fn reference_normalize_whitespace(text: &str, white_space: WhiteSpace) -> String {
    let text = text.replace("\r\n", "\n");
    let text = text.replace(['\r', '\x0c'], "\n");

    let mut result = String::new();
    let mut prev_was_space = false;

    for c in text.chars() {
        match white_space {
            WhiteSpace::Normal | WhiteSpace::Nowrap => {
                if c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\x0c' {
                    if !prev_was_space {
                        result.push(' ');
                    }
                    prev_was_space = true;
                } else {
                    result.push(c);
                    prev_was_space = false;
                }
            }
            WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces => {
                result.push(c);
                prev_was_space = false;
            }
            WhiteSpace::PreLine => {
                if c == '\n' {
                    result.push('\n');
                    prev_was_space = false;
                } else if c == ' ' || c == '\t' || c == '\r' || c == '\x0c' {
                    if !prev_was_space {
                        result.push(' ');
                    }
                    prev_was_space = true;
                } else {
                    result.push(c);
                    prev_was_space = false;
                }
            }
        }
    }

    result
}

#[test]
fn is_collapsible_whitespace_info_preserves_pre_modes() {
    use crate::engine::layouter::types::{InfoNode, NodeKind, TextFlowStyle};

    let text_info = |white_space: WhiteSpace| InfoNode {
        kind: NodeKind::Text {
            text: "  \n  ".to_string(),
            style: Default::default(),
            flow_style: TextFlowStyle {
                white_space,
                ..Default::default()
            },
            text_id: 0,
        },
        children: Vec::new(),
        dom_id: None,
    };

    // Normal and Nowrap collapse whitespace-only text.
    assert!(is_collapsible_whitespace_info(
        &text_info(WhiteSpace::Normal),
        WhiteSpace::Normal
    ));
    assert!(is_collapsible_whitespace_info(
        &text_info(WhiteSpace::Nowrap),
        WhiteSpace::Nowrap
    ));

    // Preserved-whitespace modes keep whitespace even when it is whitespace-only.
    assert!(!is_collapsible_whitespace_info(
        &text_info(WhiteSpace::Pre),
        WhiteSpace::Pre
    ));
    assert!(!is_collapsible_whitespace_info(
        &text_info(WhiteSpace::PreWrap),
        WhiteSpace::PreWrap
    ));
    assert!(!is_collapsible_whitespace_info(
        &text_info(WhiteSpace::BreakSpaces),
        WhiteSpace::BreakSpaces
    ));
    // PreLine also preserves whitespace (segment breaks are preserved).
    assert!(!is_collapsible_whitespace_info(
        &text_info(WhiteSpace::PreLine),
        WhiteSpace::PreLine
    ));
}

#[test]
fn is_collapsible_whitespace_info_non_text_is_not_collapsible() {
    use crate::engine::layouter::types::{InfoNode, NodeKind, WhiteSpace};

    let container_info = InfoNode {
        kind: NodeKind::Container {
            scroll_x: false,
            scroll_y: false,
            scroll_offset_x: 0.0,
            scroll_offset_y: 0.0,
            style: Default::default(),
            role: ContainerRole::Normal,
        },
        children: Vec::new(),
        dom_id: None,
    };

    assert!(!is_collapsible_whitespace_info(
        &container_info,
        WhiteSpace::Normal
    ));
    assert!(!is_collapsible_whitespace_info(
        &container_info,
        WhiteSpace::Pre
    ));
}

#[test]
fn shadow_dom_slot_projection_does_not_panic() {
    use crate::engine::tree::TreeNode as _;

    // Build: <html><body><div id="host"></div></body></html> with shadow root + slot
    let html = TreeNode::new(HtmlNodeType::Document);
    let body = TreeNode::new(HtmlNodeType::Element {
        tag_name: "body".to_string(),
        attributes: Vec::new(),
    });
    let host = TreeNode::new(HtmlNodeType::Element {
        tag_name: "div".to_string(),
        attributes: vec![crate::engine::html::parser::OwnedAttribute {
            name: "id".to_string(),
            value: "host".to_string(),
        }],
    });
    TreeNode::add_child(&html, Rc::clone(&body));
    TreeNode::add_child(&body, Rc::clone(&host));

    // Light DOM child assigned to slot
    let light_div = TreeNode::new(HtmlNodeType::Element {
        tag_name: "div".to_string(),
        attributes: vec![crate::engine::html::parser::OwnedAttribute {
            name: "slot".to_string(),
            value: "items".to_string(),
        }],
    });
    TreeNode::add_child(&host, Rc::clone(&light_div));

    // Shadow root attached to host
    let shadow_root = TreeNode::new(HtmlNodeType::ShadowRoot {
        mode: ShadowRootMode::Open,
        id: 1,
    });
    TreeNode::add_child(&host, Rc::clone(&shadow_root));

    // <slot name="items"> inside shadow tree
    let slot = TreeNode::new(HtmlNodeType::Element {
        tag_name: "slot".to_string(),
        attributes: vec![crate::engine::html::parser::OwnedAttribute {
            name: "name".to_string(),
            value: "items".to_string(),
        }],
    });
    TreeNode::add_child(&shadow_root, Rc::clone(&slot));

    // Fallback content inside slot
    let fallback_div = TreeNode::new(HtmlNodeType::Element {
        tag_name: "div".to_string(),
        attributes: Vec::new(),
    });
    TreeNode::add_child(&slot, Rc::clone(&fallback_div));

    let dom = crate::engine::html::DomTree::from_root(Rc::clone(&html));
    let resolved = crate::engine::layouter::css_resolver::ResolvedStyles::default();
    let (_layout, _info) = build_layout_and_info(
        &dom.root,
        &resolved,
        Arc::new(FallbackTextMeasurer),
        Arc::new(crate::engine::bridge::audio::NullAudioSinkFactory),
        crate::engine::layouter::builder::InheritedCss::default(),
        crate::engine::css::matcher::ElementChain::default(),
        ColorScheme::Light,
        crate::engine::html::ScriptingMode::default(),
        (0.0, 0.0),
    );
    // If we reach here without panicking, the test passes.
}
