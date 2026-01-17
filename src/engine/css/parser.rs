//! CSS Parser
//!
//! Consumes tokens produced by the tokenizer and builds
//! higher-level CSS syntax structures.
//!
//! ## Responsibilities
//! - Parse token streams into structured CSS data
//!   (selectors, declarations, component values)
//! - Handle nesting such as blocks and functions
//!
//! ## Non-responsibilities
//! - Tokenization of raw input
//! - Semantic interpretation (length resolution, color computation, etc.)
//!
//! ## Design notes
//! - No property-specific validation is performed here
//! - Semantic meaning is assigned in later stages (style computation, layout)
use std::fmt;

use super::tokenizer::{Token, Tokenizer};
use super::values::{CssValue, Unit};

/// Node kinds used in the CSS syntax tree.
///
/// These nodes represent **syntactic structure only**.
/// No semantic validation or value resolution is performed here.
#[derive(Debug, Clone)]
pub enum CssNodeType {
    /// Root node of a CSS document
    Stylesheet,

    /// Qualified rule (e.g. `div { ... }`)
    Rule {
        /// Selectors associated with this rule
        selectors: Vec<ComplexSelector>,
    },

    /// At-rule (e.g. `@media`, `@supports`)
    AtRule {
        /// At-rule name without `@`
        name: String,

        /// Raw parameter tokens
        params: Vec<Token>,
    },

    /// Declaration inside a rule block (e.g. `color: red`)
    Declaration {
        /// Property name
        name: String,

        /// Raw value token (not yet interpreted)
        value: CssValue,
    },
}

/// Node in the CSS syntax tree.
///
/// Each node represents a syntactic construct such as a rule,
/// at-rule, or declaration, and may contain child nodes.
pub struct CssNode {
    /// Kind of this CSS node
    node: CssNodeType,

    /// Child nodes forming the tree structure
    children: Vec<CssNode>,
}

impl CssNode {
    pub fn node(&self) -> &CssNodeType {
        &self.node
    }
    pub fn children(&self) -> &Vec<CssNode> {
        &self.children
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Selector {
    /// Type selector (e.g. `div`)
    ///
    /// `None` represents the absence of a type selector
    /// (e.g. `.class`, `#id`).
    pub tag: Option<String>,

    /// ID selector (e.g. `#main`)
    pub id: Option<String>,

    /// Class selectors (e.g. `.container`)
    pub classes: Vec<String>,

    /// Pseudo-class (e.g. `:hover`)
    pub pseudo_class: Option<String>,

    /// Pseudo-element (e.g. `::before`)
    pub pseudo_element: Option<String>,
}

/// Combinator defining the relationship between selectors.
///
/// Additional combinators (`>`, `+`, `~`) may be added later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Combinator {
    /// Descendant combinator (` `)
    Descendant,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SelectorPart {
    /// Simple selector matched at this step
    pub selector: Selector,

    /// Relationship to the next selector on the left.
    ///
    /// `None` indicates this is the leftmost selector
    /// in the selector sequence.
    pub combinator: Option<Combinator>,
}

/// A complex CSS selector composed of multiple selector parts.
///
/// Selector parts are stored **from right to left** to match
/// the order used during selector matching.
///
/// Example:
/// ```text
/// A B
/// ```
/// is stored as:
/// ```text
/// [
///   B (Descendant),
///   A (None)
/// ]
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ComplexSelector {
    pub parts: Vec<SelectorPart>,
}

/// CSS parser consuming tokens and producing syntax structures.
pub struct Parser<'a> {
    /// Source of tokens produced by the tokenizer
    tokenizer: Tokenizer<'a>,

    /// Used to detect the start and end of rule blocks (`{}`).
    brace_depth: usize,

    /// Lookahead token (optional)
    ///
    /// Parser may need to peek the next token without consuming it.
    lookahead: Option<Token>,
}

/// Parser error kinds
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParserErrorKind {
    /// Expected a token but found something else
    UnexpectedToken {
        expected: &'static str,
        found: String, // Token debug or value
    },

    /// Unexpected end of file
    UnexpectedEOF,

    /// Invalid or unsupported CSS syntax
    InvalidSyntax,

    /// Mismatched braces or parentheses
    MismatchedDelimiter { expected: char, found: char },
}

impl fmt::Display for ParserErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// Parser error
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParserError {
    /// Kind of the error
    pub kind: ParserErrorKind,
    /// Context
    pub context: Vec<String>,
}

impl ParserError {
    pub fn with_context(mut self, ctx: impl Into<String>) -> Self {
        self.context.push(ctx.into());
        self
    }
}

impl fmt::Display for ParserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut ctx = self.context.clone();
        ctx.reverse();
        write!(
            f,
            "CssParserError: {}, (Context:[{}])",
            self.kind,
            ctx.join(" <-")
        )
    }
}

impl std::error::Error for ParserError {}

/// Result type for parser functions
pub type ParseResult<T> = Result<T, ParserError>;

