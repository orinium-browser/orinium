//! CSS Tokenizer
//!
//! This module implements a **CSS tokenizer**, responsible for converting
//! a raw CSS source string into a flat stream of tokens.
//!
//! ## Responsibilities
//!
//! - Consume raw characters
//! - Produce syntactic tokens defined by the CSS specification
//! - Preserve the original structure of the input as much as possible
//!
//! ## Non-responsibilities
//!
//! - Parsing selectors or declarations
//! - Interpreting values (lengths, colors, percentages, etc.)
//! - Building trees or nested structures
//!
//! ## Design notes
//!
//! - Tokens are produced in a **linear stream**
//! - Function tokens only represent the function name
//! - Matching of parentheses and function arguments is handled by the parser

/// CSS token produced by the tokenizer.
///
/// This represents *syntactic units* only.
/// No semantic interpretation (length, color, etc.) is performed here.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// Identifier token (e.g. `div`, `color`, `--custom`)
    Ident(String),

    /// Function token (e.g. `calc`, `var`)
    Function(String),

    /// Plain number without unit (e.g. `0`, `1.5`)
    Number(f32),

    /// Quoted string token (e.g. `"hello"`, `'world'`)
    String(String),

    /// Dimension token (e.g. `10px`, `50%`, `2em`)
    ///
    /// Percentages are also represented as a dimension
    /// with `%` as the unit.
    Dimension(f32, String),

    /// Delimiter token (single-character symbols such as `:`, `;`, `>`, `+`)
    Delim(char),

    /// One or more whitespace characters
    Whitespace,

    /// End-of-input marker
    EOF,
}

/// CSS tokenizer.
///
/// This struct is responsible for converting a CSS source string
/// into a stream of `Token`s.
///
/// Responsibilities:
/// - Consume raw characters
/// - Produce syntactic tokens
///
/// Non-responsibilities:
/// - Parsing declarations or selectors
/// - Interpreting values (length, color, etc.)
/// - Building trees or higher-level structures
pub struct Tokenizer<'a> {
    /// Iterator over the input characters
    chars: std::str::Chars<'a>,

    /// Current character under examination
    current: Option<char>,
}

impl<'a> Tokenizer<'a> {
    /// Create a new tokenizer from a CSS source string.
    pub fn new(input: &'a str) -> Self {
        let mut chars = input.chars();
        let current = chars.next();

        Self { chars, current }
    }

    /// Advance to the next character.
    ///
    /// This method should update `self.current`.
    fn bump(&mut self) {
        self.current = self.chars.next();
    }

    /// Peek the current character without consuming it.
    fn peek(&self) -> Option<char> {
        self.current
    }

    /// Consume and return the next token from the input.
    ///
    /// This is the main entry point used by the parser.
    pub fn next_token(&mut self) -> Token {
        match self.peek() {
            Some(c) if c.is_whitespace() => self.consume_whitespace(),
            Some(c) if is_ident_start(c) => self.consume_ident_like(),
            Some(c) if is_string_start(c) => self.consume_string_like(),
            Some(c) if c.is_ascii_digit() => self.consume_number_like(),
            Some(c) => {
                self.bump();
                Token::Delim(c)
            }
            None => Token::EOF,
        }
    }

    /// Consume consecutive whitespace characters.
    ///
    /// Produces a single `Token::Whitespace`.
    fn consume_whitespace(&mut self) -> Token {
        while matches!(self.current, Some(c) if c.is_whitespace()) {
            self.bump();
        }
        Token::Whitespace
    }

    /// Consume an identifier or function token.
    ///
    /// If an identifier is immediately followed by `(`,
    /// this method should produce a `Token::Function`.
    fn consume_ident_like(&mut self) -> Token {
        todo!("consume identifier or function");
    }

    fn consume_string_like(&mut self) -> Token {
        todo!("consume string");
    }

    /// Consume a number-like token.
    ///
    /// This may produce:
    /// - `Token::Number`
    /// - `Token::Dimension` (including `%`)
    fn consume_number_like(&mut self) -> Token {
        todo!("consume number or dimension");
    }
}

/// Returns true if the character can start an identifier.
///
/// This is a simplified CSS identifier start check.
/// It supports:
/// - ASCII letters (A–Z, a–z)
/// - underscore (`_`)
/// - hyphen (`-`)
/// - non-ASCII characters
fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '-' || !c.is_ascii()
}

fn is_string_start(c: char) -> bool {
    todo!("implement CSS string start check");
}

/// Returns true if the character can continue an identifier.
fn is_ident_continue(c: char) -> bool {
    todo!("implement CSS identifier continuation check");
}
