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
//!
//! ## Allocation strategy
//!
//! Cascade stylesheets are populated with many short identifiers, and the
//! majority of them contain no escape sequences. Because the tokenizer
//! borrows from the input string, any such lexeme is returned as a
//! [`Cow::Borrowed`] slice of the input and only escape-containing (or
//! otherwise transformed) lexemes are materialized onto the heap. Numbers are
//! parsed directly from the input slice instead of being assembled in a
//! scratch buffer.

use std::borrow::Cow;

/// CSS token produced by the tokenizer.
///
/// This represents *syntactic units* only.
/// No semantic interpretation (length, color, etc.) is performed here.
///
/// String payloads are [`Cow`]s borrowing from the input where no escape
/// sequence was encountered, so common identifiers, units and comments avoid
/// heap allocation entirely.
#[derive(Debug, Clone)]
pub enum Token<'a> {
    /// Identifier token (e.g. `div`, `color`, `--custom`)
    Ident(Cow<'a, str>),

    /// Function token (e.g. `calc`, `var`)
    Function(Cow<'a, str>),

    /// Unquoted `url(...)` token (e.g. `url(data:image/png;base64,...)`). The
    /// raw inner content is captured as a single lexeme, including characters
    /// that would otherwise be tokenized further (such as `:` or `%` in a
    /// base64 `data:` URL).
    Url(Cow<'a, str>),

    /// Plain number without unit (e.g. `0`, `1.5`)
    Number(f32),

    /// Quoted string token (e.g. `"hello"`, `'world'`)
    String(Cow<'a, str>),

    /// Dimension token (e.g. `10px`, `50%`, `2em`)
    ///
    /// Percentages are also represented as a dimension
    /// with `%` as the unit.
    Dimension(f32, Cow<'a, str>),

    /// Delimiter token (single-character symbols such as `:`, `;`, `>`, `+`)
    Delim(char),

    /// Hash with String (e.g. `#fff`)
    Hash(Cow<'a, str>),

    /// AtKeyword (e.g. `@media`)
    AtKeyword(Cow<'a, str>),

    /// One or more whitespace characters
    Whitespace,

    /// Comment
    Comment(Cow<'a, str>),

    /// End-of-input marker
    EOF,
}

/// Equality across token lifetimes so tokens borrowing from a live input can
/// be compared against `'static` literals (as in tests) or vice versa.
impl<'a, 'b> PartialEq<Token<'b>> for Token<'a> {
    fn eq(&self, other: &Token<'b>) -> bool {
        match (self, other) {
            (Token::Ident(a), Token::Ident(b)) => a == b,
            (Token::Function(a), Token::Function(b)) => a == b,
            (Token::Url(a), Token::Url(b)) => a == b,
            (Token::Number(a), Token::Number(b)) => a == b,
            (Token::String(a), Token::String(b)) => a == b,
            (Token::Dimension(av, au), Token::Dimension(bv, bu)) => av == bv && au == bu,
            (Token::Delim(a), Token::Delim(b)) => a == b,
            (Token::Hash(a), Token::Hash(b)) => a == b,
            (Token::AtKeyword(a), Token::AtKeyword(b)) => a == b,
            (Token::Whitespace, Token::Whitespace) => true,
            (Token::Comment(a), Token::Comment(b)) => a == b,
            (Token::EOF, Token::EOF) => true,
            _ => false,
        }
    }
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
#[derive(Clone)]
pub struct Tokenizer<'a> {
    /// Iterator over the input characters
    chars: std::str::Chars<'a>,

    /// Current character under examination
    current: Option<char>,

    /// The full input, used to slice borrowed token payloads
    input: &'a str,
}

impl<'a> Tokenizer<'a> {
    /// Create a new tokenizer from a CSS source string.
    pub fn new(input: &'a str) -> Self {
        let mut chars = input.chars();
        let current = chars.next();

        Self {
            chars,
            current,
            input,
        }
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

    /// Byte offset of the current (unconsumed) character. This is also the
    /// exclusive end of the last consumed lexeme, so slices of `self.input`
    /// taken between a recorded start and this position are exact.
    fn pos(&self) -> usize {
        self.input.len() - self.chars.as_str().len()
    }

    /// Byte offset of the *first* byte of the current character (i.e. of the
    /// character that `self.pos()` points just past). Used as the start of a
    /// lexeme that begins at the current character.
    fn cur_start(&self) -> usize {
        self.pos() - self.current.map(char::len_utf8).unwrap_or(0)
    }

    /// Consume and return the next token from the input.
    ///
    /// This is the main entry point used by the parser.
    pub fn next_token(&mut self) -> Token<'a> {
        let token = match self.peek() {
            Some(c) if c.is_whitespace() => self.consume_whitespace(),
            Some(c) if is_number_start(c, self.peek_next()) => self.consume_number_like(),
            Some(c) if is_ident_start(c) => self.consume_ident_like(),
            Some(c) if is_string_delimiter(c) => self.consume_string_like(),
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
                let start = self.cur_start();
                while let Some(c) = self.peek() {
                    if is_ident_continue(c) {
                        self.bump();
                    } else {
                        break;
                    }
                }
                Token::Hash(Cow::Borrowed(&self.input[start..self.cur_start()]))
            }
            Some('@') => {
                self.bump();
                let start = self.cur_start();
                while let Some(c) = self.peek() {
                    if is_ident_continue(c) {
                        self.bump();
                    } else {
                        break;
                    }
                }
                Token::AtKeyword(Cow::Borrowed(&self.input[start..self.cur_start()]))
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
    fn consume_whitespace(&mut self) -> Token<'a> {
        while matches!(self.current, Some(c) if c.is_whitespace()) {
            self.bump();
        }
        Token::Whitespace
    }

    /// Consume an identifier or function token.
    ///
    /// If an identifier is immediately followed by `(`,
    /// this method should produce a `Token::Function`.
    ///
    /// Identifiers without escape sequences are returned as borrowed slices
    /// of the input; escape resolution materializes a heap buffer lazily.
    fn consume_ident_like(&mut self) -> Token<'a> {
        let start = self.cur_start();
        let mut owned: Option<String> = None;

        while let Some(c) = self.peek() {
            if c == '\\' {
                let buf =
                    owned.get_or_insert_with(|| self.input[start..self.cur_start()].to_string());
                if let Some(escaped) = self.consume_escape() {
                    buf.push(escaped);
                }
            } else if is_ident_continue(c) {
                if let Some(buf) = owned.as_mut() {
                    buf.push(c);
                }
                self.bump();
            } else {
                break;
            }
        }

        let ident: Cow<'a, str> = match owned {
            Some(s) => Cow::Owned(s),
            None => Cow::Borrowed(&self.input[start..self.cur_start()]),
        };

        if self.peek() == Some('(') {
            if ident.eq_ignore_ascii_case("url") {
                self.consume_url_after_name(ident)
            } else {
                Token::Function(ident)
            }
        } else {
            Token::Ident(ident)
        }
    }

    /// Consume the body of an unquoted `url(...)` token after the name has
    /// already been read.
    ///
    /// Per the CSS Syntax spec, `url(` followed by a string delimiter is a
    /// plain function token whose quoted string argument is handled by the
    /// parser. Otherwise we consume the raw content up to the closing `)`,
    /// resolving escapes, so that the URL is preserved byte-for-byte.
    fn consume_url_after_name(&mut self, ident: Cow<'a, str>) -> Token<'a> {
        self.bump(); // consume '('

        if let Some(c) = self.peek()
            && is_string_delimiter(c)
        {
            // Quoted URL (e.g. `url("foo.png")`) — behave as a function token
            // so the parser handles the string argument.
            return Token::Function(ident);
        }

        let start = self.cur_start();
        let mut owned: Option<String> = None;
        let mut end = self.input.len();

        loop {
            match self.peek() {
                None => break, // EOF without a closing paren
                Some(')') => {
                    end = self.cur_start();
                    self.bump();
                    break;
                }
                Some('\\') => {
                    let buf = owned
                        .get_or_insert_with(|| self.input[start..self.cur_start()].to_string());
                    if let Some(escaped) = self.consume_escape() {
                        buf.push(escaped);
                    } else {
                        // A lone trailing backslash: bad URL, but keep the
                        // eventual closing paren in sync.
                        end = self.cur_start();
                        self.bump();
                        break;
                    }
                }
                Some(c) => {
                    if let Some(buf) = owned.as_mut() {
                        buf.push(c);
                    }
                    self.bump();
                }
            }
        }

        let value: Cow<'a, str> = match owned {
            Some(s) => Cow::Owned(s.trim().to_string()),
            None => Cow::Borrowed(self.input[start..end].trim()),
        };

        Token::Url(value)
    }

    fn consume_string_like(&mut self) -> Token<'a> {
        let quote = self.peek().unwrap(); // '"' or '\''
        self.bump(); // consume opening quote

        let start = self.cur_start();
        let mut owned: Option<String> = None;
        let mut end = self.input.len();

        while let Some(c) = self.peek() {
            if c == quote {
                end = self.cur_start();
                self.bump(); // consume closing quote
                break;
            }

            if c == '\\' {
                let buf =
                    owned.get_or_insert_with(|| self.input[start..self.cur_start()].to_string());
                if let Some(escaped) = self.consume_escape() {
                    buf.push(escaped);
                }
                continue;
            }

            if let Some(buf) = owned.as_mut() {
                buf.push(c);
            }
            self.bump();
        }

        let value: Cow<'a, str> = match owned {
            Some(s) => Cow::Owned(s),
            None => Cow::Borrowed(&self.input[start..end]),
        };

        Token::String(value)
    }

    /// Consume a number-like token.
    ///
    /// This may produce:
    /// - `Token::Number`
    /// - `Token::Dimension` (including `%`)
    ///
    /// The numeric value is parsed straight from the input slice rather than
    /// through a scratch buffer, avoiding per-token `String` churn.
    fn consume_number_like(&mut self) -> Token<'a> {
        let start = self.cur_start();

        let mut has_dot = if self.peek() == Some('.') {
            self.bump();
            true
        } else {
            false
        };

        if self.peek() == Some('-') {
            self.bump();
        }

        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.bump();
            } else if c == '.' && !has_dot {
                has_dot = true;
                self.bump();
            } else {
                break;
            }
        }

        let value: f32 = self.input[start..self.cur_start()].parse().unwrap_or(0.0);

        // --- unit / percentage branching ---
        match self.peek() {
            Some('%') => {
                self.bump();
                Token::Dimension(value, Cow::Borrowed("%"))
            }
            Some(c) if is_ident_start(c) => {
                let unit_start = self.cur_start();
                let mut owned: Option<String> = None;

                while let Some(c) = self.peek() {
                    if c == '\\' {
                        let buf = owned.get_or_insert_with(|| {
                            self.input[unit_start..self.cur_start()].to_string()
                        });
                        if let Some(escaped) = self.consume_escape() {
                            buf.push(escaped);
                        }
                    } else if is_ident_continue(c) {
                        if let Some(buf) = owned.as_mut() {
                            buf.push(c);
                        }
                        self.bump();
                    } else {
                        break;
                    }
                }

                let unit: Cow<'a, str> = match owned {
                    Some(s) => Cow::Owned(s),
                    None => Cow::Borrowed(&self.input[unit_start..self.cur_start()]),
                };

                Token::Dimension(value, unit)
            }
            _ => Token::Number(value),
        }
    }

    /// Consume a CSS comment.
    ///
    /// Assumes the opening `/*` has already been consumed.
    fn consume_comment(&mut self) -> Token<'a> {
        let start = self.cur_start();

        while let Some(c) = self.peek() {
            if c == '*' && self.peek_next() == Some('/') {
                self.bump(); // consume '*'
                self.bump(); // consume '/'
                break;
            } else {
                self.bump();
            }
        }

        Token::Comment(Cow::Borrowed(&self.input[start..self.cur_start()]))
    }

    fn consume_escape(&mut self) -> Option<char> {
        self.bump(); // consume '\'

        // 1. Line continuation: backslash + newline => nothing
        match self.peek() {
            Some('\n') => {
                self.bump();
                return None;
            }
            Some('\r') => {
                self.bump();
                if self.peek() == Some('\n') {
                    self.bump(); // CRLF
                }
                return None;
            }
            _ => {}
        }

        // 2. Unicode escape
        let mut code: u32 = 0;
        let mut hex_count = 0;
        for _ in 0..6 {
            match self.peek() {
                Some(c) if c.is_ascii_hexdigit() => {
                    if let Some(digit) = c.to_digit(16) {
                        code = code * 16 + digit;
                        hex_count += 1;
                        self.bump();
                    }
                }
                _ => break,
            }
        }

        if hex_count > 0 {
            if matches!(self.peek(), Some(c) if c.is_whitespace()) {
                self.bump(); // optional whitespace
            }

            return std::char::from_u32(code).or(Some('\u{FFFD}'));
        }

        // 3. Simple escape
        if let Some(c) = self.peek() {
            self.bump();
            Some(c)
        } else {
            None
        }
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
    c.is_ascii_alphabetic() || c == '\\' || c == '_' || c == '-' || !c.is_ascii()
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
/// - A dot followed by a digit (e.g. `.5`)
/// - A hyphen followed by a digit or dot (e.g. `-1`, `-.5`)
fn is_number_start(current: char, next: Option<char>) -> bool {
    current.is_ascii_digit()
        || (current == '.' && matches!(next, Some(c) if c.is_ascii_digit()))
        || (current == '-' && matches!(next, Some(c) if c.is_ascii_digit() || c == '.'))
}
