use crate::engine::css::cssom::tokenizer::{Token, Tokenizer};
use crate::engine::css::values::*;
use crate::engine::tree::{Tree, TreeNode};
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
}

impl<'a> Parser<'a> {
    pub fn new(input: &'a str) -> Self {
        let tree = Tree::new(CssNodeType::Stylesheet);
        Self {
            tokenizer: Tokenizer::new(input),
            tree: tree.clone(),
            stack: vec![tree.root.clone()],
            selector_buffer: String::new(),
        }
    }

    pub fn parse(&mut self) -> Tree<CssNodeType> {
        while let Some(token) = self.tokenizer.next_token() {
            match token {
                Token::Ident(selector) => {
                    self.selector_buffer.push_str(&selector);
                    self.parse_rule(self.selector_buffer.clone());
                    self.selector_buffer.clear();
                }
                Token::AtKeyword(key) => {
                    self.parse_at_rule(key);
                }
                Token::Delim('.') | Token::Delim('#') | Token::Hash(_) => {
                    self.selector_buffer.push_str(&token_to_string(&token));
                }
                Token::Whitespace => continue,
                _ => {}
            }
        }
        self.tree.clone()
    }

    fn parse_rule(&mut self, first_selector: String) {
        println!("Parsing rule for selector: {}", first_selector);
        let mut selectors = vec![first_selector];

        while let Some(token) = self.tokenizer.next_token() {
            match token {
                Token::Colon => {
                    if let Some(Token::Ident(pseudo)) = self.tokenizer.next_token() {
                        selectors.push(':'.to_string());
                        selectors.push(pseudo);
                    }
                }
                Token::Comma => {
                    self.skip_whitespace();
                    if let Some(Token::Ident(next_sel)) = self.tokenizer.last_tokenized_token() {
                        selectors.push(next_sel.clone());
                    }
                }
                Token::Delim('.') => {
                    if let Some(Token::Ident(class_name)) = self.tokenizer.next_token() {
                        selectors.push('.'.to_string());
                        selectors.push(class_name);
                    }
                }
                Token::Whitespace => selectors.push(' '.to_string()),
                Token::Ident(name) => {
                    if !selectors.is_empty() && selectors.last() != Some(&" ".to_string()) {
                        selectors.push(' '.to_string());
                    }
                    selectors.push(name);
                }
                Token::LeftBrace => break,
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

                    let mut value = String::new();
                    while let Some(token) = self.tokenizer.next_token() {
                        match token {
                            Token::Semicolon | Token::RightBrace => break,
                            Token::Ident(s) => value.push_str(&s),
                            Token::Whitespace => value.push(' '),
                            Token::Number(n) => value.push_str(&n.to_string()),
                            Token::Hash(h) => {
                                value.push('#');
                                value.push_str(&h);
                            }
                            Token::Percentage(pct) => {
                                value.push_str(&pct.to_string());
                                value.push('%');
                            }
                            Token::Dimension(num, unit) => {
                                value.push_str(&num.to_string());
                                value.push_str(&unit);
                            }
                            Token::StringLiteral(s) => {
                                // font-family: 'Helvetica Neue'
                                if !value.is_empty() {
                                    value.push(' ');
                                }
                                value.push_str(&format!("'{}'", s));
                            }
                            Token::Comma => {
                                value.push_str(",");
                            }
                            _ => {}
                        }
                    }

                    let value = value.trim().to_string();
                    let parsed_value = self.parse_value(&value);

                    let decl_node = TreeNode::new(CssNodeType::Declaration {
                        name,
                        value: parsed_value,
                    });
                    TreeNode::add_child(self.stack.last().unwrap(), decl_node);
                }
                Some(Token::Whitespace) => continue,
                None => break,
                _ => continue,
            }
        }
    }

    fn parse_at_rule(&mut self, name: String) {
        let mut params = Vec::new();

        while let Some(token) = self.tokenizer.next_token() {
            match &token {
                Token::Semicolon => {
                    // セミコロンで終わる → 単一型 AtRule
                    let node = TreeNode::new(CssNodeType::AtRule { name, params });
                    TreeNode::add_child(self.stack.last().unwrap(), node);
                    return;
                }
                Token::LeftBrace => {
                    // ブロック型 → 中のルールをパース
                    let node = TreeNode::new(CssNodeType::AtRule { name, params });
                    TreeNode::add_child(self.stack.last().unwrap(), node.clone());
                    self.stack.push(node.clone());
                    self.skip_whitespace();
                    if let Some(selector) = self.collect_selector() {
                        self.parse_rule(selector);
                    } else {
                        panic!("Expected selector inside at-rule block");
                    }
                    self.stack.pop();
                    return;
                }
                Token::RightBrace => break, // 終了
                _ => {
                    // それ以外 → 条件文トークンを文字列化して貯める
                    params.push(token_to_string(&token));
                }
            }
        }
    }

    /// セレクタを収集するヘルパー関数
    fn collect_selector(&mut self) -> Option<String> {
        let mut selector = String::new();
        while let Some(token) = self.tokenizer.next_token() {
            match token {
                Token::Delim(c) if c == '.' || c == '#' => {
                    selector.push(c);
                    if let Some(Token::Ident(name)) = self.tokenizer.next_token() {
                        selector.push_str(&name);
                    }
                }
                Token::Ident(name) => {
                    if !selector.is_empty() && !selector.ends_with(' ') {
                        selector.push(' ');
                    }
                    selector.push_str(&name);
                }
                Token::Colon => {
                    selector.push(':');
                    if let Some(Token::Ident(pseudo)) = self.tokenizer.next_token() {
                        selector.push_str(&pseudo);
                    }
                }
                Token::Hash(s) => {
                    selector.push('#');
                    selector.push_str(&s);
                }
                _ => break,
            }
        }
        if selector.is_empty() {
            None
        } else {
            Some(selector.trim().to_string())
        }
    }

    fn expect_colon(&mut self) {
        match self.tokenizer.next_token() {
            Some(Token::Colon) => {}
            _ => panic!("Expected ':' after property name"),
        }
    }

    fn parse_value(&self, css_str: &String) -> CssValue {
        let css_str = css_str.trim();
        if let Some(length) = Length::from_css(css_str) {
            CssValue::Length(length)
        } else if let Some(color) = Color::from_hex(css_str) {
            CssValue::Color(color)
        } else if let Some(color) = Color::from_named(css_str) {
            CssValue::Color(color)
        } else {
            CssValue::Keyword(css_str.to_string())
        }
    }

    /// 空白トークンをスキップするヘルパー関数
    fn skip_whitespace(&mut self) {
        while let Some(Token::Whitespace) = self.tokenizer.next_token() {}
    }
}

