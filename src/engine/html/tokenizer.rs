#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub value: String,
}

#[allow(dead_code)]
#[derive(Debug, PartialEq)]
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

#[allow(dead_code)]
#[derive(Debug, PartialEq)]
pub enum TokenizerState {
    Data,
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

#[allow(dead_code)]
pub struct Tokenizer<'a> {
    input: &'a str,
    pos: usize,
    token: Option<Token>,
    state: TokenizerState,
    current_token: Option<Token>,
    current_attribute: Option<Attribute>,
    buffer: String,
}

#[allow(dead_code)]
impl TokenizerState {
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

impl<'a> Tokenizer<'a> {
    pub fn new(input: &'a str) -> Self {
        Tokenizer {
            input,
            pos: 0,
            token: None,
            state: TokenizerState::Data,
            current_token: None,
            current_attribute: None,
            buffer: String::new(),
        }
    }

    pub fn next_token(&mut self) -> Option<Token> {
        while self.pos < self.input.len() {
            let c = self.input[self.pos..].chars().next().unwrap();
            self.pos += c.len_utf8();

            //print!("State: {:?}, Char: '{}'\n", self.state, c);

            match self.state {
                TokenizerState::Data => self.state_data(c),
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
                    // 未実装の状態は無視してData状態に戻る
                    self.state = TokenizerState::Data;
                }
            }

            if let Some(token) = self.token.take() {
                return Some(token);
            }
        }

        // BogusComment処理
        if self.state.is_comment() {
            self.state = TokenizerState::BogusComment;
            self.commit_token();
            return self.token.take();
        }

        None
    }

    fn commit_token(&mut self) {
        self.token = self.current_token.take();
        self.buffer.clear();
    }

    fn state_data(&mut self, c: char) {
        match c {
            '<' => {
                self.commit_token();
                self.state = TokenizerState::TagOpen
            }
            '&' => {
                log::warn!("エスケープ処理（未実装）");
            }
            _ => {
                self.buffer.push(c);
                if let Some(Token::Text(ref mut text)) = self.current_token {
                    text.push(c);
                } else {
                    self.current_token = Some(Token::Text(c.to_string()));
                }
            }
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
                // cがアルファベットの場合
                self.state = TokenizerState::TagName;
                self.buffer.push(c);
                self.current_token = Some(Token::StartTag {
                    name: c.to_string(),
                    attributes: Vec::new(),
                    self_closing: false,
                });
            }
            _ => {
                // テキストノードとして処理
                self.buffer.push('<');
                self.buffer.push(c);
                if let Some(Token::Text(ref mut text)) = self.current_token {
                    text.push('<');
                    text.push(c);
                } else {
                    self.current_token = Some(Token::Text(format!("<{c}")));
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
                if let Some(Token::StartTag { ref mut name, .. }) = self.current_token {
                    name.push(c);
                } else if let Some(Token::EndTag { ref mut name }) = self.current_token {
                    name.push(c);
                }
            }
            _ => {
                // 不正な文字
                // タグの終了として処理
                self.commit_token();
                self.state = TokenizerState::Data;
            }
        }
    }

