//! Minimal XML parser that turns well-formed XML markup into a generic
//! element tree.
//!
//! This is deliberately *not* a full XML spec implementation. It covers the
//! structural subset that embedded documents (notably SVG) actually use:
//! element/attribute/name constructs, self-closing tags, processing
//! instructions (`<?...?>`), comments, and `<!DOCTYPE ...>` prologs. Element
//! and attribute names are case-sensitive, and namespace prefixes are kept as
//! literal name characters (`svg:path`), matching XML semantics rather than
//! the HTML parser's case-insensitive handling.
//!
//! The parser is a thin tree-builder over [`super::tokenizer::Tokenizer`],
//! reusing the shared HTML tokenizer for low-level lexing so that attribute
//! value quoting, comment syntax, entity decoding, and whitespace handling
//! all follow a single well-tested implementation.

use super::tokenizer::{Attribute, Token, Tokenizer};
use crate::engine::tree::{NodeRef, Tree, TreeNode};
use std::rc::Rc;

/// A parsed XML element: its (case-sensitive, possibly namespace-qualified)
/// tag name and its attribute list.
#[derive(Debug, Clone, PartialEq)]
pub struct XmlElement {
    pub name: String,
    pub attributes: Vec<Attribute>,
}

impl XmlElement {
    /// Returns the value of the attribute named `name`, or `None`.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name == name)
            .map(|attribute| attribute.value.as_str())
    }
}

/// Parses XML markup into an element tree.
///
/// The root of the returned tree is the document element (the first element of
/// the document). Everything before it — the XML declaration `<?xml ...?>`,
/// comments, and the `<!DOCTYPE ...>` prolog — is skipped. Returns an error
/// when the markup is not well-formed.
pub fn parse(markup: &str) -> Result<Tree<XmlElement>, String> {
    let mut tokenizer = Tokenizer::new(markup);
    let mut root: Option<NodeRef<XmlElement>> = None;
    let mut stack: Vec<NodeRef<XmlElement>> = Vec::new();

    while let Some(token) = tokenizer.next_token() {
        match token {
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } => {
                let node = TreeNode::new(XmlElement { name, attributes });
                match stack.last() {
                    Some(parent) => {
                        if self_closing {
                            TreeNode::add_child(parent, node);
                        } else {
                            TreeNode::add_child(parent, Rc::clone(&node));
                            stack.push(Rc::clone(&node));
                        }
                    }
                    None => {
                        if self_closing {
                            root = Some(node);
                        } else {
                            root = Some(Rc::clone(&node));
                            stack.push(node);
                        }
                    }
                }
            }
            Token::EndTag { name } => {
                if let Some(top) = stack.last() {
                    if top.borrow().value.name == name {
                        stack.pop();
                    } else {
                        return Err(format!(
                            "XML: mismatched closing tag </{name}> for <{}>",
                            top.borrow().value.name
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    match root {
        Some(root) if stack.is_empty() => Ok(Tree::from_root(root)),
        Some(_) => Err(format!(
            "XML: unclosed element <{}>",
            stack.last().unwrap().borrow().value.name
        )),
        None => Err("XML: no document element".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_svg_document_with_pi_and_comment() {
        let tree = parse(
            r##"<?xml version="1.0" encoding="UTF-8"?>
               <!-- a comment -->
               <svg width="12" height="8" xmlns="http://www.w3.org/2000/svg">
                 <g id="layer">
                   <rect width="12" height="8" fill="#ff0000"/>
                   <path d="M 0 0 L 1 1"/>
                 </g>
               </svg>"##,
        )
        .expect("parses");
        let root = tree.root.borrow();
        assert_eq!(root.value.name, "svg");
        assert_eq!(root.value.attr("width"), Some("12"));
        let children = root.children().to_vec();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].borrow().value.name, "g");
        let g_children = children[0].borrow().children().to_vec();
        assert_eq!(g_children.len(), 2);
        assert_eq!(g_children[0].borrow().value.name, "rect");
        assert_eq!(g_children[0].borrow().value.attr("fill"), Some("#ff0000"));
        assert_eq!(g_children[1].borrow().value.name, "path");
    }

    #[test]
    fn keeps_namespace_qualified_names() {
        let tree = parse(
            r#"<svg xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape">
                 <inkscape:label foo:bar="1"/>
               </svg>"#,
        )
        .expect("parses");
        let root = tree.root.borrow();
        assert_eq!(root.value.name, "svg");
        let ns = &root.children()[0].borrow().value;
        assert_eq!(ns.name, "inkscape:label");
        assert_eq!(ns.attr("foo:bar"), Some("1"));
    }

    #[test]
    fn element_names_are_case_sensitive() {
        let tree = parse(r#"<svg><path d="M0 0"/><PATH d="x"/></svg>"#).expect("parses");
        let root = tree.root.borrow();
        let children = root.children().to_vec();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].borrow().value.name, "path");
        assert_eq!(children[1].borrow().value.name, "PATH");
    }

    #[test]
    fn accepts_well_formed_and_rejects_malformed() {
        assert!(parse("<svg><rect/></svg>").is_ok());
        assert!(parse("<svg/>").is_ok());
        assert!(parse("<svg><rect/></svg>trailing").is_ok());
        assert!(parse("<svg>").is_err());
        assert!(parse("<svg><rect/></g>").is_err());
        assert!(parse("plain text").is_err());
    }
}