impl<'a> Parser<'a> {
    /// Create a new CSS parser from a source string.
    pub fn new(input: &'a str) -> Self {
        Self {
            tokenizer: Tokenizer::new(input),
            brace_depth: 0,
            lookahead: None,
        }
    }

    /// Peek at the next token without consuming it.
    fn peek_token(&mut self) -> &Token {
        if self.lookahead.is_none() {
            self.lookahead = Some(self.tokenizer.next_token());
        }
        self.lookahead.as_ref().unwrap()
    }

    /// Consume and return the next token.
    fn consume_token(&mut self) -> Token {
        if let Some(tok) = self.lookahead.take() {
            tok
        } else {
            self.tokenizer.next_token()
        }
    }

    /// Parse the entire CSS source into a syntax tree.
    ///
    /// This method consumes tokens until `Token::EOF` is reached and constructs
    /// a `CssNode` representing the stylesheet root.
    ///
    /// Parsing behavior:
    /// - Whitespace tokens are ignored
    /// - Qualified rules and at-rules are parsed into child nodes
    /// - No semantic validation is performed
    pub fn parse(&mut self) -> ParseResult<CssNode> {
        let mut stylesheet = CssNode {
            node: CssNodeType::Stylesheet,
            children: vec![],
        };

        loop {
            let token = self.peek_token().clone();

            match token {
                Token::EOF => break,
                Token::Whitespace => {
                    self.consume_token(); // ignore whitespace
                }
                _ => {
                    // Determine rule or at-rule
                    let node = self
                        .parse_rule()
                        .map_err(|e| e.with_context("parse: failed to parse rule"))?; // placeholder
                    stylesheet.children.push(node);
                }
            }
        }

        Ok(stylesheet)
    }

    /// Parse a qualified rule (e.g., `div { color: red; }`).
    ///
    /// Parses the selector list first, then the block of declarations.
    fn parse_rule(&mut self) -> ParseResult<CssNode> {
        // 1. Parse selectors
        let selectors = self.parse_selector_list();

        // 2. Expect `{`
        match self.consume_token() {
            Token::Delim('{') => self.brace_depth += 1,
            token => {
                return Err(ParserError {
                    kind: ParserErrorKind::UnexpectedToken {
                        expected: "{",
                        found: format!("{:?}", token),
                    },
                    context: vec![],
                });
            }
        }

        // 3. Parse declarations inside the block
        let mut children = vec![];
        loop {
            let token = self.peek_token().clone();
            match token {
                Token::Delim('}') => {
                    self.consume_token();
                    self.brace_depth -= 1;
                    break;
                }
                Token::EOF => {
                    return Err(ParserError {
                        kind: ParserErrorKind::UnexpectedEOF,
                        context: vec![],
                    });
                }
                _ => {
                    let mut decls = self.parse_declaration_list().map_err(|e| {
                        e.with_context("parse_rule: failed to parse declaration list")
                    })?;
                    children.append(&mut decls);
                }
            }
        }

        Ok(CssNode {
            node: CssNodeType::Rule { selectors },
            children,
        })
    }

    /// Parse a comma-separated list of selectors for a rule.
    ///
    /// Each selector is represented as a `ComplexSelector`.
    fn parse_selector_list(&mut self) -> Vec<ComplexSelector> {
        let mut selectors = vec![];
        let mut current_parts = vec![];
        let mut current_combinator = None;

        loop {
            let token = self.peek_token().clone();
            match token {
                Token::Ident(s) => {
                    let selector = Selector {
                        tag: Some(s),
                        id: None,
                        classes: vec![],
                        pseudo_class: None,
                        pseudo_element: None,
                    };
                    current_parts.push(SelectorPart {
                        selector,
                        combinator: current_combinator,
                    });
                    current_combinator = None;
                    self.consume_token();
                }

                Token::Hash(id_name) => {
                    if let Some(part) = current_parts.last_mut() {
                        part.selector.id = Some(id_name);
                    } else {
                        let selector = Selector {
                            tag: None,
                            id: Some(id_name),
                            classes: vec![],
                            pseudo_class: None,
                            pseudo_element: None,
                        };
                        current_parts.push(SelectorPart {
                            selector,
                            combinator: current_combinator,
                        });
                        current_combinator = None;
                    }
                    self.consume_token();
                }

                Token::Delim('.') => {
                    self.consume_token();
                    if let Token::Ident(class_name) = self.peek_token().clone() {
                        if let Some(part) = current_parts.last_mut() {
                            part.selector.classes.push(class_name);
                        } else {
                            let selector = Selector {
                                tag: None,
                                id: None,
                                classes: vec![class_name],
                                pseudo_class: None,
                                pseudo_element: None,
                            };
                            current_parts.push(SelectorPart {
                                selector,
                                combinator: current_combinator,
                            });
                        }
                        self.consume_token();
                    }
                }

                Token::Delim(',') => {
                    self.consume_token();
                    if !current_parts.is_empty() {
                        selectors.push(ComplexSelector {
                            parts: current_parts.clone(),
                        });
                        current_parts.clear();
                    }
                }

                Token::Delim('{') | Token::EOF => {
                    if !current_parts.is_empty() {
                        selectors.push(ComplexSelector {
                            parts: current_parts.clone(),
                        });
                    }
                    break;
                }

                Token::Whitespace => {
                    current_combinator = Some(Combinator::Descendant);
                    self.consume_token();
                }

                _ => {
                    self.consume_token(); // placeholder for unsupported token
                }
            }
        }

        selectors
    }

