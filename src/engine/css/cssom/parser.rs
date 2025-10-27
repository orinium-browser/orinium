use crate::engine::css::cssom::tokenizer::{Token, Tokenizer};
use crate::engine::css::values::*;
use crate::engine::tree::{Tree, TreeNode};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub enum CssNodeType {
    Stylesheet,
    Rule { selectors: Vec<String> },
    Declaration { name: String, value: CssValue },
}

#[derive(Debug, Clone)]
pub enum CssValue {
    Keyword(String),
    Length(Length),
    Color(Color),
}

pub struct Parser<'a> {
    tokenizer: crate::engine::css::cssom::tokenizer::Tokenizer<'a>,
    tree: Tree<CssNodeType>,
    stack: Vec<Rc<RefCell<TreeNode<CssNodeType>>>>,
}

impl<'a> Parser<'a> {
    pub fn new(input: &'a str) -> Self {
        let tree = Tree::new(CssNodeType::Stylesheet);
        Self {
            tokenizer: Tokenizer::new(input),
            tree: tree.clone(),
            stack: vec![tree.root.clone()],
        }
    }

    pub fn parse(&mut self) -> Tree<CssNodeType> {
        while let Some(token) = self.tokenizer.next_token() {
            match token {
                Token::Ident(selector) => {
                    self.parse_rule(selector);
                }
                Token::Whitespace => continue,
                _ => {}
            }
        }
        self.tree.clone()
    }

    fn parse_rule(&mut self, first_selector: String) {
        let mut selectors = vec![first_selector];

        while let Some(token) = self.tokenizer.next_token() {
            match token {
                Token::Comma => {
                    if let Some(Token::Ident(next_sel)) = self.tokenizer.next_token() {
                        selectors.push(next_sel);
                    }
                }
                Token::LeftBrace => break,
                Token::Whitespace => continue,
                _ => break,
            }
        }

        let rule_node = TreeNode::new(CssNodeType::Rule {
            selectors: selectors.clone(),
        });
        TreeNode::add_child(self.stack.last().unwrap(), rule_node.clone());

        self.stack.push(rule_node.clone());
        self.parse_declarations();
        self.stack.pop();
    }

    fn parse_declarations(&mut self) {
        loop {
            match self.tokenizer.next_token() {
                Some(Token::RightBrace) => break,
                Some(Token::Ident(name)) => {
                    self.expect_colon();
                    let value = self.parse_value();
                    self.expect_semicolon();

                    let decl_node = TreeNode::new(CssNodeType::Declaration { name, value });
                    TreeNode::add_child(self.stack.last().unwrap(), decl_node);
                }
                Some(Token::Whitespace) => continue,
                None => break,
                _ => continue,
            }
        }
    }

    fn expect_colon(&mut self) {
        match self.tokenizer.next_token() {
            Some(Token::Colon) => {}
            _ => panic!("Expected ':' after property name"),
        }
    }

    fn expect_semicolon(&mut self) {
        match self.tokenizer.next_token() {
            Some(Token::Semicolon) => {}
            _ => {} // セミコロン省略は許容
        }
    }

    fn parse_value(&mut self) -> CssValue {
        match self.tokenizer.next_token() {
            Some(Token::Ident(s)) => CssValue::Keyword(s),
            Some(Token::Dimension(n, unit)) => {
                CssValue::Length(Length::from_number_and_unit(n, &unit).unwrap_or_default())
            }
            Some(Token::Hash(hex)) => CssValue::Color(Color::from_hex(&hex).unwrap_or(Color::BLACK)),
            Some(Token::Number(n)) => CssValue::Length(Length::Px(n)),
            _ => CssValue::Keyword("invalid".into()),
        }
    }
}
