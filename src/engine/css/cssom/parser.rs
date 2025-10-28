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
                Token::LeftBrace => {
                    let selector = self.selector_buffer.clone();
                    println!("Found selector start: {}", selector);
                    self.parse_rule(selector);
                    self.selector_buffer.clear();
                    println!("Completed parsing rule.");
                }
                Token::AtKeyword(key) => {
                    self.parse_at_rule(key);
                }
                Token::Delim(_) | Token::Hash(_) | Token::Ident(_) | Token::Comma => {
                    self.selector_buffer.push_str(&token_to_string(&token));
                }
                Token::Whitespace => {
                    if self.selector_buffer.is_empty() {
                        continue;
                    } else {
                        self.selector_buffer.push(' ');
                    }
                }
                _ => {}
            }
        }
        self.tree.clone()
    }

    /// セレクタを収集するヘルパー関数
    fn collect_selector(&mut self) -> Option<String> {
        let mut selector = String::new();

        let mut token = self.tokenizer.last_tokenized_token().cloned();

        loop {
            let t = match token.take() {
                Some(t) => t, // 所有権をムーブ
                None => match self.tokenizer.next_token() {
                    Some(t2) => t2,
                    None => break,
                },
            };

            match t {
                Token::LeftBrace => break,
                Token::Delim(_) | Token::Hash(_) | Token::Ident(_) | Token::Comma => {
                    selector.push_str(&token_to_string(&t));
                }
                Token::Whitespace => {
                    if selector.is_empty() {
                        // skip
                    } else {
                        selector.push(' ');
                    }
                }
                _ => {}
            }

            token = self.tokenizer.next_token();
        }

        if selector.is_empty() {
            None
        } else {
            Some(selector.trim().to_string())
        }
    }

    fn parse_rule(&mut self, selectors: String) {
        println!("Parsing rule for selector: {}", selectors);
        let selectors: Vec<String> = selectors.split(',').map(|s| s.trim().to_string()).collect();

        let rule_node = TreeNode::new(CssNodeType::Rule {
            selectors: selectors.clone(),
        });
        TreeNode::add_child(self.stack.last().unwrap(), rule_node.clone());

        self.stack.push(rule_node.clone());
        self.parse_declarations();
        self.stack.pop();
    }

    fn parse_declarations(&mut self) {
        let mut retrun_flag = false;
        loop {
            match self.tokenizer.next_token() {
                Some(Token::RightBrace) => break,
                Some(Token::Ident(name)) => {
                    self.expect_colon();

                    let mut value = String::new();
                    while let Some(token) = self.tokenizer.next_token() {
                        match token {
                            Token::Semicolon => break,
                            Token::RightBrace => {
                                retrun_flag = true;
                                break;
                            }
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

                    if retrun_flag {
                        return;
                    }
                }
                Some(Token::Whitespace) => continue,
                None => break,
                _ => continue,
            }
            println!("Parsing declarations... Current token: {:?}", self.tokenizer.last_tokenized_token());
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
                    println!("Parsing at-rule block for: {}", name);
                    let node = TreeNode::new(CssNodeType::AtRule { name, params });
                    TreeNode::add_child(self.stack.last().unwrap(), node.clone());
                    self.stack.push(node.clone());
                    self.skip_whitespace();
                    println!("At-rule block started.");
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
