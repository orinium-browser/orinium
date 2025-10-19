use orinium_browser::engine::html::parser;

#[test]
fn test_dom_parse() {
    let html = r#"<!DOCTYPE html>
<html lang="ja">
<head>
    <title>Orinium Browser DOM Test</title>
    <!-- コメント -->
</head>
<body>
    <p>This is a <b>test page</b> for DOM module debuging.</p>
    <div>
        <p>Nested <span>span text</span></p>
        <img src="image.png">
        <br />
        <input type="text" value="Hello" />
        <p>Unclosed paragraph
    </div>
    <footer>Footer content</footer>
</body>
</html>
"#;

    html.to_string();
    let mut parser = parser::Parser::new(&html);
    let dom = parser.parse();
    println!("DOM Tree:\n{}", dom);
}

#[test]
fn test_dom_parse_malformed() {
    let html = r#"<html><head><title>Test</title></head><body><p>Paragraph 1<p>Paragraph 2<div>Div content"#;

    let mut parser = parser::Parser::new(&html);
    let dom = parser.parse();
    println!("DOM Tree:\n{}", dom);
}
