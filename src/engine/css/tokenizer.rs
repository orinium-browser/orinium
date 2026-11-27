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
///
/// String payloads are owned `String`s since the tokenizer owns its input
/// data (backed by a `Vec<char>`).
#[derive(Debug, Clone)]
pub enum Token {
    /// Identifier token (e.g. `div`, `color`, `--custom`)
    Ident(String),

    /// Function token (e.g. `calc`, `var`)
    Function(String),

    /// Unquoted `url(...)` token (e.g. `url(data:image/png;base64,...)`). The
    /// raw inner content is captured as a single lexeme, including characters
    /// that would otherwise be tokenized further (such as `:` or `%` in a
    /// base64 `data:` URL).
    Url(String),

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

/// Equality across token lifetimes so tokens can be compared against
/// `'static` literals (as in tests).
impl PartialEq for Token {
    fn eq(&self, other: &Self) -> bool {
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
pub struct Tokenizer {
    input: Vec<char>,

    /// Current character under examination
    current: Option<char>,

    /// Offset of the current (unconsumed) character
    pos: usize,
}

impl Tokenizer {
    /// Create a new tokenizer from a CSS source string.
    pub fn new(input: &str) -> Self {
        let input: Vec<char> = input.chars().collect();

        let current = input.get(0).copied();

        Self {
            input,
            current,
            pos: 0,
        }
    }

    /// Advance to the next character.
    fn bump(&mut self) {
        self.pos += 1;
        self.current = self.input.get(self.pos).copied();
    }

    /// Peek the current character without consuming it.
    fn peek(&self) -> Option<char> {
        self.current
    }

    /// Peek the next character from the current one without consuming it.
    fn peek_next(&self) -> Option<char> {
        self.input.get(self.pos + 1).copied()
    }

    /// Copies characters from `start..end` into a `String`.
    ///
    fn slice_to_string(&self, start: usize, end: usize) -> String {
        self.input
            .get(start..end)
            .map(|v| v.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Consume and return the next token from the input.
    pub fn next_token(&mut self) -> Token {
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
                let start = self.pos;
                while let Some(c) = self.peek() {
                    if is_ident_continue(c) {
                        self.bump();
                    } else {
                        break;
                    }
                }
                Token::Hash(self.slice_to_string(start, self.pos))
            }
            Some('@') => {
                self.bump();
                let start = self.pos;
                while let Some(c) = self.peek() {
                    if is_ident_continue(c) {
                        self.bump();
                    } else {
                        break;
                    }
                }
                Token::AtKeyword(self.slice_to_string(start, self.pos))
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
    fn consume_whitespace(&mut self) -> Token {
        while matches!(self.current, Some(c) if c.is_whitespace()) {
            self.bump();
        }
        Token::Whitespace
    }

    /// Consume an identifier or function token.
    fn consume_ident_like(&mut self) -> Token {
        let start = self.pos;
        let mut owned: Option<String> = None;

        while let Some(c) = self.peek() {
            if c == '\\' {
                let buf = owned.get_or_insert_with(|| self.slice_to_string(start, self.pos));
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

        let ident = match owned {
            Some(s) => s,
            None => self.slice_to_string(start, self.pos),
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
    fn consume_url_after_name(&mut self, ident: String) -> Token {
        self.bump(); // consume '('

        if let Some(c) = self.peek()
            && is_string_delimiter(c)
        {
            return Token::Function(ident);
        }

        let start = self.pos;
        let mut owned: Option<String> = None;

        loop {
            match self.peek() {
                None => break,
                Some(')') => {
                    self.bump();
                    break;
                }
                Some('\\') => {
                    let buf = owned.get_or_insert_with(|| self.slice_to_string(start, self.pos));
                    if let Some(escaped) = self.consume_escape() {
                        buf.push(escaped);
                    } else {
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

        let end = self.pos.saturating_sub(1);
        let value: String = match owned {
            Some(s) => s.trim().to_string(),
            None => self.slice_to_string(start, end).trim().to_string(),
        };

        Token::Url(value)
    }

    fn consume_string_like(&mut self) -> Token {
        let quote = self.peek().unwrap(); // '"' or '\''
        self.bump(); // consume opening quote

        let start = self.pos;
        let mut owned: Option<String> = None;

        while let Some(c) = self.peek() {
            if c == quote {
                self.bump(); // consume closing quote
                break;
            }

            if c == '\\' {
                let buf = owned.get_or_insert_with(|| self.slice_to_string(start, self.pos));
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

        let end = self.pos.saturating_sub(1);
        let value: String = match owned {
            Some(s) => s,
            None => self.slice_to_string(start, end),
        };

        Token::String(value)
    }

    /// Consume a number-like token.
    fn consume_number_like(&mut self) -> Token {
        let start = self.pos;

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

        let value: f32 = self.slice_to_string(start, self.pos).parse().unwrap_or(0.0);

        match self.peek() {
            Some('%') => {
                self.bump();
                Token::Dimension(value, "%".to_string())
            }
            Some(c) if is_ident_start(c) => {
                let unit_start = self.pos;
                let mut owned: Option<String> = None;

                while let Some(c) = self.peek() {
                    if c == '\\' {
                        let buf =
                            owned.get_or_insert_with(|| self.slice_to_string(unit_start, self.pos));
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

                let unit: String = match owned {
                    Some(s) => s,
                    None => self.slice_to_string(unit_start, self.pos),
                };

                Token::Dimension(value, unit)
            }
            _ => Token::Number(value),
        }
    }

    /// Consume a CSS comment.
    fn consume_comment(&mut self) -> Token {
        let start = self.pos;

        while let Some(c) = self.peek() {
            if c == '*' && self.peek_next() == Some('/') {
                self.bump(); // consume '*'
                self.bump(); // consume '/'
                break;
            } else {
                self.bump();
            }
        }

        Token::Comment(self.slice_to_string(start, self.pos.saturating_sub(2)))
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
fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '\\' || c == '_' || c == '-' || !c.is_ascii()
}

/// Returns true if the character is a CSS string delimiter.
fn is_string_delimiter(c: char) -> bool {
    matches!(c, '"' | '\'')
}

/// Returns true if the character can continue an identifier.
fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || !c.is_ascii()
}

/// Returns true if the character is a CSS number start.
fn is_number_start(current: char, next: Option<char>) -> bool {
    current.is_ascii_digit()
        || (current == '.' && matches!(next, Some(c) if c.is_ascii_digit()))
        || (current == '-' && matches!(next, Some(c) if c.is_ascii_digit() || c == '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokenize(input: &str) -> Vec<Token> {
        let mut tokenizer = Tokenizer::new(input);
        let mut tokens = Vec::new();

        loop {
            let token = tokenizer.next_token();
            tokens.push(token.clone());
            if token == Token::EOF {
                break;
            }
        }

        tokens
    }

    #[test]
    fn ident_simple_escape() {
        let tokens = tokenize(".foo\\+bar");

        assert_eq!(
            tokens,
            vec![
                Token::Delim('.'),
                Token::Ident("foo+bar".to_string()),
                Token::EOF,
            ]
        );
    }

    #[test]
    fn string_escape_quote() {
        let tokens = tokenize(r#""hello \"world\"""#);

        assert_eq!(
            tokens,
            vec![Token::String("hello \"world\"".to_string()), Token::EOF,]
        );
    }

    #[test]
    fn ident_unicode_escape() {
        let tokens = tokenize(r#".\31 23"#);

        assert_eq!(
            tokens,
            vec![
                Token::Delim('.'),
                Token::Ident("123".to_string()),
                Token::EOF,
            ]
        );
    }

    #[test]
    fn string_unicode_escape() {
        let tokens = tokenize(r#""\000026""#);

        assert_eq!(tokens, vec![Token::String("&".to_string()), Token::EOF,]);
    }

    #[test]
    fn complex_escape_sequence() {
        let tokens = tokenize(r#".foo\+bar { content: "a\26 b"; }"#);

        assert_eq!(
            tokens,
            vec![
                Token::Delim('.'),
                Token::Ident("foo+bar".to_string()),
                Token::Whitespace,
                Token::Delim('{'),
                Token::Whitespace,
                Token::Ident("content".to_string()),
                Token::Delim(':'),
                Token::Whitespace,
                Token::String("a&b".to_string()),
                Token::Delim(';'),
                Token::Whitespace,
                Token::Delim('}'),
                Token::EOF,
            ]
        );
    }

    #[test]
    fn escape_at_eof() {
        let tokens = tokenize(r#".foo\"#);

        assert_eq!(
            tokens,
            vec![
                Token::Delim('.'),
                Token::Ident("foo".to_string()),
                Token::EOF,
            ]
        );
    }

    #[test]
    fn ident_line_continuation() {
        let tokens = tokenize(".foo\\\nbar");

        assert_eq!(
            tokens,
            vec![
                Token::Delim('.'),
                Token::Ident("foobar".to_string()),
                Token::EOF,
            ]
        );
    }

    #[test]
    fn string_line_continuation() {
        let tokens = tokenize("\"a\\\nb\"");

        assert_eq!(tokens, vec![Token::String("ab".to_string()), Token::EOF,]);
    }
}
