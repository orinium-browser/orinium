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
                Token::Ident(selector) => self.parse_rule(selector),
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
                    self.skip_whitespace();
                    if let Some(Token::Ident(next_sel)) = self.tokenizer.last_tokenized_token() {
                        selectors.push(next_sel.clone());
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

    fn skip_whitespace(&mut self) {
        while let Some(Token::Whitespace) = self.tokenizer.next_token() {}
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
