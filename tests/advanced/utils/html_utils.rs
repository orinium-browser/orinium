use orinium_browser::{
    engine::html::parser::{HtmlNodeType, Parser},
    engine::tree::*,
};

pub const TEST_HTML: &str = r#"
    <!DOCTYPE html>
    <html>
        <head>
            <title>Test Page</title>
        </head>
        <body>
            <h1>Hello, Orinium Browser!</h1>
            <p>This is a test paragraph.</p>
            <div>
                <p>Nested paragraph in a div.</p>
            </div>
        </body>
    </html>
"#;

pub fn test_dom() -> Tree<HtmlNodeType> {
    // テスト用のHTML
    let html = TEST_HTML;

    // HTMLをパース
    let mut parser = Parser::new(html);
    let dom_tree = parser.parse();
    dom_tree
}
