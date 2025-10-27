#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Ident(String),   // color, width, etc.
    String(String),  // "Roboto"
    Number(f32),     // 1.5, 10, etc.
    Hash(String),    // #fff
    Comment(String), // /* comment */
    Delim(char),     // { } : ; ( ) , など
    Colon,
    Semicolon,
    Whitespace,
    AtKeyword(String), // @media, @import, etc.
    EOF,
}

#[derive(Debug, PartialEq, Clone)]
pub enum TokenizerState {
    Data,
    Ident,
    Number,
    StringDouble,
    StringSingle,
    Hash,
    AtKeyword,
    CommentStart,
    Comment,
    CommentEndDash,
    CommentEnd,
}

pub struct Tokenizer<'a> {
    input: &'a str,
    pos: usize,
    buffer: String,
    state: TokenizerState,
    current_token: Option<Token>,
    pub token: Option<Token>,
}

impl<'a> Tokenizer<'a> {
    pub fn new(input: &'a str) -> Self {
        Tokenizer {
            input,
            pos: 0,
            buffer: String::new(),
            state: TokenizerState::Data,
            current_token: None,
            token: None,
        }
    }

    pub fn next_token(&mut self) -> Option<Token> {
        while self.pos < self.input.len() {
            let c = self.input[self.pos..].chars().next().unwrap();
            self.pos += c.len_utf8();

            //println!("State: {:?}, Char: '{}'", self.state, c);

            match self.state {
                TokenizerState::Data => self.state_data(c),
                TokenizerState::Ident => self.state_ident(c),
                TokenizerState::Number => self.state_number(c),
                TokenizerState::StringDouble | TokenizerState::StringSingle => self.state_string(c),
                TokenizerState::Hash => self.state_hash(c),
                TokenizerState::AtKeyword => self.state_at_keyword(c),
                _ if self.state_is_comment() => self.state_comment(c),
                _ => {}
            }

            if let Some(token) = self.token.take() {
                return Some(token);
            }
        }
        None
    }

    fn state_is_comment(&self) -> bool {
        matches!(
            self.state,
            TokenizerState::CommentStart
                | TokenizerState::Comment
                | TokenizerState::CommentEndDash
                | TokenizerState::CommentEnd
        )
    }

    fn commit_token(&mut self) {
        self.token = self.current_token.take();
        self.buffer.clear();
    }

    fn state_data(&mut self, c: char) {
        match c {
            c if c.is_whitespace() => {
                self.current_token = Some(Token::Whitespace);
                self.commit_token();
            }
            c if c.is_ascii_alphabetic() => {
                self.buffer.push(c);
                self.state = TokenizerState::Ident;
                self.current_token = Some(Token::Ident(String::new()));
            }
            c if c.is_ascii_digit() => {
                self.buffer.push(c);
                self.state = TokenizerState::Number;
                self.current_token = Some(Token::Number(0.0));
            }
            '"' => self.state = TokenizerState::StringDouble,
            '\'' => self.state = TokenizerState::StringSingle,
            '#' => {
                self.state = TokenizerState::Hash;
                self.buffer.clear();
            }
            '@' => {
                self.state = TokenizerState::AtKeyword;
                self.buffer.clear();
            }
            '/' if self.input[self.pos..].starts_with('*') => {
                self.state = TokenizerState::CommentStart;
            }
            ':' => {
                self.current_token = Some(Token::Colon);
                self.commit_token();
            }
            ';' => {
                self.current_token = Some(Token::Semicolon);
                self.commit_token();
            }
            c if "{}(),".contains(c) => {
                self.current_token = Some(Token::Delim(c));
                self.commit_token();
            }
            _ => {
                self.current_token = Some(Token::Delim(c));
                self.commit_token();
            }
        }
    }

    fn state_ident(&mut self, c: char) {
        if c.is_alphanumeric() || c == '-' || c == '_' {
            self.buffer.push(c);
        } else {
            self.current_token = Some(Token::Ident(self.buffer.clone()));
            self.commit_token();
            self.state = TokenizerState::Data;
            self.pos -= c.len_utf8(); // unread
        }
    }

    fn state_number(&mut self, c: char) {
        if c.is_ascii_digit() || c == '.' {
            self.buffer.push(c);
        } else {
            let n = self.buffer.parse::<f32>().unwrap_or(0.0);
            self.current_token = Some(Token::Number(n));
            self.commit_token();
            self.state = TokenizerState::Data;
            self.pos -= c.len_utf8();
        }
    }

    fn state_string(&mut self, c: char) {
        let quote = if self.state == TokenizerState::StringDouble {
            '"'
        } else {
            '\''
        };

        match c {
            ch if ch == quote => {
                self.current_token = Some(Token::String(self.buffer.clone()));
                self.commit_token();
                self.state = TokenizerState::Data;
            }
            _ => self.buffer.push(c),
        }
    }

    fn state_hash(&mut self, c: char) {
        if c.is_alphanumeric() {
            self.buffer.push(c);
        } else {
            self.current_token = Some(Token::Hash(self.buffer.clone()));
            self.commit_token();
            self.state = TokenizerState::Data;
            self.pos -= c.len_utf8();
        }
    }

    fn state_at_keyword(&mut self, c: char) {
        if c.is_alphanumeric() || c == '-' {
            self.buffer.push(c);
        } else {
            self.current_token = Some(Token::AtKeyword(self.buffer.clone()));
            self.commit_token();
            self.state = TokenizerState::Data;
            self.pos -= c.len_utf8();
        }
    }

    fn state_comment(&mut self, c: char) {
        match self.state {
            TokenizerState::CommentStart => {
                if c == '*' {
                    self.state = TokenizerState::Comment;
                } else {
                    self.state = TokenizerState::Data;
                }
            }
            TokenizerState::Comment => {
                if c == '*' {
                    self.state = TokenizerState::CommentEndDash;
                } else {
                    self.buffer.push(c);
                }
            }
            TokenizerState::CommentEndDash => {
                if c == '/' {
                    self.current_token = Some(Token::Comment(self.buffer.clone()));
                    self.commit_token();
                    self.state = TokenizerState::Data;
                } else {
                    self.state = TokenizerState::Comment;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_css_tokenize_basic() {
        let mut t = Tokenizer::new("body { color: red; }");
        let mut tokens = Vec::new();
        while let Some(tok) = t.next_token() {
            tokens.push(tok);
        }

        assert_eq!(
            tokens,
            vec![
                Token::Ident("body".into()),
                Token::Whitespace,
                Token::Delim('{'),
                Token::Whitespace,
                Token::Ident("color".into()),
                Token::Colon,
                Token::Whitespace,
                Token::Ident("red".into()),
                Token::Semicolon,
                Token::Whitespace,
                Token::Delim('}')
            ]
        );
    }
}
