use crate::engine::css::cssom::tokenizer::{Token, Tokenizer};
use crate::engine::css::values::*;
use crate::engine::tree::{Tree, TreeNode};
use anyhow::{Result, bail};
use std::cell::RefCell;
use std::rc::Rc;

/// Node types stored in the CSS AST tree.
#[derive(Debug, Clone)]
pub enum CssNodeType {
    Stylesheet,
    Rule { selectors: Vec<Selector> },
    AtRule { name: String, params: Vec<String> },
    Declaration { name: String, value: CssValue },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Selector {
    pub tag: Option<String>,
    pub classes: Vec<String>,
    pub pseudo_class: Option<String>,
    pub pseudo_element: Option<String>,
}

/// Parsed CSS property values.
#[derive(Debug, Clone)]
pub enum CssValue {
    Keyword(String),
    Length(Length),
    Color(Color),
}

/// CSS parser consuming tokens and producing a syntax tree.
pub struct Parser<'a> {
    tokenizer: Tokenizer<'a>,
    tree: Tree<CssNodeType>,
    stack: Vec<Rc<RefCell<TreeNode<CssNodeType>>>>,
    brace_depth: usize,
}

impl<'a> Parser<'a> {
    /// Create a new CSS parser.
    pub fn new(input: &'a str) -> Self {
        let tree = Tree::new(CssNodeType::Stylesheet);
        Self {
            tokenizer: Tokenizer::new(input),
            tree: tree.clone(),
            stack: vec![tree.root.clone()],
            brace_depth: 0,
        }
    }

    /// Parse the entire stylesheet.
    pub fn parse(&mut self) -> Result<Tree<CssNodeType>> {
        let mut selector_buffer = String::new();

        while let Some(token) = self.tokenizer.next_token() {
            match token {
                Token::Whitespace => {
                    if !selector_buffer.is_empty() {
                        selector_buffer.push(' ');
                    }
                }

                Token::Comma | Token::Ident(_) | Token::Hash(_) | Token::Delim(_) => {
                    selector_buffer.push_str(&token_to_string(&token));
                }

                Token::AtKeyword(name) => {
                    self.parse_at_rule(&name)?;
                }

                Token::LeftBrace => {
                    self.brace_depth += 1;
                    let selector = selector_buffer.trim();
                    self.parse_rule(selector)?;
                    selector_buffer.clear();
                }

                _ => {}
            }
        }

        Ok(self.tree.clone())
    }

    /// Parse a qualified rule (`selector { ... }`).
    fn parse_rule(&mut self, selector: &str) -> Result<()> {
        if selector.is_empty() {
            bail!("Empty selector before '{{'");
        }

        let selectors = selector
            .split(',')
            .map(|s| Self::parse_selector(s.trim()))
            .collect::<Vec<_>>();

        let rule_node =
            TreeNode::add_child_value(self.stack.last().unwrap(), CssNodeType::Rule { selectors });

        self.stack.push(rule_node);
        self.parse_declarations()?;
        self.stack.pop();

        Ok(())
    }

    /// Parse a declaration block.
    fn parse_declarations(&mut self) -> Result<()> {
        loop {
            match self.tokenizer.next_token() {
                Some(Token::Whitespace | Token::Comment(_)) => continue,

                Some(Token::RightBrace) => {
                    self.brace_depth -= 1;
                    return Ok(());
                }

                Some(Token::Ident(name)) => {
                    self.expect_colon()?;
                    let value = self.collect_value()?;
                    let parsed_value = self.parse_value(&value)?;

                    TreeNode::add_child_value(
                        self.stack.last().unwrap(),
                        CssNodeType::Declaration {
                            name,
                            value: parsed_value,
                        },
                    );
                }

                Some(tok) => bail!("Unexpected token in declaration block: {tok:?}"),
                None => break,
            }
        }

        Ok(())
    }

    /// Parse an at-rule (`@media`, `@import`, etc.).
    fn parse_at_rule(&mut self, name: &str) -> Result<()> {
        let mut params = Vec::new();

        while let Some(token) = self.tokenizer.next_token() {
            match token {
                Token::Semicolon => {
                    TreeNode::add_child_value(
                        self.stack.last().unwrap(),
                        CssNodeType::AtRule {
                            name: name.to_string(),
                            params,
                        },
                    );
                    return Ok(());
                }

                Token::LeftBrace => {
                    self.brace_depth += 1;
                    return self.parse_at_rule_block(name, params);
                }

                Token::Whitespace => continue,
                _ => params.push(token_to_string(&token)),
            }
        }

        Ok(())
    }

