use super::util::decode_entity;

/// Represents a single HTML attribute
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub value: String,
}

/// HTML tokens emitted by the tokenizer
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Doctype {
        name: Option<String>,
        public_id: Option<String>,
        system_id: Option<String>,
        force_quirks: bool,
    },
    StartTag {
        name: String,
        attributes: Vec<Attribute>,
        self_closing: bool,
    },
    EndTag {
        name: String,
    },
    Comment(String),
    Text(String),
}

/// Represents the internal state of the tokenizer
#[derive(Debug, PartialEq)]
pub enum TokenizerState {
    Data,
    EscapeDecoding,
    TagOpen,
    EndTagOpen,
    TagName,
    BeforeAttributeName,
    AttributeName,
    AfterAttributeName,
    BeforeAttributeValue,
    AttributeValueDoubleQuoted,
    AttributeValueSingleQuoted,
    AttributeValueUnquoted,
    SelfClosingStartTag,
    CommentStartDash,
    Comment,
    CommentEndDash,
    CommentEnd,
    BogusComment,
    Doctype,
    DoctypeName,
    BeforeDoctypePublicId,
    DoctypePublicIdWithSingleQuote,
    DoctypePublicIdWithDoubleQuote,
    AfterDoctypePublicId,
    DoctypeSystemId,
    BogusDoctype,
}

impl TokenizerState {
    /// Returns true if the current state is a doctype-related state
    fn is_doctype(&self) -> bool {
        matches!(
            self,
            TokenizerState::Doctype
                | TokenizerState::DoctypeName
                | TokenizerState::BeforeDoctypePublicId
                | TokenizerState::DoctypePublicIdWithSingleQuote
                | TokenizerState::DoctypePublicIdWithDoubleQuote
                | TokenizerState::AfterDoctypePublicId
                | TokenizerState::DoctypeSystemId
                | TokenizerState::BogusDoctype
        )
    }

    /// Returns true if the current state is a comment-related state
    fn is_comment(&self) -> bool {
        matches!(
            self,
            TokenizerState::Comment
                | TokenizerState::CommentStartDash
                | TokenizerState::CommentEndDash
                | TokenizerState::CommentEnd
                | TokenizerState::BogusComment
        )
    }
}

/// HTML tokenizer implementation
pub struct Tokenizer<'a> {
    input: &'a str,
    pos: usize,
    token: Option<Token>,
    state: TokenizerState,
    current_token: Option<Token>,
    current_attribute: Option<Attribute>,
    buffer: String,
}

