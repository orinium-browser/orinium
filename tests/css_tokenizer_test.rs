use orinium_browser::engine::css::tokenizer::{Token, Tokenizer};

// Collect all tokens until EOF
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
            Token::Ident("foo+bar".into()),
            Token::EOF,
        ]
    );
}

#[test]
fn string_escape_quote() {
    let tokens = tokenize(r#""hello \"world\"""#);

    assert_eq!(
        tokens,
        vec![Token::String("hello \"world\"".into()), Token::EOF,]
    );
}

#[test]
fn ident_unicode_escape() {
    let tokens = tokenize(r#".\31 23"#);

    assert_eq!(
        tokens,
        vec![Token::Delim('.'), Token::Ident("123".into()), Token::EOF,]
    );
}

#[test]
fn string_unicode_escape() {
    let tokens = tokenize(r#""\000026""#);

    assert_eq!(tokens, vec![Token::String("&".into()), Token::EOF,]);
}

#[test]
fn complex_escape_sequence() {
    let tokens = tokenize(r#".foo\+bar { content: "a\26 b"; }"#);

    assert_eq!(
        tokens,
        vec![
            Token::Delim('.'),
            Token::Ident("foo+bar".into()),
            Token::Whitespace,
            Token::Delim('{'),
            Token::Whitespace,
            Token::Ident("content".into()),
            Token::Delim(':'),
            Token::Whitespace,
            Token::String("a&b".into()),
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
        vec![Token::Delim('.'), Token::Ident("foo".into()), Token::EOF,]
    );
}

#[test]
fn ident_line_continuation() {
    let tokens = tokenize(".foo\\\nbar");

    assert_eq!(
        tokens,
        vec![Token::Delim('.'), Token::Ident("foobar".into()), Token::EOF,]
    );
}

#[test]
fn string_line_continuation() {
    let tokens = tokenize("\"a\\\nb\"");

    assert_eq!(tokens, vec![Token::String("ab".into()), Token::EOF,]);
}
