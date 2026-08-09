use orinium_browser::engine::html::parser;
use orinium_browser::engine::html::parser::ClassicScriptSource;

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
        <p>Nested <span>span glyph</span></p>
        <img src="image.png">
        <br />
        <input type="glyph" value="Hello" />
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

#[test]
fn classic_scripts_are_collected_in_document_order() {
    let html = r#"
        <script>window.order = "inline-1";</script>
        <script src="one.js"></script>
        <script type="module" src="module.js"></script>
        <script type="application/json">{"not":"javascript"}</script>
        <script type="text/javascript">window.order = "inline-2";</script>
        <script src="two.js"></script>
    "#;

    let dom = parser::Parser::new(html).parse();
    assert_eq!(
        dom.collect_classic_scripts(),
        [
            ClassicScriptSource::Inline("window.order = \"inline-1\";".to_string()),
            ClassicScriptSource::External("one.js".to_string()),
            ClassicScriptSource::Inline("window.order = \"inline-2\";".to_string()),
            ClassicScriptSource::External("two.js".to_string()),
        ]
    );
}
