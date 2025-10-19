use crate::engine::html::tokenizer::{Attribute, Token, Tokenizer};
use crate::engine::html::util as html_util;
use crate::engine::tree::{Tree, TreeNode};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub enum NodeType {
    Document,
    Element {
        tag_name: String,
        attributes: Vec<Attribute>,
    },
    Text(String),
    Comment(String),
    Doctype {
        name: Option<String>,
        public_id: Option<String>,
        system_id: Option<String>,
    },
}

pub struct Parser<'a> {
    tokenizer: crate::engine::html::tokenizer::Tokenizer<'a>,
    tree: Tree<NodeType>,
    stack: Vec<Rc<RefCell<TreeNode<NodeType>>>>,
}

impl<'a> Parser<'a> {
    pub fn new(input: &'a str) -> Self {
        let document = Tree::new(NodeType::Document);

        Self {
            tokenizer: Tokenizer::new(input),
            tree: document.clone(),
            stack: vec![document.root.clone()],
        }
    }

    pub fn parse(&mut self) -> Tree<NodeType> {
        while let Some(token) = self.tokenizer.next_token() {
            //println!("---");
            //println!("Processing token: {token:?}");
            match token {
                Token::StartTag { .. } => self.handle_start_tag(token),
                Token::EndTag { .. } => self.handle_end_tag(token),
                Token::Doctype { .. } => self.handle_doctype(token),
                Token::Comment(_) => self.handle_comment(token),
                Token::Text(_) => self.handle_text(token),
            }
        }
        self.autofill_elements();

        self.tree.clone()
    }

    fn handle_start_tag(&mut self, token: Token) {
        if let Token::StartTag {
            name,
            attributes,
            self_closing,
        } = token
        {
            let mut parent = Rc::clone(self.stack.last().unwrap());
            if self.check_start_tag_with_invalid_nesting(&name, &parent) {
                if let NodeType::Element { tag_name, .. } = &parent.borrow().value {
                    //println!("Auto-closing tag: <{}> to allow <{}> inside it.", tag_name, name);
                    self.handle_end_tag(Token::EndTag {
                        name: tag_name.clone(),
                    });
                }
                parent = Rc::clone(self.stack.last().unwrap());
            }

            let new_node = TreeNode::new(NodeType::Element {
                tag_name: name.clone(),
                attributes: attributes.clone(),
            });

            TreeNode::add_child(&parent, new_node.clone());
            /*
            let new_node = Rc::new(RefCell::new(Node {
                value: NodeType::Element {
                    tag_name: name,
                    attributes,
                },
                children: vec![],
                parent: Some(Rc::clone(&parent)),
            }));

            parent.borrow_mut().children.push(Rc::clone(&new_node));
            */

            // Self-closing タグは stack に push しない
            if !self_closing {
                self.stack.push(new_node);
            }
        }
    }

    fn handle_end_tag(&mut self, token: Token) {
        if let Token::EndTag { name } = token {
            while let Some(top) = self.stack.pop() {
                if let NodeType::Element { tag_name, .. } = &top.borrow().value {
                    if tag_name == &name {
                        break;
                    }
                }
            }
        }
    }

    fn handle_text(&mut self, token: Token) {
        if let Token::Text(data) = token {
            let parent = Rc::clone(self.stack.last().unwrap());
            // 親ノードが pre, textarea, script, style でない場合、空白改行を無視する
            if let Some(parent_node) = &parent.borrow().parent {
                let parent_node_borrow = parent_node.borrow();
                if let NodeType::Element { tag_name, .. } = &parent_node_borrow.value {
                    if !matches!(tag_name.as_str(), "pre" | "textarea" | "script" | "style")
                        && data.trim().is_empty()
                    {
                        return;
                    }
                } else if data.trim().is_empty() {
                    return;
                }
            } else if data.trim().is_empty() {
                return;
            }
            let text_node = TreeNode::new(NodeType::Text(data));
            TreeNode::add_child(&parent, text_node);
        }
    }

