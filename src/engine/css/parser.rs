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
    /// This method consumes tokens from the tokenizer until `Token::EOF`
    /// is reached and constructs a `CssNode` representing the stylesheet
    /// root.
    ///
    /// Parsing behavior:
    /// - Whitespace tokens are generally ignored
    /// - Qualified rules and at-rules are parsed into child nodes
    /// - No semantic validation is performed at this stage
    ///
    /// The returned syntax tree represents **syntactic structure only**.
    /// All semantic interpretation (cascade, inheritance, value resolution)
    /// is handled by later stages of the engine.
    pub fn parse(&mut self) -> CssNode {
        let mut stylesheet = CssNode {
            node: CssNodeType::Stylesheet,
            children: vec![],
        };

        loop {
            let token = self.tokenizer.next_token();

            match token {
                Token::EOF => break,
                Token::Whitespace => { /* Ignore for now */ }

                _ => {
                    // ここで rule / at-rule / error に分岐
                    // let node = self.parse_rule_or_at_rule(token);
                    // stylesheet.children.push(node);
                }
            }
        }

        stylesheet
    }
}
