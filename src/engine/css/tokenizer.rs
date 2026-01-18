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

    /// Hash with String (e.g. `#fff`)
    Hash(String),

    /// AtKeyword (e.g. `@media`)
    AtKeyword(String),

    /// One or more whitespace characters
    Whitespace,

    /// Comment
    Comment(String),

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

    /// Peek the next character from the current one without consuming it.
    fn peek_next(&self) -> Option<char> {
        self.chars.clone().next()
    }

    /// Consume and return the next token from the input.
    ///
    /// This is the main entry point used by the parser.
    pub fn next_token(&mut self) -> Token {
        let token = match self.peek() {
            Some(c) if c.is_whitespace() => self.consume_whitespace(),
            Some(c) if is_ident_start(c) => self.consume_ident_like(),
            Some(c) if is_string_delimiter(c) => self.consume_string_like(),
            Some(c) if is_number_start(c, self.peek_next()) => self.consume_number_like(),
            Some('/') => {
                if self.peek_next() == Some('*') {
                    self.bump(); // consume '/'
                    self.bump(); // consume '*'
                    self.consume_comment()
                } else {
                    self.bump();
                    Token::Delim('/')
                }
            }
            Some('#') => {
                self.bump(); // consume '#'
                let mut value = String::new();
                while let Some(c) = self.peek() {
                    if is_ident_continue(c) {
                        value.push(c);
                        self.bump();
                    } else {
                        break;
                    }
                }
                Token::Hash(value)
            }
            Some('@') => {
                self.bump();
                let mut value = String::new();
                while let Some(c) = self.peek() {
                    if is_ident_continue(c) {
                        value.push(c);
                        self.bump();
                    } else {
                        break;
                    }
                }
                Token::AtKeyword(value)
            }
            Some(c) => {
                self.bump();
                Token::Delim(c)
            }
            None => Token::EOF,
        };

        log::debug!(target: "CssTokenizer", "Tokenized: {:?}", token);

        token
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
        let mut ident = String::new();

        while let Some(c) = self.peek() {
            if is_ident_continue(c) {
                ident.push(c);
                self.bump();
            } else {
                break;
            }
        }
        if self.peek() == Some('(') {
            Token::Function(ident)
        } else {
            Token::Ident(ident)
        }
    }

    fn consume_string_like(&mut self) -> Token {
        let quote = self.peek().unwrap(); // '"' or '\''
        self.bump(); // consume opening quote

        let mut value = String::new();

        while let Some(c) = self.peek() {
            if c == quote {
                self.bump(); // consume closing quote
                break;
            }

            // escape / newline handling will go here later
            value.push(c);
            self.bump();
        }

        Token::String(value)
    }

    /// Consume a number-like token.
    ///
    /// This may produce:
    /// - `Token::Number`
    /// - `Token::Dimension` (including `%`)
    fn consume_number_like(&mut self) -> Token {
        let mut buf = String::new();

        let mut has_dot = if self.peek() == Some('.') {
            buf.push('.');
            self.bump();
            true
        } else {
            false
        };

        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                buf.push(c);
                self.bump();
            } else if c == '.' && !has_dot {
                has_dot = true;
                buf.push(c);
                self.bump();
            } else {
                break;
            }
        }

        let value: f32 = buf.parse().unwrap_or(0.0);

        // --- unit / percentage branching ---
        match self.peek() {
            Some('%') => {
                self.bump();
                Token::Dimension(value, "%".to_string())
            }
            Some(c) if is_ident_start(c) => {
                let mut unit = String::new();
                while let Some(c) = self.peek() {
                    if is_ident_continue(c) {
                        unit.push(c);
                        self.bump();
                    } else {
                        break;
                    }
                }
                Token::Dimension(value, unit)
            }
            _ => Token::Number(value),
        }
    }

    /// Consume a CSS comment.
    ///
    /// Assumes the opening `/*` has already been consumed.
    fn consume_comment(&mut self) -> Token {
        let mut value = String::new();

        while let Some(c) = self.peek() {
            if c == '*' && self.peek_next() == Some('/') {
                self.bump(); // consume '*'
                self.bump(); // consume '/'
                break;
            } else {
                value.push(c);
                self.bump();
            }
        }

        Token::Comment(value)
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

/// Returns true if the character is a CSS string delimiter.
///
/// CSS strings are delimited by either double quotes (`"`)
/// or single quotes (`'`).
fn is_string_delimiter(c: char) -> bool {
    matches!(c, '"' | '\'')
}

/// Returns true if the character can continue an identifier.
///
/// - ASCII letters (A–Z, a–z)
/// - ASCII digits (0–9)
/// - Underscore (`_`)
/// - Hyphen (`-`)
/// - Non-ASCII characters
fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || !c.is_ascii()
}

/// Returns true if the character is a CSS number start.
///
/// - ASCII digits (0-9)
/// - Dot (`.`)
fn is_number_start(current: char, next: Option<char>) -> bool {
    current.is_ascii_digit() || (current == '.' && matches!(next, Some(c) if c.is_ascii_digit()))
}
