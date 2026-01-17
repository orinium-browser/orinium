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
use super::tokenizer::{Token, Tokenizer};

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
        value: Token,
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
    pub fn parse(&mut self) -> CssNode {
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
                    let node = self.parse_rule(); // placeholder
                    stylesheet.children.push(node);
                }
            }
        }

        stylesheet
    }

    /// Parse a qualified rule (e.g., `div { color: red; }`).
    ///
    /// Parses the selector list first, then the block of declarations.
    fn parse_rule(&mut self) -> CssNode {
        // 1. Parse selectors
        let selectors = self.parse_selector_list();

        // 2. Expect `{`
        match self.consume_token() {
            Token::Delim('{') => self.brace_depth += 1,
            token => panic!("Expected '{{', found {:?}", token),
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
                Token::EOF => break,
                _ => {
                    // TODO: parse a declaration
                    self.consume_token(); // placeholder consume
                }
            }
        }

        CssNode {
            node: CssNodeType::Rule { selectors },
            children,
        }
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
                    self.consume_token();
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
}