    fn parse_declaration_list(&mut self) -> ParseResult<Vec<CssNode>> {
        let mut declarations = vec![];
        let mut parsing_name = true;
        let mut name = String::new();
        let mut value_tokens = vec![];

        loop {
            let token = self.peek_token().clone();
            match token {
                Token::Delim(':') if parsing_name => {
                    parsing_name = false;
                    self.consume_token();
                }
                Token::Delim(';') if !parsing_name => {
                    self.consume_token(); // consume ;
                    declarations.push(CssNode {
                        node: CssNodeType::Declaration {
                            name: std::mem::take(&mut name),
                            value: Self::parse_tokens_to_css_value(std::mem::take(
                                &mut value_tokens,
                            ))
                            .map_err(|e| {
                                e.with_context(
                                    "parse_declaration: failed to parse declaration value list",
                                )
                            })?,
                        },
                        children: vec![],
                    });
                    parsing_name = true;
                }
                Token::Delim('}') | Token::EOF => {
                    // Stop parsing declarations, do not consume `}` here
                    break;
                }
                Token::Ident(s) if parsing_name => {
                    name.push_str(&s);
                    self.consume_token();
                }
                _ => {
                    if !parsing_name {
                        value_tokens.push(self.consume_token());
                    } else {
                        self.consume_token(); // skip unsupported token in name
                    }
                }
            }
        }

        Ok(declarations)
    }

    fn parse_tokens_to_css_value(tokens: Vec<Token>) -> ParseResult<CssValue> {
        let mut values = vec![];

        let mut find_function = false;
        let mut parsing_function = false;

        let mut function_buffer: (String, Vec<CssValue>) = (String::new(), vec![]);

        for token in tokens {
            let css_value = if find_function {
                if matches!(token, Token::Delim('(')) {
                    parsing_function = true;
                    find_function = false;
                    continue;
                } else {
                    return Err(ParserError {
                        kind: ParserErrorKind::UnexpectedToken {
                            expected: "(",
                            found: format!("{:?}", token),
                        },
                        context: vec![],
                    });
                }
            } else if parsing_function {
                match token {
                    Token::Delim(')') => {
                        let (name, args) = std::mem::take(&mut function_buffer);
                        CssValue::Function(name, args)
                    }
                    _ => {
                        function_buffer.1.push(
                            Self::parse_tokens_to_css_value(vec![token]).map_err(|e| {
                                e.with_context(
                                    "parse_to_css_value: failed to parse tokens to css value",
                                )
                            })?,
                        );
                        continue;
                    }
                }
            } else {
                match token {
                    Token::Ident(s) => CssValue::Keyword(s),
                    Token::Number(n) => CssValue::Number(n),
                    Token::Dimension(value, unit) => {
                        let unit = match unit.as_str() {
                            "px" => Unit::Px,
                            "em" => Unit::Em,
                            "rem" => Unit::Rem,
                            "%" => Unit::Percent,
                            "vw" => Unit::Vw,
                            "vh" => Unit::Vh,
                            _ => Unit::Px, // fallback
                        };
                        CssValue::Length(value, unit)
                    }
                    Token::Function(name) => {
                        function_buffer.0 = name;
                        find_function = true;
                        continue;
                    }
                    Token::Hash(s) => CssValue::Color(s),
                    _ => continue,
                }
            };
            values.push(css_value);
        }

        if values.len() == 1 {
            Ok(values.into_iter().next().unwrap())
        } else {
            Ok(CssValue::List(values))
        }
    }
}

// ====================
impl fmt::Display for CssNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_tree_node(&self, f, &[])
    }
}

/// 再帰的にツリーを表示するヘルパー関数
fn fmt_tree_node(
    node: &CssNode,
    f: &mut fmt::Formatter<'_>,
    ancestors_last: &[bool],
) -> fmt::Result {
    let is_last = *ancestors_last.last().unwrap_or(&true);
    let connector = if ancestors_last.is_empty() {
        ""
    } else if is_last {
        "└── "
    } else {
        "├── "
    };

    let mut prefix = String::new();
    for &ancestor_last in &ancestors_last[..ancestors_last.len().saturating_sub(1)] {
        prefix.push_str(if ancestor_last { "    " } else { "│   " });
    }

    writeln!(f, "{}{}{:?}", prefix, connector, node.node())?;

    let child_count = node.children().len();
    for (i, child) in node.children().iter().enumerate() {
        let child_is_last = i == child_count - 1;
        let mut new_ancestors = ancestors_last.to_vec();
        new_ancestors.push(child_is_last);
        fmt_tree_node(child, f, &new_ancestors)?;
    }

    Ok(())
}
