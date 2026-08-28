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

use crate::css::values::CssIdent;

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

#[derive(Debug, Clone, PartialEq)]
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
    /// Nesting selector (`&`).
    ///
    /// When `true` this selector represents the CSS nesting selector `&`
    /// and will be replaced with the parent rule's selector during nesting
    /// resolution. It may carry additional simple selectors (tag, classes,
    /// etc.) when it appears inside a compound selector such as `&.highlight`.
    pub is_nesting: bool,

    /// Type selector (e.g. `div`)
    ///
    /// `None` represents the absence of a type selector
    /// (e.g. `.class`, `#id`).
    pub tag: Option<String>,

    /// ID selector (e.g. `#main`)
    pub id: Option<String>,

    /// Class selectors (e.g. `.container`)
    pub classes: Vec<String>,

    /// Attribute selectors (e.g. `[hidden]`, `[type="text"]`)
    pub attributes: Vec<AttributeSelector>,

    /// Pseudo-classes (e.g. `:hover`, `:first-child`, `:not(.hidden)`)
    pub pseudo_classes: Vec<PseudoClass>,

    /// Pseudo-element (e.g. `::before`)
    pub pseudo_element: Option<String>,
}

impl Selector {
    /// Returns `true` when this selector carries fields beyond the nesting
    /// flag (i.e. it is a compound selector such as `&.highlight`).
    fn is_compound(&self) -> bool {
        self.tag.is_some()
            || self.id.is_some()
            || !self.classes.is_empty()
            || !self.attributes.is_empty()
            || !self.pseudo_classes.is_empty()
            || self.pseudo_element.is_some()
    }

    /// Merges the non-nesting fields of `other` into this selector.
    fn merge_from(&mut self, other: &Selector) {
        if let Some(tag) = &other.tag {
            self.tag = Some(tag.clone());
        }
        if let Some(id) = &other.id {
            self.id = Some(id.clone());
        }
        self.classes.extend(other.classes.iter().cloned());
        self.attributes.extend(other.attributes.iter().cloned());
        self.pseudo_classes
            .extend(other.pseudo_classes.iter().cloned());
        if let Some(pe) = &other.pseudo_element {
            self.pseudo_element = Some(pe.clone());
        }
    }
}

/// A pseudo-class attached to a simple selector.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PseudoClass {
    /// A non-functional pseudo-class such as `:first-child`.
    Simple(String),
    /// A selector-list pseudo-class such as `:is()` or `:not()`.
    SelectorList {
        /// Lower-level function name.
        name: String,
        /// Parsed selector arguments.
        selectors: Vec<ComplexSelector>,
    },
    /// A structural `An+B` pseudo-class such as `:nth-child(2n+1)`.
    Nth {
        /// Function name (`nth-child`, `nth-last-child`, etc.).
        name: String,
        /// Step coefficient in `An+B`.
        a: i32,
        /// Offset in `An+B`.
        b: i32,
    },
}

/// An attribute-presence or exact-value selector.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AttributeSelector {
    /// Attribute name to match.
    pub name: String,
    /// Required exact value, or `None` for a presence selector.
    pub value: Option<String>,
}