    fn handle_comment(&mut self, token: Token) {
        if let Token::Comment(data) = token {
            let parent = Rc::clone(self.stack.last().unwrap());
            let comment_node = TreeNode::new(NodeType::Comment(data));
            TreeNode::add_child(&parent, comment_node);
        }
    }

    fn handle_doctype(&mut self, token: Token) {
        if let Token::Doctype {
            name,
            public_id,
            system_id,
            ..
        } = token
        {
            let parent = Rc::clone(self.stack.last().unwrap());
            let doctype_node = TreeNode::new(NodeType::Doctype {
                name,
                public_id,
                system_id,
            });
            TreeNode::add_child(&parent, doctype_node);
        }
    }

    fn check_start_tag_with_invalid_nesting(
        &self,
        name: &String,
        parent: &Rc<RefCell<TreeNode<NodeType>>>,
    ) -> bool {
        if let NodeType::Element { tag_name, .. } = &parent.borrow().value {
            // <p> の中に <p> が来た場合、前の <p> を閉じる
            if tag_name == "p" && name == "p" {
                return true;
            }
            // <li> の中に <li> が来た場合、前の <li> を閉じる
            if tag_name == "li" && name == "li" {
                return true;
            }
            // <a> の中に <a> が来た場合、前の <a> を閉じる
            if tag_name == "a" && name == "a" {
                return true;
            }
            // <dt> の中に <dt> または <dd> が来た場合、前の <dt> を閉じる
            if tag_name == "dt" && (name == "dt" || name == "dd") {
                return true;
            }
            // <dd> の中に <dt> または <dd> が来た場合、前の <dd> を閉じる
            if tag_name == "dd" && (name == "dt" || name == "dd") {
                return true;
            }
            // <option> の中に <option> が来た場合、前の <option> を閉じる
            if tag_name == "option" && name == "option" {
                return true;
            }
            // <p> の中にブロック要素が来た場合、前の <p> を閉じる
            if matches!(
                tag_name.as_str(),
                "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            ) && html_util::is_block_level_element(name)
            {
                return true;
            }
        }
        false
    }

    /// DOCTYPE宣言、html, head, body 要素が存在しない場合に補完する
    fn autofill_elements(&mut self) {
        let root = Rc::clone(&self.stack[0]);
        let mut has_doctype = false;
        let mut has_html = false;
        let mut has_head = false;
        let mut has_body = false;

        for child in &root.borrow().children {
            match &child.borrow().value {
                NodeType::Doctype { .. } => has_doctype = true,
                NodeType::Element { tag_name, .. } if tag_name.to_lowercase() == "html" => {
                    has_html = true;
                    for html_child in &child.borrow().children {
                        match &html_child.borrow().value {
                            NodeType::Element { tag_name, .. }
                                if tag_name.to_lowercase() == "head" =>
                            {
                                has_head = true;
                            }
                            NodeType::Element { tag_name, .. }
                                if tag_name.to_lowercase() == "body" =>
                            {
                                has_body = true;
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        if !has_doctype {
            let doctype_node = TreeNode::new(NodeType::Doctype {
                name: Some("html".to_string()),
                public_id: None,
                system_id: None,
            });
            root.borrow_mut().children.insert(0, doctype_node);
        }

        if !has_html {
            let html_node = TreeNode::new(NodeType::Element {
                tag_name: "html".to_string(),
                attributes: vec![],
            });
            root.borrow_mut().children.push(Rc::clone(&html_node));

            if !has_head {
                let head_node = TreeNode::new(NodeType::Element {
                    tag_name: "head".to_string(),
                    attributes: vec![],
                });
                TreeNode::add_child(&html_node, head_node);
            }

            if !has_body {
                let body_node = TreeNode::new(NodeType::Element {
                    tag_name: "body".to_string(),
                    attributes: vec![],
                });
                TreeNode::add_child(&html_node, body_node);
            }
        }
    }
}
