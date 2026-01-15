/// CSS token definitions produced by the tokenizer.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Ident(String),         // Identifiers: color, margin, etc.
    StringLiteral(String), // Quoted strings: "string" or 'string'
    Number(f32),           // Plain numbers: 1, 1.5, 10
    Function { name: String, value: FunctionValue },
    AtKeyword(String),      // @media, @import, etc.
    Hash(String),           // Hash tokens: #fff, #id
    Dimension(f32, String), // Dimensions: 10px, 2em
    Percentage(f32),        // Percentages: 50%
    Colon,
    Semicolon,
    Comma,
    Whitespace,
    LeftBrace,       // {
    RightBrace,      // }
    LeftParen,       // (
    RightParen,      // )
    LeftBracket,     // [
    RightBracket,    // ]
    CDO,             // <!--
    CDC,             // -->
    Delim(char),     // Any other delimiter
    Comment(String), // /* comment */
}

#[derive(Debug, Clone, PartialEq)]
pub enum FunctionValue {
    Raw(String),        // url(...)
    Tokens(Vec<Token>), // rgb(), calc(), etc
}

/// Internal tokenizer states.
/// This is a simplified state machine inspired by the CSS Syntax spec.
#[derive(Debug, PartialEq, Clone)]
enum TokenizerState {
    Data,
    Ident,
    Number,
    StringDouble,
    StringSingle,
    Hash,
    AtKeyword,
    Comment,
    CommentEndStar,
}

/// CSS tokenizer that converts a character stream into tokens.
pub struct Tokenizer<'a> {
    input: &'a str,
    pos: usize,
    buffer: String,
    state: TokenizerState,
    emitted_token: Option<Token>,
    last_tokenized: Option<Token>,
}

