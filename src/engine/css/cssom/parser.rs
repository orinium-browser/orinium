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
    Rule { selectors: Vec<ComplexSelector> },
    AtRule { name: String, params: Vec<String> },
    Declaration { name: String, value: CssValue },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Selector {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub pseudo_class: Option<String>,
    pub pseudo_element: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Combinator {
    Descendant,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SelectorPart {
    pub selector: Selector,
    pub combinator: Option<Combinator>, // 左側との関係
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ComplexSelector {
    /// Sort: Right → Left
    pub parts: Vec<SelectorPart>,
}

/// Parsed CSS property values.
#[derive(Debug, Clone)]
pub enum CssValue {
    Keyword(String),
    Length(Length),
    Color(Color),
    List(Vec<CssValue>),
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

                Token::Comma
                | Token::Ident(_)
                | Token::Hash(_)
                | Token::Delim(_)
                | Token::Colon => {
                    selector_buffer.push_str(&token_to_string(&token));
                }

                Token::AtKeyword(name) => {
                    self.parse_at_rule(&name)?;
                }

                Token::LeftBrace => {
                    self.brace_depth += 1;
                    let selector = selector_buffer.trim().to_string();
                    selector_buffer.clear();
                    self.parse_rule(&selector)?;
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
            .map(|s| Self::parse_complex_selector(s.trim()))
            .collect::<Vec<_>>();

        log::info!(
            target: "CssParser::Rule",
            "Parsed Selectors: {:?}",
            selectors
        );

        let rule_node =
            TreeNode::add_child_value(self.stack.last().unwrap(), CssNodeType::Rule { selectors });

        self.stack.push(rule_node);
        self.parse_declarations()?;
        self.stack.pop();

        log::info!(target: "CssParser::Rule", "Done parsing rule.");

        Ok(())
    }

    fn parse_complex_selector(input: &str) -> ComplexSelector {
        log::info!(
            target: "CssParser::ComplexSelector",
            "Parsing complex selector: {}",
            input
        );

        let mut parts = Vec::new();

        // descendant combinator）
        let simples: Vec<&str> = input.split_whitespace().collect();

        for (i, simple) in simples.iter().rev().enumerate() {
            let selector = Self::parse_selector(simple);

            let combinator = if i + 1 < simples.len() {
                Some(Combinator::Descendant)
            } else {
                None
            };

            parts.push(SelectorPart {
                selector,
                combinator,
            });
        }

        ComplexSelector { parts }
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

                Some(Token::LeftBrace) => {
                    // Start of next rule
                    self.tokenizer.unread_token();
                    return Ok(());
                }

                Some(Token::Ident(name)) => {
                    self.expect_colon()?;
                    let value = self.collect_value()?;
                    let parsed_value = Self::parse_value(&value)?;

                    log::info!(
                        target: "CssParser::Declaration",
                        "Added declaration: {}: {:?}",
                        name,
                        parsed_value
                    );
                    TreeNode::add_child_value(
                        self.stack.last().unwrap(),
                        CssNodeType::Declaration {
                            name,
                            value: parsed_value,
                        },
                    );
                }
                // CSS vendor prefix (ignoring for now) OR CSS hack (ex: _width, *opacity)
                // TODO: ignore CSS hack
                Some(Token::Delim(_)) => {}

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
                    log::info!(target:"CssParser::AtRule", "Parsed at-rule: @{} with params: {:?}", name, params);
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
                    if !selector_buffer.is_empty() && !selector_buffer.ends_with(' ') {
                        selector_buffer.push(' ');
                    }
                }
                Some(
                    Token::Ident(_)
                    | Token::Hash(_)
                    | Token::Delim(_)
                    | Token::Comma
                    | Token::Colon,
                ) => {
                    selector_buffer.push_str(&token_to_string(
                        self.tokenizer.last_tokenized_token().unwrap(),
                    ));
                }
                Some(Token::LeftBrace) => {
                    self.brace_depth += 1;
                    let selector = selector_buffer.trim();
                    if !selector.is_empty() {
                        self.parse_rule(selector)?;
                    }
                    selector_buffer.clear();
                }
                Some(Token::RightBrace) => {
                    self.brace_depth -= 1;
                }
                Some(Token::AtKeyword(name)) => self.parse_at_rule(&name)?,
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
                    self.tokenizer.unread_token();
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
    fn parse_value(css: &str) -> Result<CssValue> {
        let parts: Vec<&str> = css.split_whitespace().collect();

        if parts.len() > 1 {
            let mut values = Vec::new();
            for part in parts {
                values.push(Self::parse_single_value(part)?);
            }
            return Ok(CssValue::List(values));
        }

        Self::parse_single_value(css)
    }

    fn parse_single_value(css: &str) -> Result<CssValue> {
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

    /// Parse CSS selector.
    fn parse_selector(input: &str) -> Selector {
        log::info!(target:"CssParser::Selector", "Parsing selector: {}", input);

        let mut rest = input;
        let mut pseudo_class = None;
        let mut pseudo_element = None;

        // 疑似要素・疑似クラス
        if let Some(idx) = rest.find("::") {
            pseudo_element = Some(rest[idx + 2..].to_string());
            rest = &rest[..idx];
        } else if let Some(idx) = rest.find(':') {
            pseudo_class = Some(rest[idx + 1..].to_string());
            rest = &rest[..idx];
        }

        let mut tag = None;
        let mut id = None;
        let mut classes = Vec::new();

        // 例: div#main.content.large
        let mut buf = String::new();
        let chars = rest.chars().peekable();

        enum Mode {
            Tag,
            Id,
            Class,
        }

        let mut mode = Mode::Tag;

        for c in chars {
            match c {
                '#' => {
                    if !buf.is_empty() && tag.is_none() {
                        tag = Some(buf.clone());
                    }
                    buf.clear();
                    mode = Mode::Id;
                }
                '.' => {
                    match mode {
                        Mode::Tag => {
                            if !buf.is_empty() && tag.is_none() {
                                tag = Some(buf.clone());
                            }
                        }
                        Mode::Id => {
                            if !buf.is_empty() && id.is_none() {
                                id = Some(buf.clone());
                            }
                        }
                        Mode::Class => {
                            if !buf.is_empty() {
                                classes.push(buf.clone());
                            }
                        }
                    }
                    buf.clear();
                    mode = Mode::Class;
                }
                _ => buf.push(c),
            }
        }

        // 残りを確定
        match mode {
            Mode::Tag => {
                if !buf.is_empty() && tag.is_none() {
                    tag = Some(buf);
                }
            }
            Mode::Id => {
                if !buf.is_empty() && id.is_none() {
                    id = Some(buf);
                }
            }
            Mode::Class => {
                if !buf.is_empty() {
                    classes.push(buf);
                }
            }
        }

        Selector {
            tag,
            id,
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
        Token::LeftBrace => "{".into(),
        Token::RightBrace => "}".into(),
        Token::LeftBracket => "[".into(),
        Token::RightBracket => "]".into(),
        Token::Function { name, value } => format!("{name}({value:?})"),
        Token::AtKeyword(name) => format!("@{name}"),
        Token::CDO => "<!--".into(),
        Token::CDC => "-->".into(),
        Token::Hash(h) => format!("#{h}"),
        Token::Delim(c) => c.to_string(),
        Token::Whitespace => " ".into(),
        Token::Comment(_) => String::new(),
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
        assert_eq!(tree.root.borrow().children().len(), 1);
    }

    #[test]
    fn parse_at_rule() {
        let css = "@media screen { body { margin: 10px; } }";
        let mut parser = Parser::new(css);
        let tree = parser.parse().unwrap();
        assert_eq!(tree.root.borrow().children().len(), 1);
    }

    #[test]
    fn parse_multiple_rules() {
        let css = "body{background:#eee;width:60vw;margin:15vh auto;font-family:system-ui,sans-serif}h1{font-size:1.5em}";
        let mut parser = Parser::new(css);
        let tree = parser.parse().unwrap();
        assert_eq!(tree.root.borrow().children().len(), 2);
    }
}