/// Combinator defining the relationship between selectors.
///
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Combinator {
    /// Descendant combinator (` `)
    Descendant,
    /// Child combinator (`>`)
    Child,
    /// Next-sibling combinator (`+`)
    NextSibling,
    /// Subsequent-sibling combinator (`~`)
    SubsequentSibling,
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

impl ComplexSelector {
    pub fn empty() -> Self {
        Self { parts: Vec::new() }
    }

    pub fn nest(&self, child: &Self) -> Self {
        if self.parts.is_empty() {
            return child.clone();
        }
        if child.parts.is_empty() {
            return self.clone();
        }

        let has_nesting = child.parts.iter().any(|p| p.selector.is_nesting);

        if !has_nesting {
            // No & in child – connect with a descendant combinator.
            let mut parts = child.parts.clone();
            parts
                .last_mut()
                .expect("child selector is known to be non-empty")
                .combinator = Some(Combinator::Descendant);
            parts.extend(self.parts.iter().cloned());
            return Self { parts };
        }

        // Resolve & nesting selector references.
        let mut result_parts: Vec<SelectorPart> = Vec::new();

        for child_part in &child.parts {
            if child_part.selector.is_nesting {
                if child_part.selector.is_compound() {
                    // Compound & (e.g. &.highlight) – merge parent subject
                    // fields into this selector and append remaining parent
                    // parts so the parent chain is preserved.
                    let mut merged = self.parts[0].selector.clone();
                    merged.merge_from(&child_part.selector);
                    merged.is_nesting = false;

                    let combinator = child_part.combinator.or(self.parts[0].combinator);

                    result_parts.push(SelectorPart {
                        selector: merged,
                        combinator,
                    });

                    for parent_part in self.parts.iter().skip(1) {
                        result_parts.push(parent_part.clone());
                    }
                } else {
                    // Standalone & – replace with the full parent selector
                    // chain.  The last (leftmost) parent part inherits the
                    // combinator that & carried.
                    let child_combinator = child_part.combinator;
                    let parent_len = self.parts.len();
                    for (i, parent_part) in self.parts.iter().enumerate() {
                        let mut part = parent_part.clone();
                        if i == parent_len - 1 {
                            part.combinator = child_combinator;
                        }
                        result_parts.push(part);
                    }
                }
            } else {
                result_parts.push(child_part.clone());
            }
        }

        Self {
            parts: result_parts,
        }
    }
}

/// Parse the integer `An+B` grammar used by structural pseudo-classes.
fn parse_an_plus_b(tokens: &[Token]) -> Option<(i32, i32)> {
    let mut expression = String::new();
    for token in tokens {
        match token {
            Token::Whitespace | Token::Comment(_) => {}
            Token::Ident(value) => expression.push_str(&value.to_ascii_lowercase()),
            Token::Number(value) if value.fract() == 0.0 => {
                expression.push_str(&(*value as i32).to_string());
            }
            Token::Dimension(value, unit) if value.fract() == 0.0 => {
                expression.push_str(&(*value as i32).to_string());
                expression.push_str(&unit.to_ascii_lowercase());
            }
            Token::Delim(value @ ('+' | '-')) => expression.push(*value),
            _ => return None,
        }
    }

    match expression.as_str() {
        "odd" => return Some((2, 1)),
        "even" => return Some((2, 0)),
        _ => {}
    }

    if let Some(n_index) = expression.find('n') {
        if expression[n_index + 1..].contains('n') {
            return None;
        }
        let coefficient = match &expression[..n_index] {
            "" | "+" => 1,
            "-" => -1,
            value => value.parse().ok()?,
        };
        let offset = match &expression[n_index + 1..] {
            "" => 0,
            value => value.parse().ok()?,
        };
        Some((coefficient, offset))
    } else {
        Some((0, expression.parse().ok()?))
    }
}

/// CSS parser consuming tokens and producing syntax structures.
#[derive(Clone)]
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

    /// Peek at the next token without consuming it.
    fn peek_token(&mut self) -> &Token {
        self.peek_next_token(0)
    }

    /// Parse a bare declaration list (e.g. the value of a `style` attribute).
    ///
    /// Unlike `parse()`, this does not expect selectors or a surrounding block.
    /// It consumes declarations until EOF or `}` and returns them as
    /// `Declaration` nodes, mirroring the body of a rule.
    pub fn parse_declarations(&mut self) -> ParseResult<Vec<CssNode>> {
        self.parse_declaration_and_nested_rule_list()
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
                    log::debug!(target: "CssParser", "AtRule parsed: {:?}", node);
                    stylesheet.children.push(node);
                }
                _ => {
                    let node = self
                        .parse_rule()
                        .map_err(|e| e.with_context("parse: failed to parse rule"))?;
                    log::debug!(target: "CssParser", "Rule parsed: {:?}", node);
                    stylesheet.children.push(node);
                }
            }
        }

        Ok(stylesheet)
    }

    /// Parses a stylesheet while recovering from unsupported top-level items.
    ///
    /// Browser stylesheets are frequently generated and may contain selectors
    /// or declarations that this engine does not support yet. Dropping the
    /// entire stylesheet for one such rule would also discard all compatible
    /// rules, so this mode skips the failing item and resumes at the next
    /// top-level boundary. The strict [`Self::parse`] entry point remains
    /// available for validation and unit tests.
    pub fn parse_lossy(&mut self) -> CssNode {
        let mut stylesheet = CssNode {
            node: CssNodeType::Stylesheet,
            children: vec![],
        };

        loop {
            let token = self.peek_token().clone();
            let checkpoint = self.clone();
            match token {
                Token::EOF => break,
                Token::Whitespace | Token::Comment(_) => {
                    self.consume_token();
                }
                Token::AtKeyword(_) => match self.parse_at_rule() {
                    Ok(node) => stylesheet.children.push(node),
                    Err(error) => {
                        log::warn!(
                            target: "CssParser",
                            "Skipping unsupported at-rule: {error}"
                        );
                        *self = checkpoint;
                        self.recover_top_level_item();
                    }
                },
                _ => match self.parse_rule() {
                    Ok(node) => stylesheet.children.push(node),
                    Err(error) => {
                        log::warn!(
                            target: "CssParser",
                            "Skipping unsupported CSS rule: {error}"
                        );
                        *self = checkpoint;
                        self.recover_top_level_item();
                    }
                },
            }
        }

        stylesheet
    }

    fn recover_top_level_item(&mut self) {
        let mut depth = 0_usize;
        let mut consumed_any = false;
        loop {
            match self.peek_token().clone() {
                Token::EOF => break,
                // An at-rule starts a new top-level item. Preserve it when an
                // invalid selector prefix (for example a concatenated BOM)
                // was the item that failed, instead of swallowing the whole
                // following @media block during recovery.
                Token::AtKeyword(_) if depth == 0 && consumed_any => break,
                Token::Delim('{') => {
                    depth += 1;
                    self.consume_token();
                    consumed_any = true;
                }
                Token::Delim('}') => {
                    self.consume_token();
                    if depth <= 1 {
                        break;
                    }
                    depth -= 1;
                    consumed_any = true;
                }
                Token::Delim(';') if depth == 0 => {
                    self.consume_token();
                    break;
                }
                _ => {
                    self.consume_token();
                    consumed_any = true;
                }
            }
        }
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
                            self.parse_declaration_and_nested_rule_list().map_err(|e| {
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

                Token::Delim(',') => {
                    items.push(AtQuery::Keyword(",".into()));
                    *cursor += 1;
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

        let mut colon = *cursor;
        while matches!(
            tokens.get(colon),
            Some(Token::Whitespace | Token::Comment(_))
        ) {
            colon += 1;
        }
        if matches!(tokens.get(colon), Some(Token::Delim(':'))) {
            *cursor = colon + 1;
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
                    context: vec![format!(
                        "While parsing rule with selectors: {}",
                        selectors
                            .iter()
                            .map(|s| format!("{:?}", s))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )],
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
                    let mut child = self.parse_declaration_and_nested_rule_list().map_err(|e| {
                        e.with_context("parse_rule: failed to parse declaration list")
                    })?;
                    children.append(&mut child);
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
        self.parse_selector_list_until(None)
    }

    /// Parse a selector list up to a rule block or a functional pseudo-class
    /// closing delimiter.
    fn parse_selector_list_until(&mut self, terminator: Option<char>) -> Vec<ComplexSelector> {
        let mut selectors = vec![];
        let mut parts = vec![];

        let mut current_selector: Option<Selector> = None;
        let mut current_combinator: Option<Combinator> = None;

        loop {
            let token = self.peek_token().clone();
            match token {
                Token::Ident(name) => {
                    let sel = current_selector.get_or_insert_with(|| Selector {
                        is_nesting: false,
                        tag: None,
                        id: None,
                        classes: vec![],
                        attributes: vec![],
                        pseudo_classes: vec![],
                        pseudo_element: None,
                    });

                    if sel.tag.is_none() {
                        sel.tag = Some(name);
                    }

                    self.consume_token();
                }

                Token::Hash(id) => {
                    let sel = current_selector.get_or_insert_with(|| Selector {
                        is_nesting: false,
                        tag: None,
                        id: None,
                        classes: vec![],
                        attributes: vec![],
                        pseudo_classes: vec![],
                        pseudo_element: None,
                    });
                    sel.id = Some(id);
                    self.consume_token();
                }

                Token::Delim('.') => {
                    self.consume_token();
                    if let Token::Ident(class) = self.consume_token() {
                        let sel = current_selector.get_or_insert_with(|| Selector {
                            is_nesting: false,
                            tag: None,
                            id: None,
                            classes: vec![],
                            attributes: vec![],
                            pseudo_classes: vec![],
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
                                is_nesting: false,
                                tag: None,
                                id: None,
                                classes: vec![],
                                attributes: vec![],
                                pseudo_classes: vec![],
                                pseudo_element: None,
                            });
                            sel.pseudo_element = Some(name);
                        }
                    } else {
                        let pseudo_class = match self.consume_token() {
                            Token::Ident(name) => Some(PseudoClass::Simple(name)),
                            Token::Function(name) => {
                                if self.peek_token() == &Token::Delim('(') {
                                    self.consume_token();
                                }
                                let lower_name = name.to_ascii_lowercase();
                                let pseudo = match lower_name.as_str() {
                                    "is" | "where" | "not" => PseudoClass::SelectorList {
                                        name: lower_name,
                                        selectors: self.parse_selector_list_until(Some(')')),
                                    },
                                    "nth-child" | "nth-last-child" | "nth-of-type"
                                    | "nth-last-of-type" => {
                                        let tokens = self.consume_until_closing_parenthesis();
                                        let (a, b) =
                                            parse_an_plus_b(&tokens).unwrap_or((0, i32::MIN));
                                        PseudoClass::Nth {
                                            name: lower_name,
                                            a,
                                            b,
                                        }
                                    }
                                    _ => {
                                        self.consume_until_closing_parenthesis();
                                        PseudoClass::SelectorList {
                                            name: lower_name,
                                            selectors: Vec::new(),
                                        }
                                    }
                                };
                                if self.peek_token() == &Token::Delim(')') {
                                    self.consume_token();
                                }
                                Some(pseudo)
                            }
                            _ => None,
                        };
                        if let Some(pseudo_class) = pseudo_class {
                            let sel = current_selector.get_or_insert_with(|| Selector {
                                is_nesting: false,
                                tag: None,
                                id: None,
                                classes: vec![],
                                attributes: vec![],
                                pseudo_classes: vec![],
                                pseudo_element: None,
                            });
                            sel.pseudo_classes.push(pseudo_class);
                        }
                    }
                }

                Token::Delim('[') => {
                    self.consume_token();
                    while matches!(self.peek_token(), Token::Whitespace | Token::Comment(_)) {
                        self.consume_token();
                    }

                    let name = match self.consume_token() {
                        Token::Ident(name) => name,
                        _ => continue,
                    };

                    while matches!(self.peek_token(), Token::Whitespace | Token::Comment(_)) {
                        self.consume_token();
                    }

                    let value = if self.peek_token() == &Token::Delim('=') {
                        self.consume_token();
                        while matches!(self.peek_token(), Token::Whitespace | Token::Comment(_)) {
                            self.consume_token();
                        }
                        match self.consume_token() {
                            Token::Ident(value) | Token::String(value) => Some(value),
                            _ => continue,
                        }
                    } else {
                        None
                    };

                    while matches!(self.peek_token(), Token::Whitespace | Token::Comment(_)) {
                        self.consume_token();
                    }
                    if self.peek_token() == &Token::Delim(']') {
                        self.consume_token();
                        let sel = current_selector.get_or_insert_with(|| Selector {
                            is_nesting: false,
                            tag: None,
                            id: None,
                            classes: vec![],
                            attributes: vec![],
                            pseudo_classes: vec![],
                            pseudo_element: None,
                        });
                        sel.attributes.push(AttributeSelector { name, value });
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
                    if current_combinator.is_none() {
                        current_combinator = Some(Combinator::Descendant);
                    }
                    self.consume_token();
                }

                Token::Delim('>') => {
                    if let Some(sel) = current_selector.take() {
                        parts.push(SelectorPart {
                            selector: sel,
                            combinator: current_combinator.take(),
                        });
                    }
                    current_combinator = Some(Combinator::Child);
                    self.consume_token();
                }

                Token::Delim('+') | Token::Delim('~') => {
                    if let Some(sel) = current_selector.take() {
                        parts.push(SelectorPart {
                            selector: sel,
                            combinator: current_combinator.take(),
                        });
                    }
                    current_combinator = Some(if token == Token::Delim('+') {
                        Combinator::NextSibling
                    } else {
                        Combinator::SubsequentSibling
                    });
                    self.consume_token();
                }

                Token::Delim('*') => {
                    current_selector.get_or_insert_with(|| Selector {
                        is_nesting: false,
                        tag: None,
                        id: None,
                        classes: vec![],
                        attributes: vec![],
                        pseudo_classes: vec![],
                        pseudo_element: None,
                    });
                    self.consume_token();
                }

                Token::Delim('&') => {
                    let sel = current_selector.get_or_insert_with(|| Selector {
                        is_nesting: false,
                        tag: None,
                        id: None,
                        classes: vec![],
                        attributes: vec![],
                        pseudo_classes: vec![],
                        pseudo_element: None,
                    });
                    sel.is_nesting = true;
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

                Token::Delim(')') if terminator == Some(')') => {
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

                // At-keywords cannot occur inside a qualified-rule prelude.
                // Leave the token untouched so parse_rule reports the invalid
                // prefix and lossy top-level recovery can resume at the at-rule.
                Token::AtKeyword(_) => break,

                _ => {
                    self.consume_token();
                }
            }
        }

        selectors
    }

    /// Consume tokens through the matching `)` of a functional pseudo-class.
    fn consume_until_closing_parenthesis(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut depth = 0;
        loop {
            match self.peek_token() {
                Token::EOF => break,
                Token::Delim(')') if depth == 0 => break,
                Token::Delim('(') => {
                    depth += 1;
                    tokens.push(self.consume_token());
                }
                Token::Delim(')') => {
                    depth -= 1;
                    tokens.push(self.consume_token());
                }
                _ => tokens.push(self.consume_token()),
            }
        }
        tokens
    }

    /// Parse declarations and nested rules until `Token::Delim('}')`.
    fn parse_declaration_and_nested_rule_list(&mut self) -> ParseResult<Vec<CssNode>> {
        let mut children = vec![];
        let mut parsing_name = true;
        let mut name = String::new();
        let mut value_tokens = vec![];

        loop {
            let mut cursor = 0;

            loop {
                let token = self.peek_next_token(cursor);

                match token {
                    Token::Delim(':') if parsing_name => {
                        for _ in 0..cursor {
                            if let Token::Ident(s) = self.consume_token() {
                                name.push_str(&s);
                            }
                        }

                        self.consume_token(); // consume :
                        parsing_name = false;
                        break;
                    }
                    Token::Delim(';') if !parsing_name => {
                        for _ in 0..cursor {
                            value_tokens.push(self.consume_token());
                        }

                        self.consume_token(); // consume ;
                        children.push(CssNode {
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
                        break;
                    }
                    Token::Delim('{') => {
                        children.push(self.parse_rule()?);
                        cursor = 0;
                    }
                    Token::Delim('}') | Token::EOF => {
                        if !parsing_name && !name.is_empty() {
                            for _ in 0..cursor {
                                value_tokens.push(self.consume_token());
                            }

                            children.push(CssNode {
                                node: CssNodeType::Declaration {
                                    name: std::mem::take(&mut name),
                                    value: Self::parse_tokens_to_css_value(std::mem::take(
                                        &mut value_tokens,
                                    ))?,
                                },
                                children: vec![],
                            });
                        } else {
                            for _ in 0..cursor {
                                // Just consume token.
                                self.consume_token();
                            }
                        }

                        break;
                    }
                    Token::Ident(s) if parsing_name => {
                        if cursor == 0 {
                            name.push_str(s);
                            self.consume_token();
                            break;
                        }

                        cursor += 1;
                    }
                    _ => {
                        cursor += 1;
                    }
                }
            }

            if matches!(self.peek_next_token(0), Token::Delim('}') | Token::EOF) {
                break;
            }
        }

        Ok(children)
    }

    pub fn parse_tokens_to_css_value(tokens: Vec<Token>) -> ParseResult<CssValue> {
        let mut values = vec![];
        let mut iter = tokens.into_iter().peekable();

        while let Some(token) = iter.next() {
            log::debug!(target: "CssParser", "parse_tokens_to_css_value: token={:?}", token);

            match token {
                Token::Ident(s) => values.push(CssValue::Keyword(s.into())),

                Token::Delim(',') => {
                    // List separator
                    continue;
                }

                Token::Delim('(') | Token::Delim(')') => {
                    // Function の構文用なので無視
                    continue;
                }

                Token::Delim(c) => {
                    let mut s = [0_u8; 4];
                    let s = c.encode_utf8(&mut s);

                    values.push(CssValue::Keyword(s.into()));
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
                        "deg" => Unit::Deg,
                        "fr" => Unit::Fr,
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
            0 => CssValue::Keyword(CssIdent::new_static("")),
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

impl std::fmt::Display for Combinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Combinator::Descendant => f.write_str(" "),
            Combinator::Child => f.write_str(" > "),
            Combinator::NextSibling => f.write_str(" + "),
            Combinator::SubsequentSibling => f.write_str(" ~ "),
        }
    }
}

/// Formats the `An+B` microsyntax stored by [`PseudoClass::Nth`], e.g.
/// `2n+1`, `odd`-style `2n`, or a bare `3`.
fn write_an_plus_b(f: &mut std::fmt::Formatter<'_>, a: i32, b: i32) -> std::fmt::Result {
    if a == 0 {
        return write!(f, "{b}");
    }
    match a {
        1 => f.write_str("n")?,
        -1 => f.write_str("-n")?,
        _ => write!(f, "{a}n")?,
    }
    if b > 0 {
        write!(f, "+{b}")
    } else if b < 0 {
        write!(f, "{b}")
    } else {
        Ok(())
    }
}

impl std::fmt::Display for PseudoClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PseudoClass::Simple(name) => write!(f, ":{name}"),
            PseudoClass::SelectorList { name, selectors } => {
                let arguments = selectors
                    .iter()
                    .map(ComplexSelector::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, ":{name}({arguments})")
            }
            PseudoClass::Nth { name, a, b } => {
                write!(f, ":{name}(")?;
                write_an_plus_b(f, *a, *b)?;
                f.write_str(")")
            }
        }
    }
}

impl std::fmt::Display for Selector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_nesting {
            f.write_str("&")?;
        }
        if let Some(tag) = &self.tag {
            f.write_str(tag)?;
        }
        if let Some(id) = &self.id {
            write!(f, "#{id}")?;
        }
        for class in &self.classes {
            write!(f, ".{class}")?;
        }
        for attribute in &self.attributes {
            f.write_str("[")?;
            f.write_str(&attribute.name)?;
            if let Some(value) = &attribute.value {
                write!(f, "=\"{value}\"")?;
            }
            f.write_str("]")?;
        }
        for pseudo_class in &self.pseudo_classes {
            write!(f, "{pseudo_class}")?;
        }
        if let Some(pseudo_element) = &self.pseudo_element {
            write!(f, "::{pseudo_element}")?;
        }
        Ok(())
    }
}

impl std::fmt::Display for ComplexSelector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Parts are stored right-to-left and each part carries the
        // combinator linking it to the part on its left (`parts[k]` holds the
        // relationship between `parts[k + 1]` and itself). Emitting left to
        // right therefore reads the combinator from the *next* part.
        for index in (0..self.parts.len()).rev() {
            write!(f, "{}", self.parts[index].selector)?;
            if index > 0
                && let Some(combinator) = self.parts[index - 1].combinator
            {
                write!(f, "{combinator}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complex_selector_display_renders_combinators_and_compounds() {
        let stylesheet = Parser::new("div.a > p#x + span:hover { color: red; }")
            .parse()
            .unwrap();
        let CssNodeType::Rule { selectors } = stylesheet.children()[0].node() else {
            panic!("expected a rule");
        };
        assert_eq!(selectors[0].to_string(), "div.a > p#x + span:hover");

        let stylesheet = Parser::new("ul li ~ a[rel=\"tag\"] { color: red; }")
            .parse()
            .unwrap();
        let CssNodeType::Rule { selectors } = stylesheet.children()[0].node() else {
            panic!("expected a rule");
        };
        assert_eq!(selectors[0].to_string(), "ul li ~ a[rel=\"tag\"]");
    }

    #[test]
    fn selector_display_renders_nth_arguments() {
        let stylesheet = Parser::new("li:nth-child(2n+1) { color: red; }")
            .parse()
            .unwrap();
        let CssNodeType::Rule { selectors } = stylesheet.children()[0].node() else {
            panic!("expected a rule");
        };
        assert_eq!(selectors[0].to_string(), "li:nth-child(2n+1)");
    }

    #[test]
    fn parses_exact_attribute_selector() {
        let stylesheet = Parser::new(r#"input[type="hidden"] { display: none; }"#)
            .parse()
            .unwrap();
        let CssNodeType::Rule { selectors } = stylesheet.children()[0].node() else {
            panic!("expected CSS rule");
        };
        let selector = &selectors[0].parts[0].selector;

        assert_eq!(selector.tag.as_deref(), Some("input"));
        assert_eq!(
            selector.attributes,
            vec![AttributeSelector {
                name: "type".into(),
                value: Some("hidden".into()),
            }]
        );
    }

    #[test]
    fn preserves_fractional_grid_units() {
        let stylesheet = Parser::new("main { grid-template-columns: 100px 2fr auto; }")
            .parse()
            .unwrap();
        let declaration = stylesheet.children()[0].children()[0].node();
        let CssNodeType::Declaration { value, .. } = declaration else {
            panic!("expected declaration");
        };
        assert_eq!(
            value,
            &CssValue::List(vec![
                CssValue::Length(100.0, Unit::Px),
                CssValue::Length(2.0, Unit::Fr),
                CssValue::Keyword("auto".into()),
            ])
        );
    }

    #[test]
    fn preserves_grid_functions_and_area_strings() {
        let stylesheet = Parser::new(
            r#"main {
                grid-template-columns: repeat(auto-fit, minmax(100px, 1fr));
                grid-template-areas: "header header" "sidebar main";
            }"#,
        )
        .parse()
        .unwrap();
        let declarations = stylesheet.children()[0].children();
        let CssNodeType::Declaration { value, .. } = declarations[0].node() else {
            panic!("expected declaration");
        };
        assert_eq!(
            value,
            &CssValue::Function(
                "repeat".into(),
                vec![
                    CssValue::Keyword("auto-fit".into()),
                    CssValue::Function(
                        "minmax".into(),
                        vec![
                            CssValue::Length(100.0, Unit::Px),
                            CssValue::Length(1.0, Unit::Fr),
                        ],
                    ),
                ],
            )
        );
        let CssNodeType::Declaration { value, .. } = declarations[1].node() else {
            panic!("expected declaration");
        };
        assert_eq!(
            value,
            &CssValue::List(vec![
                CssValue::String("header header".into()),
                CssValue::String("sidebar main".into()),
            ])
        );
    }

    #[test]
    fn lossy_parser_resumes_after_recovering_a_failed_at_rule() {
        let mut parser = Parser::new("@media { @broken } .valid { color: green; }");
        let stylesheet = parser.parse_lossy();

        assert_eq!(stylesheet.children().len(), 1);
        let CssNodeType::Rule { selectors } = stylesheet.children()[0].node() else {
            panic!("expected recovered CSS rule");
        };
        assert_eq!(selectors[0].parts[0].selector.tag.as_deref(), None);
        assert_eq!(
            selectors[0].parts[0].selector.classes,
            vec![String::from("valid")]
        );
    }

    #[test]
    fn nesting_without_ampersand_uses_descendant_combinator() {
        let stylesheet = Parser::new(".parent { span { color: red; } }")
            .parse()
            .unwrap();
        let CssNodeType::Rule { selectors: parent } = stylesheet.children()[0].node() else {
            panic!("expected parent rule");
        };
        let CssNodeType::Rule { selectors: child } = stylesheet.children()[0].children()[0].node()
        else {
            panic!("expected child rule");
        };

        // Child selector should remain as-is (no & in child).
        assert_eq!(child[0].to_string(), "span");

        // Nesting at resolver level: .parent span
        let resolved = parent[0].nest(&child[0]);
        assert_eq!(resolved.to_string(), ".parent span");
    }

    #[test]
    fn nesting_with_standalone_ampersand() {
        let stylesheet = Parser::new(".parent { & { color: red; } }")
            .parse()
            .unwrap();
        let CssNodeType::Rule { selectors: parent } = stylesheet.children()[0].node() else {
            panic!("expected parent rule");
        };
        let CssNodeType::Rule { selectors: child } = stylesheet.children()[0].children()[0].node()
        else {
            panic!("expected child rule");
        };

        assert!(child[0].parts[0].selector.is_nesting);

        let resolved = parent[0].nest(&child[0]);
        assert_eq!(resolved.to_string(), ".parent");
    }

    #[test]
    fn nesting_with_ampersand_class_compound() {
        let stylesheet = Parser::new(".parent { &.highlight { color: red; } }")
            .parse()
            .unwrap();
        let CssNodeType::Rule { selectors: parent } = stylesheet.children()[0].node() else {
            panic!("expected parent rule");
        };
        let CssNodeType::Rule { selectors: child } = stylesheet.children()[0].children()[0].node()
        else {
            panic!("expected child rule");
        };

        assert!(child[0].parts[0].selector.is_nesting);
        assert_eq!(child[0].parts[0].selector.classes, vec!["highlight"]);

        let resolved = parent[0].nest(&child[0]);
        assert_eq!(resolved.to_string(), ".parent.highlight");
    }

    #[test]
    fn nesting_with_ampersand_at_end_of_compound() {
        let stylesheet = Parser::new(".parent { .sidebar& { color: red; } }")
            .parse()
            .unwrap();
        let CssNodeType::Rule { selectors: parent } = stylesheet.children()[0].node() else {
            panic!("expected parent rule");
        };
        let CssNodeType::Rule { selectors: child } = stylesheet.children()[0].children()[0].node()
        else {
            panic!("expected child rule");
        };

        assert!(child[0].parts[0].selector.is_nesting);
        assert_eq!(child[0].parts[0].selector.classes, vec!["sidebar"]);

        let resolved = parent[0].nest(&child[0]);
        assert_eq!(resolved.to_string(), ".parent.sidebar");
    }

    #[test]
    fn nesting_with_ampersand_and_child_combinator() {
        let stylesheet = Parser::new(".parent { & > span { color: red; } }")
            .parse()
            .unwrap();
        let CssNodeType::Rule { selectors: parent } = stylesheet.children()[0].node() else {
            panic!("expected parent rule");
        };
        let CssNodeType::Rule { selectors: child } = stylesheet.children()[0].children()[0].node()
        else {
            panic!("expected child rule");
        };

        let resolved = parent[0].nest(&child[0]);
        assert_eq!(resolved.to_string(), ".parent > span");
    }

    #[test]
    fn nesting_with_multiple_selectors_using_ampersand() {
        let stylesheet = Parser::new(".parent { &.a, &.b { color: red; } }")
            .parse()
            .unwrap();
        let CssNodeType::Rule { selectors: parent } = stylesheet.children()[0].node() else {
            panic!("expected parent rule");
        };
        let CssNodeType::Rule { selectors: child } = stylesheet.children()[0].children()[0].node()
        else {
            panic!("expected child rule");
        };

        assert_eq!(child.len(), 2);
        let resolved_a = parent[0].nest(&child[0]);
        let resolved_b = parent[0].nest(&child[1]);
        assert_eq!(resolved_a.to_string(), ".parent.a");
        assert_eq!(resolved_b.to_string(), ".parent.b");
    }

    #[test]
    fn nesting_with_descendant_then_ampersand() {
        let stylesheet = Parser::new(".outer { .parent { &.highlight { color: red; } } }")
            .parse()
            .unwrap();
        let CssNodeType::Rule { selectors: outer } = stylesheet.children()[0].node() else {
            panic!("expected outer rule");
        };
        let CssNodeType::Rule { selectors: parent } = stylesheet.children()[0].children()[0].node()
        else {
            panic!("expected parent rule");
        };
        let CssNodeType::Rule { selectors: child } =
            stylesheet.children()[0].children()[0].children()[0].node()
        else {
            panic!("expected child rule");
        };

        // First level: .outer .parent
        let resolved_parent = outer[0].nest(&parent[0]);
        assert_eq!(resolved_parent.to_string(), ".outer .parent");

        // Second level: .outer .parent.highlight
        let resolved_child = resolved_parent.nest(&child[0]);
        assert_eq!(resolved_child.to_string(), ".outer .parent.highlight");
    }

    #[test]
    fn nesting_with_multilevel_ampersand_and_combinator() {
        let stylesheet = Parser::new("#id .parent { .a & .b > span { color: red; } }")
            .parse()
            .unwrap();
        let CssNodeType::Rule { selectors: parent } = stylesheet.children()[0].node() else {
            panic!("expected parent rule");
        };
        let CssNodeType::Rule { selectors: child } = stylesheet.children()[0].children()[0].node()
        else {
            panic!("expected child rule");
        };

        let resolved = parent[0].nest(&child[0]);
        assert_eq!(resolved.to_string(), ".a #id .parent .b > span");
    }

    #[test]
    fn nested_declaration_parsing() {
        let stylesheet = Parser::new(".parent { color: red; & { font-size: 14px; } }")
            .parse()
            .unwrap();
        let CssNodeType::Rule { selectors } = stylesheet.children()[0].node() else {
            panic!("expected parent rule");
        };
        assert_eq!(selectors[0].to_string(), ".parent");

        let children = stylesheet.children()[0].children();
        assert_eq!(children.len(), 2);

        // First child: declaration
        let CssNodeType::Declaration { name, .. } = children[0].node() else {
            panic!("expected declaration");
        };
        assert_eq!(name, "color");

        // Second child: nested rule
        let CssNodeType::Rule { selectors: nested } = children[1].node() else {
            panic!("expected nested rule");
        };
        assert!(nested[0].parts[0].selector.is_nesting);
    }
}