impl<'a> Tokenizer<'a> {
    /// Create a new tokenizer from an input string.
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            pos: 0,
            buffer: String::new(),
            state: TokenizerState::Data,
            emitted_token: None,
            last_tokenized: None,
        }
    }

    /// Returns the most recently emitted token.
    pub fn last_tokenized_token(&self) -> Option<&Token> {
        self.last_tokenized.as_ref()
    }

    /// Consume input and return the next token, if any.
    pub fn next_token(&mut self) -> Option<Token> {
        // Read `unread_token` without logging.
        if let Some(token) = self.emitted_token.take() {
            self.last_tokenized = Some(token.clone());
            return Some(token);
        }

        while self.pos < self.input.len() {
            let c = self.next_char();

            match self.state {
                TokenizerState::Data => self.state_data(c),
                TokenizerState::Ident => self.state_ident(c),
                TokenizerState::Number => self.state_number(c),
                TokenizerState::StringDouble | TokenizerState::StringSingle => self.state_string(c),
                TokenizerState::Hash => self.state_hash(c),
                TokenizerState::AtKeyword => self.state_at_keyword(c),
                TokenizerState::Comment | TokenizerState::CommentEndStar => self.state_comment(c),
            }

            if let Some(token) = self.emitted_token.take() {
                self.last_tokenized = Some(token.clone());
                log::debug!(target: "CssTokenizer", "Tokenized token: {:?}", token);
                return Some(token);
            }
        }

        None
    }

    /// Read the next UTF-8 character and advance the cursor.
    fn next_char(&mut self) -> char {
        let c = self.input[self.pos..].chars().next().unwrap();
        self.pos += c.len_utf8();
        c
    }

    /// Step back one character (used for re-consuming).
    fn unread_char(&mut self, c: char) {
        self.pos -= c.len_utf8();
    }

    /// Finalize the current token and reset the buffer.
    fn emit(&mut self, token: Token) {
        self.buffer.clear();
        self.emitted_token = Some(token);
        self.state = TokenizerState::Data;
    }

    /// Make last_tokenized token unread.
    pub fn unread_token(&mut self) {
        if let Some(ref token) = self.last_tokenized {
            self.emitted_token = Some(token.clone());
        }
    }

    fn state_data(&mut self, c: char) {
        match c {
            c if c.is_whitespace() => self.emit(Token::Whitespace),

            c if c.is_ascii_alphabetic() || c == '_' => {
                self.buffer.push(c);
                self.state = TokenizerState::Ident;
            }

            c if c.is_ascii_digit() => {
                self.buffer.push(c);
                self.state = TokenizerState::Number;
            }

            '"' => {
                self.buffer.clear();
                self.state = TokenizerState::StringDouble;
            }

            '\'' => {
                self.buffer.clear();
                self.state = TokenizerState::StringSingle;
            }

            '#' => {
                self.buffer.clear();
                self.state = TokenizerState::Hash;
            }

            '@' => {
                self.buffer.clear();
                self.state = TokenizerState::AtKeyword;
            }

            '/' if self.input[self.pos..].starts_with('*') => {
                self.pos += 1; // consume '*'
                self.buffer.clear();
                self.state = TokenizerState::Comment;
            }

            ':' => self.emit(Token::Colon),
            ';' => self.emit(Token::Semicolon),
            ',' => self.emit(Token::Comma),
            '{' => self.emit(Token::LeftBrace),
            '}' => self.emit(Token::RightBrace),

            '(' => {
                if let Some(Token::Ident(name)) = self.last_tokenized.take() {
                    if let Some(func) = self.consume_function(name) {
                        self.emit(func);
                    }
                } else {
                    self.emit(Token::Delim('('));
                }
            }

            ')' => self.emit(Token::RightParen),
            '[' => self.emit(Token::LeftBracket),
            ']' => self.emit(Token::RightBracket),

            '<' if self.input[self.pos..].starts_with("!--") => {
                self.pos += 3;
                self.emit(Token::CDO);
            }

            '-' if self.input[self.pos..].starts_with("->") => {
                self.pos += 2;
                self.emit(Token::CDC);
            }

            _ => self.emit(Token::Delim(c)),
        }
    }

    fn state_ident(&mut self, c: char) {
        if c.is_alphanumeric() || c == '-' || c == '_' {
            self.buffer.push(c);
        } else {
            let ident = self.buffer.clone();
            self.emit(Token::Ident(ident));
            self.unread_char(c);
        }
    }

    fn state_number(&mut self, c: char) {
        if c.is_ascii_digit() || c == '.' {
            self.buffer.push(c);
            return;
        }

        let value = self.buffer.parse::<f32>().unwrap_or(0.0);

        if c == '%' {
            self.emit(Token::Percentage(value));
        } else if c.is_ascii_alphabetic() {
            let mut unit = String::new();
            unit.push(c);

            while self.pos < self.input.len() {
                let next = self.input[self.pos..].chars().next().unwrap();
                if next.is_ascii_alphabetic() {
                    self.pos += next.len_utf8();
                    unit.push(next);
                } else {
                    break;
                }
            }

            self.emit(Token::Dimension(value, unit));
        } else {
            self.emit(Token::Number(value));
            self.unread_char(c);
        }
    }

    fn state_string(&mut self, c: char) {
        let quote = if self.state == TokenizerState::StringDouble {
            '"'
        } else {
            '\''
        };

        if c == quote {
            let s = self.buffer.clone();
            self.emit(Token::StringLiteral(s));
        } else {
            self.buffer.push(c);
        }
    }

    fn state_hash(&mut self, c: char) {
        if c.is_alphanumeric() || c == '-' {
            self.buffer.push(c);
        } else {
            let hash = self.buffer.clone();
            self.emit(Token::Hash(hash));
            self.unread_char(c);
        }
    }

    fn state_at_keyword(&mut self, c: char) {
        if c.is_alphanumeric() || c == '-' {
            self.buffer.push(c);
        } else {
            let kw = self.buffer.clone();
            self.emit(Token::AtKeyword(kw));
            self.unread_char(c);
        }
    }

    fn state_comment(&mut self, c: char) {
        match self.state {
            TokenizerState::Comment => {
                if c == '*' {
                    self.state = TokenizerState::CommentEndStar;
                } else {
                    self.buffer.push(c);
                }
            }
            TokenizerState::CommentEndStar => {
                if c == '/' {
                    let comment = self.buffer.clone();
                    self.emit(Token::Comment(comment));
                } else {
                    self.buffer.push('*');
                    self.buffer.push(c);
                    self.state = TokenizerState::Comment;
                }
            }
            _ => {}
        }
    }

    fn consume_function(&mut self, name: String) -> Option<Token> {
        // url() は Raw
        if name.eq_ignore_ascii_case("url") {
            let mut raw = String::new();

            while self.pos < self.input.len() {
                let c = self.next_char();
                if c == ')' {
                    break;
                }
                raw.push(c);
            }

            return Some(Token::Function {
                name,
                value: FunctionValue::Raw(raw.trim().to_string()),
            });
        }

        // それ以外は Tokens
        let mut tokens = Vec::new();
        let mut depth = 1;

        while self.pos < self.input.len() {
            let token = self.next_token()?;

            match token {
                Token::Function { .. } => {
                    depth += 1;
                    tokens.push(token);
                }
                Token::RightParen => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => tokens.push(token),
            }
        }

        Some(Token::Function {
            name,
            value: FunctionValue::Tokens(tokens),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_css_tokenize_basic() {
        let mut t = Tokenizer::new("body { color: red; }");
        let tokens: Vec<_> = std::iter::from_fn(|| t.next_token()).collect();

        assert_eq!(
            tokens,
            vec![
                Token::Ident("body".into()),
                Token::Whitespace,
                Token::LeftBrace,
                Token::Whitespace,
                Token::Ident("color".into()),
                Token::Colon,
                Token::Whitespace,
                Token::Ident("red".into()),
                Token::Semicolon,
                Token::Whitespace,
                Token::RightBrace,
            ]
        );
    }

    #[test]
    fn test_tokenize_dimension_and_percent() {
        let mut t = Tokenizer::new("margin: 10px 50%;");
        let tokens: Vec<_> = std::iter::from_fn(|| t.next_token()).collect();

        assert_eq!(
            tokens,
            vec![
                Token::Ident("margin".into()),
                Token::Colon,
                Token::Whitespace,
                Token::Dimension(10.0, "px".into()),
                Token::Whitespace,
                Token::Percentage(50.0),
                Token::Semicolon,
            ]
        );
    }
}
