use crate::engine::css::cssom::tokenizer::{Token, Tokenizer};
use crate::engine::css::values::*;
use crate::engine::tree::{Tree, TreeNode};
use anyhow::{bail, Result};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub enum CssNodeType {
    Stylesheet,
    Rule { selectors: Vec<String> },
    AtRule { name: String, params: Vec<String> },
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
    selector_buffer: String,
    brace_depth: usize,
}

#[derive(Debug)]
enum MaybeSelector {
    Selector(String),
    NotSelector(String),
    EndRule,
    None,
}

impl<'a> Parser<'a> {
    pub fn new(input: &'a str) -> Self {
        let tree = Tree::new(CssNodeType::Stylesheet);
        Self {
            tokenizer: Tokenizer::new(input),
            tree: tree.clone(),
            stack: vec![tree.root.clone()],
            selector_buffer: String::new(),
            brace_depth: 0,
        }
    }

    pub fn parse(&mut self) -> Result<Tree<CssNodeType>> {
        while let Some(token) = self.tokenizer.next_token() {
            match token {
                Token::LeftBrace => {
                    self.brace_depth += 1;
                    let selector = self.selector_buffer.trim().to_string();
                    self.parse_rule(selector)?;
                    self.selector_buffer.clear();
                }
                Token::AtKeyword(key) => self.parse_at_rule(key)?,
                Token::Delim(_) | Token::Hash(_) | Token::Ident(_) | Token::Comma => {
                    self.selector_buffer.push_str(&token_to_string(&token));
                }
                _ => {}
            }
        }
        Ok(self.tree.clone())
    }

    /// セレクタを収集するヘルパー関数
    fn collect_selector(&mut self) -> MaybeSelector {
        let mut selector = String::new();
        let mut token = self.tokenizer.last_tokenized_token().cloned();

        loop {
            let t = match token.take() {
                Some(t) => t,
                None => match self.tokenizer.next_token() {
                    Some(t2) => t2,
                    None => break,
                },
            };

            match t {
                Token::LeftBrace => {
                    self.brace_depth += 1;
                    break;
                }
                Token::RightBrace => {
                    self.brace_depth -= 1;
                    return MaybeSelector::EndRule;
                }
                Token::Delim(_) | Token::Hash(_) | Token::Ident(_) | Token::Comma => {
                    selector.push_str(&token_to_string(&t));
                }
                Token::Whitespace => {
                    if !selector.is_empty() {
                        selector.push(' ');
                    }
                }
                _ => return MaybeSelector::NotSelector(selector.trim().to_string()),
            }

            token = self.tokenizer.next_token();
        }

        if selector.is_empty() {
            MaybeSelector::None
        } else {
            MaybeSelector::Selector(selector.trim().to_string())
        }
    }

    fn parse_rule(&mut self, selectors: String) -> Result<()> {
        if selectors.trim().is_empty() {
            bail!("Selector name is empty before '{{'");
        }

        let selectors: Vec<String> = selectors.split(',').map(|s| s.trim().to_string()).collect();

        let rule_node = TreeNode::new(CssNodeType::Rule {
            selectors: selectors.clone(),
        });
        TreeNode::add_child(self.stack.last().unwrap(), rule_node.clone());

        self.stack.push(rule_node.clone());
        self.parse_declarations()?;
        self.stack.pop();
        Ok(())
    }

    fn parse_declarations(&mut self) -> Result<()> {
        let mut return_flag = false;
        let mut delim_name = String::new();
        loop {
            match self.tokenizer.next_token() {
                Some(Token::RightBrace) => {
                    self.brace_depth -= 1;
                    break;
                }
                Some(Token::Ident(name)) => {
                    let name = delim_name.to_string() + name.as_str();
                    delim_name.clear();
                    self.expect_colon()?;
                    let mut value = String::new();

                    while let Some(token) = self.tokenizer.next_token() {
                        match token {
                            Token::Semicolon => break,
                            Token::RightBrace => {
                                self.brace_depth -= 1;
                                return_flag = true;
                                break;
                            }
                            _ => value.push_str(&token_to_string(&token)),
                        }
                    }

                    let value = value.trim().to_string();
                    let parsed_value = self.parse_value(&value)?;

                    let decl_node = TreeNode::new(CssNodeType::Declaration {
                        name,
                        value: parsed_value,
                    });
                    TreeNode::add_child(self.stack.last().unwrap(), decl_node);

                    if return_flag {
                        return Ok(());
                    }
                }
                Some(Token::Whitespace) => continue,
                Some(Token::Delim(c)) => {
                    delim_name.push(c);
                }
                None => break,
                Some(Token::Comment(_)) => continue,
                Some(tok) => bail!("Unexpected token in declaration: {:?}", tok),
            }
        }
        Ok(())
    }

