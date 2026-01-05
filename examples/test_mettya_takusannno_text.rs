use anyhow::Result;
use orinium_browser::browser::{BrowserApp, Tab};
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let _font_path = if args.len() > 1 {
        Some(args[1].clone())
    } else {
        None
    };

    env_logger::init();

    let html = r#"
        <!DOCTYPE html>
        <html>
            <head>
                <title>Test Page</title>
            </head>
            <body>
                <h1>Hello, Orinium Browser!</h1>
                <p>This is a test paragraph.</p>
                <p>日本語テスト</p>
                <div>
                    <p>Nested paragraph in a div.</p>
                    <p>Span inside a paragraph: <span>Span Text</span></p>
                    <p>&amp; &lt; &gt; &quot; &#65; &#x41;</p>
                </div>
                <p>asdfaaaaaaaaaaaaaaaaaaaa</p>
                <p>afdsssssssssssssssa</p>
                <p>afsadddddddddddddddddddddddddddddddd</p>
                <p>afdasssssssssssssssssssss</p>
                <p>afadssssssssssssssssssss</p>
                <p>fasdddddddddddddddddddddddda</p>
                <p>fasddddddddddddddddddddddddddddddddddda</p>
                <p>fadssssssssssssssssa</p>
                <p>fsaddddddddddddddddddddddddddddddddddddddda</p>
                <p>afadsssssssssssssssssssssssssssssssssssssss</p>
                <p>afadsssssssssssssssssssssssssssss</p>
                <p>afadsfadsssssssssssssssssssssssss</p>
                <p>afadsfdsafdsafdsfbasdfbasd</p>
                <p>dsabfdsabfdsbafa</p>
                <p>abfasbfsabfsdabfsdabf</p>
                <p>absafdsabfdsabfdsabfdsbafdsab</p>
                <p>fdsabfdsbafsdbabfdsaa</p>
                <p>abfdsabfdsabfsdbffddasb</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
            </body>
        </html>
    "#;

    let mut browser = BrowserApp::default();

    let mut tab = Tab::new(browser.network());
    tab.load_from_raw_html(html);

    browser.add_tab(tab);

    browser.run()?;

    Ok(())
}