/// トークンを文字列化するヘルパー関数
fn token_to_string(token: &Token) -> String {
    match token {
        Token::Ident(s) => s.clone(),
        Token::StringLiteral(s) => format!("\"{}\"", s),
        Token::Number(n) => n.to_string(),
        Token::Dimension(n, unit) => format!("{}{}", n, unit),
        Token::Percentage(n) => format!("{}%", n),
        Token::Colon => ":".into(),
        Token::Comma => ",".into(),
        Token::LeftParen => "(".into(),
        Token::RightParen => ")".into(),
        Token::Whitespace => " ".into(),
        Token::Hash(h) => format!("#{}", h),
        Token::Delim(c) => c.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::css::values::Color;

    #[test]
    fn test_font_family() {
        let css = r#"
            body {
                font-family: 'Helvetica Neue', sans-serif;
            }
        "#;

        let mut parser = Parser::new(css);
        let tree = parser.parse();

        let root_ref = tree.root.borrow();
        assert!(matches!(root_ref.value, CssNodeType::Stylesheet));

        let rule_rc = Rc::clone(&root_ref.children[0]);
        drop(root_ref);

        let rule_ref = rule_rc.borrow();
        if let CssNodeType::Rule { selectors } = &rule_ref.value {
            assert_eq!(selectors, &vec!["body".to_string()]);
        } else {
            panic!("Expected Rule node");
        }

        let decl_rc = Rc::clone(&rule_ref.children[0]);
        drop(rule_ref);

        let decl_ref = decl_rc.borrow();
        if let CssNodeType::Declaration { name, value } = &decl_ref.value {
            assert_eq!(name, "font-family");
            if let CssValue::Keyword(s) = value {
                assert_eq!(s, "'Helvetica Neue', sans-serif");
            } else {
                panic!("Expected Keyword value");
            }
        }
    }

    #[test]
    fn test_color_and_length() {
        let css = r#"
            h1 {
                color: #00ff00;
                margin: 12px;
            }
        "#;
        let mut parser = Parser::new(css);
        let tree = parser.parse();

        let root_ref = tree.root.borrow();
        let rule_rc = Rc::clone(&root_ref.children[0]);
        drop(root_ref);

        let rule_ref = rule_rc.borrow();
        assert_eq!(rule_ref.children.len(), 2);

        let decl1_rc = Rc::clone(&rule_ref.children[0]);
        let decl1 = decl1_rc.borrow();
        if let CssNodeType::Declaration { name, value } = &decl1.value {
            assert_eq!(name, "color");
            if let CssValue::Color(c) = value {
                assert_eq!(c, &Color::from_hex("00ff00").unwrap());
            }
        }

        let decl2_rc = Rc::clone(&rule_ref.children[1]);
        let decl2 = decl2_rc.borrow();
        if let CssNodeType::Declaration { name, value } = &decl2.value {
            assert_eq!(name, "margin");
            if let CssValue::Length(Length::Px(px)) = value {
                assert_eq!(*px, 12.0);
            }
        }
    }
}