impl<'a> Tokenizer<'a> {
    /// Creates a new tokenizer for the given input
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            pos: 0,
            token: None,
            state: TokenizerState::Data,
            current_token: None,
            current_attribute: None,
            buffer: String::new(),
        }
    }

    /// Returns the next character from input and advances the position
    fn next_char(&mut self) -> Option<char> {
        if self.pos >= self.input.len() {
            None
        } else {
            let c = self.input[self.pos..].chars().next().unwrap();
            self.pos += c.len_utf8();
            Some(c)
        }
    }

    /// Emits the current token and clears the buffer
    fn commit_token(&mut self) {
        self.token = self.current_token.take();
        self.buffer.clear();
    }

    /// Pushes the current attribute to the start tag if exists
    fn push_current_attribute(&mut self) {
        if let (Some(attr), Some(Token::StartTag { attributes, .. })) =
            (self.current_attribute.take(), &mut self.current_token)
        {
            attributes.push(attr);
        }
    }

    /// Debug log for emitted tokens
    fn debug_emit(&self, token: &Token) {
        #[cfg(debug_assertions)]
        match token {
            Token::StartTag { name, .. } => {
                log::debug!(target:"HtmlTokenizer::EmitToken::TagStart", "Emitting token: {name}, Pos: {}", self.pos)
            }
            Token::EndTag { name } => {
                log::debug!(target:"HtmlTokenizer::EmitToken::TagEnd", "Emitting token: {name}, Pos: {}", self.pos)
            }
            Token::Comment(comment) => {
                log::debug!(target:"HtmlTokenizer::EmitToken::Comment", "Emitting token: {}, Pos: {}", comment, self.pos)
            }
            Token::Text(text) => {
                log::debug!(target:"HtmlTokenizer::EmitToken::Text", "Emitting token: `{text}`, Pos: {}", self.pos)
            }
            _ => {}
        }
    }

    /// Returns the next token if available
    pub fn next_token(&mut self) -> Option<Token> {
        while let Some(c) = self.next_char() {
            log::debug!(target:"HtmlTokenizer::Char", "State: {:?}, Char: '{}'", self.state, c);

            match self.state {
                TokenizerState::Data => self.state_data(c),
                TokenizerState::EscapeDecoding => self.state_escape_decoding(c),
                _ if self.state.is_doctype() => self.state_doctype(c),
                TokenizerState::TagOpen => self.state_tag_open(c),
                TokenizerState::TagName => self.state_tag_name(c),
                TokenizerState::BeforeAttributeName => self.state_before_attribute_name(c),
                TokenizerState::AttributeName => self.state_attribute_name(c),
                TokenizerState::BeforeAttributeValue => self.state_before_attribute_value(c),
                TokenizerState::AttributeValueDoubleQuoted
                | TokenizerState::AttributeValueSingleQuoted => {
                    self.state_attribute_value_quoted(c)
                }
                TokenizerState::AfterAttributeName => self.state_after_attribute_name(c),
                TokenizerState::AttributeValueUnquoted => self.state_attribute_value_unquoted(c),
                TokenizerState::SelfClosingStartTag => self.state_self_closing_start_tag(c),
                TokenizerState::EndTagOpen => self.state_end_tag_open(c),
                _ if self.state.is_comment() => self.state_comment(c),
                _ => {
                    log::warn!(target:"HtmlTokenizer::State", "Unimplemented state: {:?}, returning to Data state", self.state);
                    self.state = TokenizerState::Data;
                }
            }

            if let Some(token) = self.token.take() {
                self.debug_emit(&token);
                return Some(token);
            }
        }

        // End of input: commit remaining current_token if exists
        if self.current_token.is_some() {
            self.commit_token();
            return self.token.take();
        }

        // Emit BogusComment if input ended while in comment
        if self.state.is_comment() {
            self.state = TokenizerState::BogusComment;
            self.commit_token();
            return self.token.take();
        }

        None
    }

    // --- State handlers ---
    fn state_data(&mut self, c: char) {
        match c {
            '<' => {
                self.commit_token();
                self.state = TokenizerState::TagOpen;
            }
            '&' => {
                self.buffer.push('&');
                self.state = TokenizerState::EscapeDecoding;
            }
            _ => {
                self.buffer.push(c);
                match &mut self.current_token {
                    Some(Token::Text(text)) => text.push(c),
                    _ => self.current_token = Some(Token::Text(c.to_string())),
                }
            }
        }
    }

    fn state_escape_decoding(&mut self, c: char) {
        if c == ';' {
            let mut iter = self.buffer.rsplitn(2, '&');
            let entity = iter.next().unwrap_or("");

            let decoded = decode_entity(entity).unwrap_or_else(|| format!("&{};", entity));

            match &mut self.current_token {
                Some(Token::Text(text)) => text.push_str(&decoded),
                _ => self.current_token = Some(Token::Text(decoded)),
            }

            self.buffer.clear();
            self.state = TokenizerState::Data;
        } else {
            self.buffer.push(c);
        }
    }

    fn state_tag_open(&mut self, c: char) {
        match c {
            '/' => self.state = TokenizerState::EndTagOpen,
            '!' => {
                if self.input[self.pos..].starts_with('-') {
                    self.pos += 1;
                    self.state = TokenizerState::CommentStartDash;
                } else if self.input[self.pos..].to_lowercase().starts_with("doctype") {
                    self.pos += 7;
                    self.state = TokenizerState::Doctype;
                    self.current_token = Some(Token::Doctype {
                        name: None,
                        public_id: None,
                        system_id: None,
                        force_quirks: false,
                    });
                } else {
                    self.state = TokenizerState::BogusComment;
                }
            }
            c if c.is_ascii_alphabetic() => {
                self.state = TokenizerState::TagName;
                self.buffer.push(c);
                self.current_token = Some(Token::StartTag {
                    name: c.to_string(),
                    attributes: Vec::new(),
                    self_closing: false,
                });
            }
            _ => {
                self.buffer.push('<');
                self.buffer.push(c);
                match &mut self.current_token {
                    Some(Token::Text(text)) => {
                        text.push('<');
                        text.push(c);
                    }
                    _ => self.current_token = Some(Token::Text(format!("<{c}"))),
                }
                self.state = TokenizerState::Data;
            }
        }
    }

    fn state_tag_name(&mut self, c: char) {
        match c {
            c if c.is_whitespace() => self.state = TokenizerState::BeforeAttributeName,
            '/' => self.state = TokenizerState::SelfClosingStartTag,
            '>' => {
                self.commit_token();
                self.state = TokenizerState::Data;
            }
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':' => {
                self.buffer.push(c);
                match &mut self.current_token {
                    Some(Token::StartTag { name, .. }) => name.push(c),
                    Some(Token::EndTag { name }) => name.push(c),
                    _ => {}
                }
            }
            _ => {
                self.commit_token();
                self.state = TokenizerState::Data;
            }
        }
    }

    fn state_before_attribute_name(&mut self, c: char) {
        match c {
            c if c.is_whitespace() => {}
            '/' => self.state = TokenizerState::SelfClosingStartTag,
            '>' => {
                self.commit_token();
                self.state = TokenizerState::Data;
            }
            c if c.is_ascii_alphanumeric() => {
                self.state = TokenizerState::AttributeName;
                self.buffer.push(c);
                self.current_attribute = Some(Attribute {
                    name: c.to_string(),
                    value: String::new(),
                });
            }
            _ => {}
        }
    }

    fn state_attribute_name(&mut self, c: char) {
        match c {
            c if c.is_whitespace() => self.state = TokenizerState::AfterAttributeName,
            '=' => self.state = TokenizerState::BeforeAttributeValue,
            '/' => {
                self.push_current_attribute();
                self.state = TokenizerState::SelfClosingStartTag;
            }
            '>' => {
                self.push_current_attribute();
                self.commit_token();
                self.state = TokenizerState::Data;
            }
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':' => {
                self.buffer.push(c);
                if let Some(attr) = &mut self.current_attribute {
                    attr.name.push(c);
                }
            }
            _ => {}
        }
    }

    fn state_before_attribute_value(&mut self, c: char) {
        match c {
            c if c.is_whitespace() => {}
            '"' => self.state = TokenizerState::AttributeValueDoubleQuoted,
            '\'' => self.state = TokenizerState::AttributeValueSingleQuoted,
            '>' => {
                self.push_current_attribute();
                self.commit_token();
                self.state = TokenizerState::Data;
            }
            _ => {
                self.state = TokenizerState::AttributeValueUnquoted;
                if let Some(attr) = &mut self.current_attribute {
                    attr.value.push(c);
                }
            }
        }
    }

    fn state_attribute_value_quoted(&mut self, c: char) {
        match (&self.state, c) {
            (&TokenizerState::AttributeValueDoubleQuoted, '"')
            | (&TokenizerState::AttributeValueSingleQuoted, '\'') => {
                self.push_current_attribute();
                self.state = TokenizerState::AfterAttributeName;
            }
            _ => {
                if let Some(attr) = &mut self.current_attribute {
                    attr.value.push(c);
                }
            }
        }
    }

    fn state_after_attribute_name(&mut self, c: char) {
        match c {
            c if c.is_whitespace() => {}
            '/' => self.state = TokenizerState::SelfClosingStartTag,
            '>' => {
                self.commit_token();
                self.state = TokenizerState::Data;
            }
            c if c.is_ascii_alphanumeric() => {
                self.state = TokenizerState::AttributeName;
                self.buffer.push(c);
                self.current_attribute = Some(Attribute {
                    name: c.to_string(),
                    value: String::new(),
                });
            }
            _ => {}
        }
    }

    fn state_attribute_value_unquoted(&mut self, c: char) {
        match c {
            c if c.is_whitespace() => {
                self.push_current_attribute();
                self.state = TokenizerState::BeforeAttributeName;
            }
            '>' => {
                self.push_current_attribute();
                self.commit_token();
                self.state = TokenizerState::Data;
            }
            _ => {
                if let Some(attr) = &mut self.current_attribute {
                    attr.value.push(c);
                }
            }
        }
    }

    fn state_self_closing_start_tag(&mut self, c: char) {
        match c {
            '>' => {
                if let Some(Token::StartTag { self_closing, .. }) = &mut self.current_token {
                    *self_closing = true;
                }
                self.commit_token();
                self.state = TokenizerState::Data;
            }
            _ => self.state = TokenizerState::Data,
        }
    }

    fn state_end_tag_open(&mut self, c: char) {
        match c {
            c if c.is_ascii_alphabetic() => {
                self.state = TokenizerState::TagName;
                self.buffer.push(c);
                self.current_token = Some(Token::EndTag {
                    name: c.to_string(),
                });
            }
            _ => self.state = TokenizerState::Data,
        }
    }

    fn state_comment(&mut self, c: char) {
        match self.state {
            TokenizerState::CommentStartDash => {
                if c == '-' {
                    self.state = TokenizerState::Comment;
                    self.current_token = Some(Token::Comment(String::new()));
                } else {
                    self.state = TokenizerState::BogusComment;
                }
            }
            TokenizerState::Comment => {
                if c == '-' {
                    self.state = TokenizerState::CommentEndDash;
                } else if let Some(Token::Comment(comment)) = &mut self.current_token {
                    comment.push(c);
                }
            }
            TokenizerState::CommentEndDash => {
                if c == '-' {
                    self.state = TokenizerState::CommentEnd;
                } else {
                    self.state = TokenizerState::Comment;
                    if let Some(Token::Comment(comment)) = &mut self.current_token {
                        comment.push('-');
                        comment.push(c);
                    }
                }
            }
            TokenizerState::CommentEnd => {
                if c == '>' {
                    self.commit_token();
                    self.state = TokenizerState::Data;
                } else {
                    self.state = TokenizerState::Comment;
                    if let Some(Token::Comment(comment)) = &mut self.current_token {
                        comment.push_str("--");
                        comment.push(c);
                    }
                }
            }
            _ => {}
        }
    }

    fn state_doctype(&mut self, c: char) {
        match c {
            c if c.is_whitespace() => match self.state {
                TokenizerState::Doctype => self.state = TokenizerState::DoctypeName,
                TokenizerState::DoctypeName => {
                    if self.input[self.pos..].to_lowercase().starts_with("public")
                        || self.input[self.pos..].to_lowercase().starts_with("system")
                    {
                        self.pos += 6;
                        self.state = TokenizerState::BeforeDoctypePublicId;
                    }
                }
                TokenizerState::AfterDoctypePublicId => {
                    self.state = TokenizerState::DoctypeSystemId;
                }
                _ => {}
            },
            '>' => {
                if let Some(Token::Doctype { force_quirks, .. }) = &mut self.current_token
                    && self.state == TokenizerState::BogusDoctype
                {
                    *force_quirks = true;
                }
                self.commit_token();
                self.state = TokenizerState::Data;
            }
            _ => {
                self.buffer.push(c);
                match self.state {
                    TokenizerState::Doctype => self.state = TokenizerState::BogusDoctype,
                    TokenizerState::DoctypeName => {
                        if let Some(Token::Doctype { name, .. }) = &mut self.current_token {
                            if name.is_none() {
                                *name = Some(c.to_string());
                            } else if let Some(n) = name {
                                n.push(c);
                            }
                        }
                    }
                    TokenizerState::BeforeDoctypePublicId => {
                        match c {
                            '"' => self.state = TokenizerState::DoctypePublicIdWithDoubleQuote,
                            '\'' => self.state = TokenizerState::DoctypePublicIdWithSingleQuote,
                            _ if c.is_whitespace() => {}
                            _ => self.state = TokenizerState::BogusDoctype,
                        }
                        if let Some(Token::Doctype { public_id, .. }) = &mut self.current_token {
                            *public_id = Some(c.to_string());
                        }
                    }
                    TokenizerState::DoctypePublicIdWithSingleQuote
                    | TokenizerState::DoctypePublicIdWithDoubleQuote => {
                        if let Some(Token::Doctype { public_id, .. }) = &mut self.current_token
                            && let Some(pid) = public_id
                        {
                            pid.push(c);
                        }
                        if (self.state == TokenizerState::DoctypePublicIdWithSingleQuote
                            && c == '\'')
                            || (self.state == TokenizerState::DoctypePublicIdWithDoubleQuote
                                && c == '"')
                        {
                            self.state = TokenizerState::AfterDoctypePublicId;
                        }
                    }
                    TokenizerState::DoctypeSystemId => {
                        if let Some(Token::Doctype { system_id, .. }) = &mut self.current_token {
                            if system_id.is_none() {
                                *system_id = Some(c.to_string());
                            } else if let Some(sid) = system_id {
                                sid.push(c);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_tokens(input: &str) -> Vec<Token> {
        let mut tokenizer = Tokenizer::new(input);
        let mut tokens = Vec::new();
        while let Some(token) = tokenizer.next_token() {
            tokens.push(token);
        }
        tokens
    }

    #[test]
    fn test_text_node() {
        let input = "Hello, world!";
        let tokens = collect_tokens(input);
        assert_eq!(tokens, vec![Token::Text("Hello, world!".to_string())]);
    }

    #[test]
    fn test_simple_tag() {
        let input = "<div></div>";
        let tokens = collect_tokens(input);
        assert_eq!(
            tokens,
            vec![
                Token::StartTag {
                    name: "div".to_string(),
                    attributes: vec![],
                    self_closing: false
                },
                Token::EndTag {
                    name: "div".to_string()
                }
            ]
        );
    }

    #[test]
    fn test_tag_with_attributes() {
        let input = r#"<a href="https://example.com" target='_blank'>Link</a>"#;
        let tokens = collect_tokens(input);
        assert_eq!(
            tokens,
            vec![
                Token::StartTag {
                    name: "a".to_string(),
                    attributes: vec![
                        Attribute {
                            name: "href".to_string(),
                            value: "https://example.com".to_string()
                        },
                        Attribute {
                            name: "target".to_string(),
                            value: "_blank".to_string()
                        },
                    ],
                    self_closing: false
                },
                Token::Text("Link".to_string()),
                Token::EndTag {
                    name: "a".to_string()
                }
            ]
        );
    }

    #[test]
    fn test_self_closing_tag() {
        let input = "<img src='image.png'/>";
        let tokens = collect_tokens(input);
        assert_eq!(
            tokens,
            vec![Token::StartTag {
                name: "img".to_string(),
                attributes: vec![Attribute {
                    name: "src".to_string(),
                    value: "image.png".to_string()
                }],
                self_closing: true
            }]
        );
    }

    #[test]
    fn test_comment() {
        let input = "<!-- This is a comment -->";
        let tokens = collect_tokens(input);
        assert_eq!(
            tokens,
            vec![Token::Comment(" This is a comment ".to_string())]
        );
    }

    #[test]
    fn test_doctype() {
        let input = "<!DOCTYPE html>";
        let tokens = collect_tokens(input);
        assert_eq!(
            tokens,
            vec![Token::Doctype {
                name: Some("html".to_string()),
                public_id: None,
                system_id: None,
                force_quirks: false
            }]
        );
    }

    #[test]
    fn test_escape_entity() {
        let input = "Hello &amp; goodbye";
        let tokens = collect_tokens(input);
        assert_eq!(tokens, vec![Token::Text("Hello & goodbye".to_string())]);
    }

    #[test]
    fn test_nested_tags() {
        let input = "<div><span>Text</span></div>";
        let tokens = collect_tokens(input);
        assert_eq!(
            tokens,
            vec![
                Token::StartTag {
                    name: "div".to_string(),
                    attributes: vec![],
                    self_closing: false
                },
                Token::StartTag {
                    name: "span".to_string(),
                    attributes: vec![],
                    self_closing: false
                },
                Token::Text("Text".to_string()),
                Token::EndTag {
                    name: "span".to_string()
                },
                Token::EndTag {
                    name: "div".to_string()
                },
            ]
        );
    }
}