    /// Parse an at-rule with a block (`@media ... { ... }`).
    fn parse_at_rule_block(&mut self, name: &str, params: Vec<String>) -> Result<()> {
        let node = TreeNode::add_child_value(
            self.stack.last().unwrap(),
            CssNodeType::AtRule {
                name: name.to_string(),
                params,
            },
        );

        self.stack.push(node);

        let mut selector_buffer = String::new();

        while self.brace_depth > 0 {
            match self.tokenizer.next_token() {
                Some(Token::Whitespace) => {
                    if !selector_buffer.is_empty() {
                        selector_buffer.push(' ');
                    }
                }

                Some(Token::Ident(_) | Token::Hash(_) | Token::Delim(_) | Token::Comma) => {
                    selector_buffer.push_str(&token_to_string(
                        self.tokenizer.last_tokenized_token().unwrap(),
                    ));
                }

                Some(Token::LeftBrace) => {
                    self.brace_depth += 1;
                    let selector = selector_buffer.trim();
                    self.parse_rule(selector)?;
                    selector_buffer.clear();
                }

                Some(Token::RightBrace) => {
                    self.brace_depth -= 1;
                }

                Some(Token::AtKeyword(name)) => {
                    self.parse_at_rule(&name)?;
                }

                Some(Token::Comment(_)) => {}

                None => break,
                _ => {}
            }
        }

        self.stack.pop();
        Ok(())
    }

    /// Collect a property value until `;` or `}`.
    fn collect_value(&mut self) -> Result<String> {
        let mut value = String::new();

        while let Some(token) = self.tokenizer.next_token() {
            match token {
                Token::Semicolon => break,
                Token::RightBrace => {
                    self.brace_depth -= 1;
                    break;
                }
                Token::Whitespace => value.push(' '),
                _ => value.push_str(&token_to_string(&token)),
            }
        }

        Ok(value.trim().to_string())
    }

    /// Expect a colon token after a property name.
    fn expect_colon(&mut self) -> Result<()> {
        match self.tokenizer.next_token() {
            Some(Token::Colon) => Ok(()),
            Some(tok) => bail!("Expected ':' after property name, found {tok:?}"),
            None => bail!("Unexpected end of input, expected ':'"),
        }
    }

    /// Parse a CSS value string into a structured value.
    fn parse_value(&self, css: &str) -> Result<CssValue> {
        if let Some(length) = Length::from_css(css) {
            Ok(CssValue::Length(length))
        } else if let Some(color) = Color::from_hex(css) {
            Ok(CssValue::Color(color))
        } else if let Some(color) = Color::from_named(css) {
            Ok(CssValue::Color(color))
        } else {
            Ok(CssValue::Keyword(css.to_string()))
        }
    }

    /// Parse css selector.
    fn parse_selector(input: &str) -> Selector {
        let mut rest = input;
        let mut pseudo_class = None;
        let mut pseudo_element = None;

        if let Some(idx) = rest.find("::") {
            pseudo_element = Some(rest[idx + 2..].to_string());
            rest = &rest[..idx];
        } else if let Some(idx) = rest.find(':') {
            pseudo_class = Some(rest[idx + 1..].to_string());
            rest = &rest[..idx];
        }

        let mut tag = None;
        let mut classes = Vec::new();

        for part in rest.split('.') {
            if tag.is_none() {
                if !part.is_empty() {
                    tag = Some(part.to_string());
                }
            } else {
                classes.push(part.to_string());
            }
        }

        Selector {
            tag,
            classes,
            pseudo_class,
            pseudo_element,
        }
    }
}

/// Convert a token back into its textual representation.
fn token_to_string(token: &Token) -> String {
    match token {
        Token::Ident(s) => s.clone(),
        Token::StringLiteral(s) => format!("\"{s}\""),
        Token::Number(n) => n.to_string(),
        Token::Dimension(n, unit) => format!("{n}{unit}"),
        Token::Percentage(n) => format!("{n}%"),
        Token::Colon => ":".into(),
        Token::Semicolon => ";".into(),
        Token::Comma => ",".into(),
        Token::LeftParen => "(".into(),
        Token::RightParen => ")".into(),
        Token::Whitespace => " ".into(),
        Token::Hash(h) => format!("#{h}"),
        Token::Delim(c) => c.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_rule() {
        let css = "body { color: red; }";
        let mut parser = Parser::new(css);
        let tree = parser.parse().unwrap();

        let root = tree.root.borrow();
        assert_eq!(root.children().len(), 1);
    }

    #[test]
    fn parse_at_rule() {
        let css = "@media screen { body { margin: 10px; } }";
        let mut parser = Parser::new(css);
        let tree = parser.parse().unwrap();

        let root = tree.root.borrow();
        assert_eq!(root.children().len(), 1);
    }
}
