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
use std::collections::VecDeque;
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

        params: AtQuery,
    },

    /// Declaration inside a rule block (e.g. `color: red`)
    Declaration {
        /// Property name
        name: String,

        value: CssValue,
    },
}

#[derive(Debug, Clone)]
pub enum AtQuery {
    Keyword(String), // screen, and, not
    Condition {
        name: String,    // max-width
        value: CssValue, // 600px
    },
    Group(Vec<AtQuery>), // ( ... )
}

/// Node in the CSS syntax tree.
///
/// Each node represents a syntactic construct such as a rule,
/// at-rule, or declaration, and may contain child nodes.
#[derive(Debug)]
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
    lookahead: VecDeque<Token>,
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
            lookahead: VecDeque::new(),
        }
    }

    fn ensure_lookahead(&mut self, n: usize) {
        while self.lookahead.len() <= n {
            let tok = self.tokenizer.next_token();
            self.lookahead.push_back(tok);
        }
    }

    fn peek_next_token(&mut self, cursor_size: usize) -> &Token {
        self.ensure_lookahead(cursor_size);
        &self.lookahead[cursor_size]
    }

    /// Consume and return the next token.
    fn peek_token(&mut self) -> &Token {
        self.peek_next_token(0)
    }

    fn consume_token(&mut self) -> Token {
        if let Some(tok) = self.lookahead.pop_front() {
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
                Token::Whitespace | Token::Comment(_) => {
                    self.consume_token();
                }
                Token::AtKeyword(_) => {
                    let node = self
                        .parse_at_rule()
                        .map_err(|e| e.with_context("parse: failed to parse at-rule"))?;
                    log::debug!(target: "CssParser", "AtRule parsed: {:?}", &node);
                    stylesheet.children.push(node);
                }
                _ => {
                    let node = self
                        .parse_rule()
                        .map_err(|e| e.with_context("parse: failed to parse rule"))?;
                    log::debug!(target: "CssParser", "Rule parsed: {:?}", &node);
                    stylesheet.children.push(node);
                }
            }
        }

        Ok(stylesheet)
    }

    fn parse_at_rule(&mut self) -> ParseResult<CssNode> {
        // 1. consume '@' token
        let at_name = if let Token::AtKeyword(name) = self.consume_token() {
            name
        } else {
            return Err(ParserError {
                kind: ParserErrorKind::UnexpectedToken {
                    expected: "@keyword",
                    found: format!("{:?}", self.peek_token()),
                },
                context: vec![],
            });
        };

        // 2. Collect prelude tokens (until '{' or ';'), handling nested parentheses
        let mut prelude = vec![];
        let mut paren_depth = 0;

        loop {
            match self.peek_token() {
                Token::Delim('{') if paren_depth == 0 => break,
                Token::Delim(';') if paren_depth == 0 => break,
                Token::Delim('(') => {
                    paren_depth += 1;
                    prelude.push(self.consume_token());
                }
                Token::Delim(')') => {
                    paren_depth -= 1;
                    prelude.push(self.consume_token());
                }
                Token::EOF => break,
                _ => prelude.push(self.consume_token()),
            }
        }

        // 3. Convert prelude tokens to CssValue (handles functions and nested parentheses)
        let params = Self::parse_at_query(prelude).map_err(|e| {
            e.with_context("parse_at_rule: failed to parse params via parse_at_query")
        })?;

        // 4. Block vs semicolon
        let children = if self.peek_token() == &Token::Delim('{') {
            self.consume_token();
            self.brace_depth += 1;

            let mut children = vec![];
            while self.peek_token() != &Token::Delim('}') {
                match self.peek_token() {
                    Token::EOF => {
                        return Err(ParserError {
                            kind: ParserErrorKind::UnexpectedEOF,
                            context: vec![],
                        });
                    }
                    Token::Whitespace => {
                        self.consume_token();
                    }
                    Token::AtKeyword(_) => {
                        let node = self.parse_at_rule().map_err(|e| {
                            e.with_context("parse_at_rule: failed to parse nested at-rule")
                        })?;
                        children.push(node);
                    }
                    _ => {
                        let mut cursor = 0;
                        let mut is_declaration = false;

                        loop {
                            match self.peek_next_token(cursor) {
                                Token::Delim('{') => {
                                    break;
                                }
                                Token::Delim('}') => {
                                    is_declaration = true;
                                    break;
                                }
                                Token::EOF => {
                                    return Err(ParserError {
                                        kind: ParserErrorKind::UnexpectedEOF,
                                        context: vec![],
                                    });
                                }
                                _ => {}
                            }
                            cursor += 1;
                        }

                        let nodes = if is_declaration {
                            self.parse_declaration_list().map_err(|e| {
                                e.with_context(
                                    "parse_at_rule: failed to parse declaration in block",
                                )
                            })?
                        } else {
                            vec![self.parse_rule().map_err(|e| {
                                e.with_context("parse_at_rule: failed to parse rule in block")
                            })?]
                        };

                        children.extend(nodes);
                    }
                }
            }

            self.consume_token(); // consume '}'
            self.brace_depth -= 1;
            children
        } else {
            if self.consume_token() != Token::Delim(';') {
                return Err(ParserError {
                    kind: ParserErrorKind::UnexpectedToken {
                        expected: ";",
                        found: format!("{:?}", self.peek_token()),
                    },
                    context: vec![],
                });
            }
            vec![]
        };

        Ok(CssNode {
            node: CssNodeType::AtRule {
                name: at_name,
                params,
            },
            children,
        })
    }

    fn parse_at_query(tokens: Vec<Token>) -> ParseResult<AtQuery> {
        let mut cursor = 0;
        let items = Self::parse_at_query_list(&tokens, &mut cursor)?;
        Ok(AtQuery::Group(items))
    }

    fn parse_at_query_list(tokens: &[Token], cursor: &mut usize) -> ParseResult<Vec<AtQuery>> {
        let mut items = Vec::new();

        while *cursor < tokens.len() {
            match &tokens[*cursor] {
                Token::Whitespace => {
                    *cursor += 1;
                }

                Token::Delim('(') => {
                    *cursor += 1;
                    let group = Self::parse_at_query_list(tokens, cursor)?;
                    items.push(AtQuery::Group(group));
                }

                Token::Delim(')') => {
                    *cursor += 1;
                    break;
                }

                Token::Ident(_) => {
                    items.push(Self::parse_at_query_item(tokens, cursor)?);
                }

                _ => {
                    *cursor += 1;
                }
            }
        }

        Ok(items)
    }

    fn parse_at_query_item(tokens: &[Token], cursor: &mut usize) -> ParseResult<AtQuery> {
        let name = match &tokens[*cursor] {
            Token::Ident(s) => s.clone(),
            _ => unreachable!(),
        };
        *cursor += 1;

        if matches!(tokens.get(*cursor), Some(Token::Delim(':'))) {
            *cursor += 1;
            let value = Self::parse_at_query_value(tokens, cursor)?;
            Ok(AtQuery::Condition { name, value })
        } else {
            Ok(AtQuery::Keyword(name))
        }
    }

    fn parse_at_query_value(tokens: &[Token], cursor: &mut usize) -> ParseResult<CssValue> {
        let mut buf = Vec::new();
        let mut paren_depth = 0;

        while *cursor < tokens.len() {
            match &tokens[*cursor] {
                Token::Delim('(') => {
                    paren_depth += 1;
                    buf.push(tokens[*cursor].clone());
                    *cursor += 1;
                }
                Token::Delim(')') if paren_depth == 0 => break,
                Token::Delim(')') => {
                    paren_depth -= 1;
                    buf.push(tokens[*cursor].clone());
                    *cursor += 1;
                }
                _ => {
                    buf.push(tokens[*cursor].clone());
                    *cursor += 1;
                }
            }
        }

        Self::parse_tokens_to_css_value(buf)
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
        let mut parts = vec![];

        let mut current_selector: Option<Selector> = None;
        let mut current_combinator: Option<Combinator> = None;

        loop {
            let token = self.peek_token().clone();
            match token {
                Token::Ident(name) => {
                    let sel = current_selector.get_or_insert_with(|| Selector {
                        tag: None,
                        id: None,
                        classes: vec![],
                        pseudo_class: None,
                        pseudo_element: None,
                    });

                    if sel.tag.is_none() {
                        sel.tag = Some(name);
                    }

                    self.consume_token();
                }

                Token::Hash(id) => {
                    let sel = current_selector.get_or_insert_with(|| Selector {
                        tag: None,
                        id: None,
                        classes: vec![],
                        pseudo_class: None,
                        pseudo_element: None,
                    });
                    sel.id = Some(id);
                    self.consume_token();
                }

                Token::Delim('.') => {
                    self.consume_token();
                    if let Token::Ident(class) = self.consume_token() {
                        let sel = current_selector.get_or_insert_with(|| Selector {
                            tag: None,
                            id: None,
                            classes: vec![],
                            pseudo_class: None,
                            pseudo_element: None,
                        });
                        sel.classes.push(class);
                    }
                }

                Token::Delim(':') => {
                    self.consume_token();
                    if self.peek_token() == &Token::Delim(':') {
                        // pseudo-element
                        self.consume_token();
                        if let Token::Ident(name) = self.consume_token() {
                            let sel = current_selector.get_or_insert_with(|| Selector {
                                tag: None,
                                id: None,
                                classes: vec![],
                                pseudo_class: None,
                                pseudo_element: None,
                            });
                            sel.pseudo_element = Some(name);
                        }
                    } else if let Token::Ident(name) = self.consume_token() {
                        let sel = current_selector.get_or_insert_with(|| Selector {
                            tag: None,
                            id: None,
                            classes: vec![],
                            pseudo_class: None,
                            pseudo_element: None,
                        });
                        sel.pseudo_class = Some(name);
                    }
                }

                Token::Whitespace | Token::Comment(_) => {
                    // descendant combinator
                    if let Some(sel) = current_selector.take() {
                        parts.push(SelectorPart {
                            selector: sel,
                            combinator: current_combinator.take(),
                        });
                    }
                    current_combinator = Some(Combinator::Descendant);
                    self.consume_token();
                }

                Token::Delim(',') => {
                    if let Some(sel) = current_selector.take() {
                        parts.push(SelectorPart {
                            selector: sel,
                            combinator: current_combinator.take(),
                        });
                    }
                    parts.reverse();
                    selectors.push(ComplexSelector {
                        parts: parts.clone(),
                    });
                    parts.clear();
                    current_combinator = None;
                    self.consume_token();

                    while matches!(self.peek_token(), Token::Whitespace | Token::Comment(_)) {
                        self.consume_token();
                    }
                }

                Token::Delim('{') | Token::EOF => {
                    if let Some(sel) = current_selector.take() {
                        parts.push(SelectorPart {
                            selector: sel,
                            combinator: current_combinator.take(),
                        });
                    }
                    if !parts.is_empty() {
                        parts.reverse();
                        selectors.push(ComplexSelector { parts });
                    }
                    break;
                }

                _ => {
                    self.consume_token();
                }
            }
        }

        selectors
    }

    /// Parse declaration until `Token::Delim('}')`.
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
                    if !parsing_name && !name.is_empty() {
                        declarations.push(CssNode {
                            node: CssNodeType::Declaration {
                                name: std::mem::take(&mut name),
                                value: Self::parse_tokens_to_css_value(std::mem::take(
                                    &mut value_tokens,
                                ))?,
                            },
                            children: vec![],
                        });
                    }
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
        let mut iter = tokens.into_iter().peekable();

        while let Some(token) = iter.next() {
            log::debug!(target: "CssParser", "parse_tokens_to_css_value: token={:?}", token);

            match token {
                Token::Ident(s) => values.push(CssValue::Keyword(s)),

                Token::Delim(',') => {
                    // List separator
                    continue;
                }

                Token::Delim('(') | Token::Delim(')') => {
                    // Function の構文用なので無視
                    continue;
                }

                Token::Delim(c) => {
                    values.push(CssValue::Keyword(c.to_string()));
                }

                Token::Number(n) => values.push(CssValue::Number(n)),

                Token::String(s) => values.push(CssValue::String(s)),

                Token::Dimension(value, unit) => {
                    let unit = match unit.as_str() {
                        "px" => Unit::Px,
                        "em" => Unit::Em,
                        "rem" => Unit::Rem,
                        "%" => Unit::Percent,
                        "vw" => Unit::Vw,
                        "vh" => Unit::Vh,
                        _ => Unit::Px,
                    };
                    values.push(CssValue::Length(value, unit));
                }

                Token::Hash(s) => values.push(CssValue::Color(s)),

                Token::Function(name) => {
                    // () の中をそのまま集める
                    let mut depth = 0;
                    let mut func_tokens = vec![];

                    for tok in iter.by_ref() {
                        match &tok {
                            Token::Delim('(') => {
                                depth += 1;
                                func_tokens.push(tok);
                            }
                            Token::Delim(')') => {
                                func_tokens.push(tok);
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => func_tokens.push(tok),
                        }
                    }

                    let arg_value = Self::parse_tokens_to_css_value(func_tokens)
                        .map_err(|e| e.with_context("parse function args"))?;

                    let args = match arg_value {
                        CssValue::List(list) => list,
                        other => vec![other],
                    };

                    values.push(CssValue::Function(name, args));
                }

                _ => continue,
            }
        }

        // 複数値なら List、単数ならそのまま
        Ok(match values.len() {
            0 => CssValue::Keyword(String::new()),
            1 => values.remove(0),
            _ => CssValue::List(values),
        })
    }
}

// ====================
impl fmt::Display for CssNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_tree_node(self, f, &[])
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
