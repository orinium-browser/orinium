use crate::engine::html::tokenizer::{Attribute, Token, Tokenizer};
use crate::engine::html::util as html_util;
use crate::engine::tree::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub enum HtmlNodeType {
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
    InvalidNode(Token, String), // 不正なトークン用
}

impl HtmlNodeType {
    pub fn tag_name(&self) -> String {
        match self {
            HtmlNodeType::Element { tag_name, .. } => tag_name.clone(),
            _ => "".to_string(),
        }
    }
}

pub type DomTree = Tree<HtmlNodeType>;

impl DomTree {
    /// 指定したタグ名の要素のテキストノードをすべて集める
    pub fn collect_text_by_tag(&self, tag_name: &str) -> Vec<String> {
        let mut texts = Vec::new();

        self.traverse(&mut |node| {
            let n = node.borrow();
            if let HtmlNodeType::Element { tag_name: t, .. } = &n.value
                && t.eq_ignore_ascii_case(tag_name)
            {
                let children = &n.children;
                for child in children {
                    let child_ref = child.borrow();
                    if let HtmlNodeType::Text(content) = &child_ref.value {
                        texts.push(content.clone());
                    }
                }
            }
        });

        texts
    }
}

pub struct Parser<'a> {
    tokenizer: crate::engine::html::tokenizer::Tokenizer<'a>,
    tree: DomTree,
    stack: Vec<Rc<RefCell<TreeNode<HtmlNodeType>>>>,
    tag_stack: Vec<String>,
}

impl<'a> Parser<'a> {
    pub fn new(input: &'a str) -> Self {
        let document = Tree::new(HtmlNodeType::Document);

        Self {
            tokenizer: Tokenizer::new(input),
            tree: document.clone(),
            stack: vec![document.root.clone()],
            tag_stack: vec![],
        }
    }

    pub fn parse(&mut self) -> DomTree {
        while let Some(token) = self.tokenizer.next_token() {
            log::debug!(target:"HtmlParser::Token" ,"Processing token: {token:?}");
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
            self.tag_stack.push(name.clone());
            let mut parent = Rc::clone(self.stack.last().unwrap());
            if self.check_start_tag_with_invalid_nesting(&name, &parent) {
                if let HtmlNodeType::Element { tag_name, .. } = &parent.borrow().value {
                    //println!("Auto-closing tag: <{}> to allow <{}> inside it.", tag_name, name);
                    self.handle_end_tag(Token::EndTag {
                        name: tag_name.clone(),
                    });
                }
                parent = Rc::clone(self.stack.last().unwrap());
            }

            let new_node = TreeNode::add_child_value(
                &parent,
                HtmlNodeType::Element {
                    tag_name: name.clone(),
                    attributes: attributes.clone(),
                },
            );

            // Self-closing タグは stack に push しない
            if !self_closing {
                self.stack.push(new_node);
                log::debug!(target:"HtmlParser::Stack" ,"Stack len: {}, +Pushed <{}> to stack.", self.stack.len(), name);
            }
        }
    }

    fn handle_end_tag(&mut self, token: Token) {
        if let Token::EndTag { ref name } = token {
            let name = name.clone();
            if self.tag_stack.contains(&name) {
                while let Some(top) = self.stack.pop() {
                    self.tag_stack.pop();
                    if let HtmlNodeType::Element { tag_name, .. } = &top.borrow().value {
                        if tag_name == &name {
                            log::debug!(target:"HtmlParser::Stack" ,"Stack len: {}, -Popped </{}> from stack.", self.stack.len(), name);
                            break;
                        } else {
                            log::debug!(target:"HtmlParser::Stack" ,"Stack len: {}, Unmatched end tag: </{}>, Find <{}>", self.stack.len(), name, tag_name);
                        }
                    }
                }
            } else {
                let parent = Rc::clone(self.stack.last().unwrap());
                TreeNode::add_child_value(
                    &parent,
                    HtmlNodeType::InvalidNode(
                        token,
                        format!("No matching start tag for </{}>", name),
                    ),
                );
                log::debug!(target:"HtmlParser::Invalid" ,"Invalid end tag: </{}>", name);
            }
        }
    }

    fn handle_text(&mut self, token: Token) {
        if let Token::Text(data) = token {
            let parent = Rc::clone(self.stack.last().unwrap());
            // 親ノードが pre, textarea, script, style でない場合、空白改行を無視する
            if let Some(parent_node) = &parent.borrow().parent {
                let parent_ref = parent_node.upgrade().unwrap();
                let parent_node_borrow = parent_ref.borrow();
                if let HtmlNodeType::Element { tag_name, .. } = &parent_node_borrow.value {
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
            TreeNode::add_child_value(&parent, HtmlNodeType::Text(data));
        }
    }

    fn handle_comment(&mut self, token: Token) {
        if let Token::Comment(data) = token {
            let parent = Rc::clone(self.stack.last().unwrap());
            TreeNode::add_child_value(&parent, HtmlNodeType::Comment(data));
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
            TreeNode::add_child_value(
                &parent,
                HtmlNodeType::Doctype {
                    name,
                    public_id,
                    system_id,
                },
            );
        }
    }

    fn check_start_tag_with_invalid_nesting(
        &self,
        name: &String,
        parent: &Rc<RefCell<TreeNode<HtmlNodeType>>>,
    ) -> bool {
        if let HtmlNodeType::Element { tag_name, .. } = &parent.borrow().value {
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
                HtmlNodeType::Doctype { .. } => has_doctype = true,
                HtmlNodeType::Element { tag_name, .. } if tag_name.to_lowercase() == "html" => {
                    has_html = true;
                    for html_child in &child.borrow().children {
                        match &html_child.borrow().value {
                            HtmlNodeType::Element { tag_name, .. }
                                if tag_name.to_lowercase() == "head" =>
                            {
                                has_head = true;
                            }
                            HtmlNodeType::Element { tag_name, .. }
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
            let doctype_node = TreeNode::new(HtmlNodeType::Doctype {
                name: Some("html".to_string()),
                public_id: None,
                system_id: None,
            });
            root.borrow_mut().children.insert(0, doctype_node);
        }

        if !has_html {
            let html_node = TreeNode::new(HtmlNodeType::Element {
                tag_name: "html".to_string(),
                attributes: vec![],
            });
            root.borrow_mut().children.push(Rc::clone(&html_node));

            if !has_head {
                TreeNode::add_child_value(
                    &html_node,
                    HtmlNodeType::Element {
                        tag_name: "head".to_string(),
                        attributes: vec![],
                    },
                );
            }

            if !has_body {
                TreeNode::add_child_value(
                    &html_node,
                    HtmlNodeType::Element {
                        tag_name: "body".to_string(),
                        attributes: vec![],
                    },
                );
            }
        }
    }
}