    fn parse_at_rule(&mut self, name: String) -> Result<()> {
        println!("depth: {}", self.brace_depth);
        let mut params = Vec::new();

        while let Some(token) = self.tokenizer.next_token() {
            match &token {
                Token::Semicolon => {
                    let node = TreeNode::new(CssNodeType::AtRule { name, params });
                    TreeNode::add_child(self.stack.last().unwrap(), node);
                    return Ok(());
                }
                Token::LeftBrace => {
                    self.brace_depth += 1;
                    return self.parse_at_rule_block(name, params);
                }
                Token::RightBrace => {
                    self.brace_depth -= 1;
                    break;
                }
                _ => params.push(token_to_string(&token)),
            }
        }
        Ok(())
    }

    fn parse_at_rule_block(&mut self, name: String, params: Vec<String>) -> Result<()> {
        println!("Parsing at-rule block: {} {:?}", name, params);
        let node = TreeNode::new(CssNodeType::AtRule { name, params });
        TreeNode::add_child(self.stack.last().unwrap(), node.clone());
        self.stack.push(node.clone());

        while self.brace_depth > 0 {
            self.skip_whitespace();
            match dbg!(self.collect_selector()) {
                MaybeSelector::Selector(selector) => self.parse_rule(selector)?,
                MaybeSelector::NotSelector(name) => self.parse_at_rule_declaration(name)?,
                MaybeSelector::EndRule => break,
                MaybeSelector::None => {
                    bail!("Expected selector or declaration inside at-rule block")
                }
            }
        }

        self.stack.pop();
        println!("Finished parsing at-rule block");
        Ok(())
    }

    fn parse_at_rule_declaration(&mut self, name: String) -> Result<()> {
        let mut value = String::new();
        while let Some(token) = self.tokenizer.next_token() {
            match token {
                Token::Semicolon => break,
                Token::RightBrace => {
                    self.brace_depth -= 1;
                    break;
                }
                _ => value.push_str(&token_to_string(&token)),
            }
        }
        let parsed_value = self.parse_value(&value)?;
        let decl_node = TreeNode::new(CssNodeType::Declaration {
            name: name.trim().to_string(),
            value: parsed_value,
        });
        TreeNode::add_child(self.stack.last().unwrap(), decl_node);

        self.parse_declarations()?;
        Ok(())
    }

    fn expect_colon(&mut self) -> Result<()> {
        match self.tokenizer.next_token() {
            Some(Token::Colon) => Ok(()),
            Some(tok) => bail!("Expected ':' after property name, found {:?}", tok),
            None => bail!("Unexpected end of input: expected ':'"),
        }
    }

    fn parse_value(&self, css_str: &String) -> Result<CssValue> {
        let css_str = css_str.trim();
        if let Some(length) = Length::from_css(css_str) {
            Ok(CssValue::Length(length))
        } else if let Some(color) = Color::from_hex(css_str) {
            Ok(CssValue::Color(color))
        } else if let Some(color) = Color::from_named(css_str) {
            Ok(CssValue::Color(color))
        } else {
            Ok(CssValue::Keyword(css_str.to_string()))
        }
    }

    /// 空白トークンをスキップするヘルパー関数
    fn skip_whitespace(&mut self) {
        while let Some(Token::Whitespace) = self.tokenizer.next_token() {}
    }
}

/// トークンを文字列化するヘルパー関数
/// 例: Token::Ident("body") -> "body"
/// コメントは無視する
fn token_to_string(token: &Token) -> String {
    match token {
        Token::Ident(s) => s.clone(),
        Token::StringLiteral(s) => format!("\"{}\"", s),
        Token::Number(n) => n.to_string(),
        Token::Dimension(n, unit) => format!("{}{}", n, unit),
        Token::Percentage(n) => format!("{}%", n),
        Token::Colon => ":".into(),
        Token::Semicolon => ";".into(),
        Token::Comma => ",".into(),
        Token::LeftParen => "(".into(),
        Token::RightParen => ")".into(),
        Token::Whitespace => " ".into(),
        Token::Hash(h) => format!("#{}", h),
        Token::Delim(c) => c.to_string(),
        _ => String::new(),
    }
}