    fn state_doctype(&mut self, c: char) {
        match c {
            c if c.is_whitespace() => match self.state {
                TokenizerState::Doctype => self.state = TokenizerState::DoctypeName,
                TokenizerState::DoctypeName => {
                    if self.input[self.pos..].to_lowercase().starts_with("public") {
                        self.pos += 6;
                        self.state = TokenizerState::BeforeDoctypePublicId
                    } else if self.input[self.pos..].to_lowercase().starts_with("system") {
                        self.pos += 6;
                        self.state = TokenizerState::BeforeDoctypePublicId
                    }
                }
                TokenizerState::AfterDoctypePublicId => {
                    self.state = TokenizerState::DoctypeSystemId
                }
                _ => {}
            },
            '>' => {
                if self.state == TokenizerState::BogusDoctype {
                    if let Some(Token::Doctype {
                        ref mut force_quirks,
                        ..
                    }) = self.current_token
                    {
                        *force_quirks = true;
                    }
                }
                self.commit_token();
                self.state = TokenizerState::Data;
            }
            _ => {
                self.buffer.push(c);
                match self.state {
                    TokenizerState::Doctype => {
                        self.state = TokenizerState::BogusDoctype;
                    }
                    TokenizerState::DoctypeName => {
                        if let Some(Token::Doctype { ref mut name, .. }) = self.current_token {
                            if name.is_none() {
                                *name = Some(c.to_string());
                            } else if let Some(ref mut n) = name {
                                n.push(c);
                            }
                        }
                    }
                    TokenizerState::BeforeDoctypePublicId => {
                        match c {
                            '"' => self.state = TokenizerState::DoctypePublicIdWithDoubleQuote,
                            '\'' => self.state = TokenizerState::DoctypePublicIdWithSingleQuote,
                            _ if c.is_whitespace() => {} // 無視
                            _ => {
                                // 不正な文字
                                self.state = TokenizerState::BogusDoctype;
                            }
                        }
                        if let Some(Token::Doctype {
                            ref mut public_id, ..
                        }) = self.current_token
                        {
                            *public_id = Some(c.to_string());
                        }
                    }
                    TokenizerState::DoctypePublicIdWithSingleQuote
                    | TokenizerState::DoctypePublicIdWithDoubleQuote => {
                        if let Some(Token::Doctype {
                            ref mut public_id, ..
                        }) = self.current_token
                        {
                            if let Some(ref mut pid) = public_id {
                                pid.push(c);
                            }
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
                        if let Some(Token::Doctype {
                            ref mut system_id, ..
                        }) = self.current_token
                        {
                            if system_id.is_none() {
                                *system_id = Some(c.to_string());
                            } else if let Some(ref mut sid) = system_id {
                                sid.push(c);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn state_before_attribute_name(&mut self, c: char) {
        match c {
            c if c.is_whitespace() => {} // 無視
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
            _ => { // 不正な文字
                 // 壊れたトークンは無視
            }
        }
    }

    fn state_attribute_name(&mut self, c: char) {
        match c {
            c if c.is_whitespace() => self.state = TokenizerState::AfterAttributeName,
            '=' => self.state = TokenizerState::BeforeAttributeValue,
            '/' => {
                if let Some(attr) = self.current_attribute.take() {
                    if let Some(Token::StartTag {
                        ref mut attributes, ..
                    }) = self.current_token
                    {
                        attributes.push(attr);
                    }
                }
                self.state = TokenizerState::SelfClosingStartTag;
            }
            '>' => {
                if let Some(attr) = self.current_attribute.take() {
                    if let Some(Token::StartTag {
                        ref mut attributes, ..
                    }) = self.current_token
                    {
                        attributes.push(attr);
                    }
                }
                self.commit_token();
                self.state = TokenizerState::Data;
            }
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':' => {
                self.buffer.push(c);
                if let Some(ref mut attr) = self.current_attribute {
                    attr.name.push(c);
                }
            }
            _ => { // 不正な文字
                 // 壊れたトークンは無視
            }
        }
    }

    fn state_before_attribute_value(&mut self, c: char) {
        match c {
            c if c.is_whitespace() => {} // 無視
            '"' => self.state = TokenizerState::AttributeValueDoubleQuoted,
            '\'' => self.state = TokenizerState::AttributeValueSingleQuoted,
            '>' => {
                // 属性値がない場合は空文字列として扱う
                if let Some(attr) = self.current_attribute.take() {
                    if let Some(Token::StartTag {
                        ref mut attributes, ..
                    }) = self.current_token
                    {
                        attributes.push(attr);
                    }
                }
                self.commit_token();
                self.state = TokenizerState::Data;
            }
            _ => {
                // 属性値が引用符で囲まれていない場合
                self.state = TokenizerState::AttributeValueUnquoted;
                if let Some(ref mut attr) = self.current_attribute {
                    attr.value.push(c);
                }
            }
        }
    }

    fn state_attribute_value_quoted(&mut self, c: char) {
        match c {
            '"' if self.state == TokenizerState::AttributeValueDoubleQuoted => {
                if let Some(attr) = self.current_attribute.take() {
                    if let Some(Token::StartTag {
                        ref mut attributes, ..
                    }) = self.current_token
                    {
                        attributes.push(attr);
                    }
                }
                self.state = TokenizerState::AfterAttributeName;
            }
            '\'' if self.state == TokenizerState::AttributeValueSingleQuoted => {
                if let Some(attr) = self.current_attribute.take() {
                    if let Some(Token::StartTag {
                        ref mut attributes, ..
                    }) = self.current_token
                    {
                        attributes.push(attr);
                    }
                }
                self.state = TokenizerState::AfterAttributeName;
            }
            _ => {
                if let Some(ref mut attr) = self.current_attribute {
                    attr.value.push(c);
                }
            }
        }
    }

    fn state_after_attribute_name(&mut self, c: char) {
        match c {
            c if c.is_whitespace() => {} // 無視
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
            _ => { // 不正な文字
                 // 壊れたトークンは無視
            }
        }
    }

    fn state_attribute_value_unquoted(&mut self, c: char) {
        match c {
            c if c.is_whitespace() => {
                if let Some(attr) = self.current_attribute.take() {
                    if let Some(Token::StartTag {
                        ref mut attributes, ..
                    }) = self.current_token
                    {
                        attributes.push(attr);
                    }
                }
                self.state = TokenizerState::BeforeAttributeName;
            }
            '>' => {
                if let Some(attr) = self.current_attribute.take() {
                    if let Some(Token::StartTag {
                        ref mut attributes, ..
                    }) = self.current_token
                    {
                        attributes.push(attr);
                    }
                }
                self.commit_token();
                self.state = TokenizerState::Data;
            }
            _ => {
                if let Some(ref mut attr) = self.current_attribute {
                    attr.value.push(c);
                }
            }
        }
    }

    fn state_self_closing_start_tag(&mut self, c: char) {
        match c {
            '>' => {
                if let Some(Token::StartTag {
                    ref mut self_closing,
                    ..
                }) = self.current_token
                {
                    *self_closing = true;
                }
                self.commit_token();
                self.state = TokenizerState::Data;
            }
            _ => {
                // 壊れたトークンは無視
                self.state = TokenizerState::Data;
            }
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
            '>' => {
                // 壊れたトークンは無視
                self.state = TokenizerState::Data;
            }
            _ => {
                // 壊れたトークンは無視
                self.state = TokenizerState::Data;
            }
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
                } else if let Some(Token::Comment(ref mut comment)) = self.current_token {
                    comment.push(c);
                }
            }
            TokenizerState::CommentEndDash => {
                if c == '-' {
                    // コメント終了
                    self.state = TokenizerState::CommentEnd;
                } else {
                    self.state = TokenizerState::Comment;
                    self.buffer.push('-');
                    self.buffer.push(c);
                    if let Some(Token::Comment(ref mut comment)) = self.current_token {
                        comment.push('-');
                        comment.push(c);
                    } else {
                        self.current_token = Some(Token::Comment(format!("-{c}")));
                    }
                }
            }
            TokenizerState::CommentEnd => {
                if c == '>' {
                    // コメント終了
                    self.commit_token();
                    self.state = TokenizerState::Data;
                } else {
                    self.state = TokenizerState::Comment;
                    self.buffer.push_str("--");
                    self.buffer.push(c);
                    if let Some(Token::Comment(ref mut comment)) = self.current_token {
                        comment.push_str("--");
                        comment.push(c);
                    } else {
                        self.current_token = Some(Token::Comment(format!("--{c}")));
                    }
                }
            }
            _ => {}
        }
    }
}
