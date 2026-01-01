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
                <title>Image test (ねこがくれによるかわいいイラストを使用しています)</title>
            </head>
            <body>
                <h1>Image Test</h1>
                <p>This page tests texture rendering.</p>
                <img src="resource:///images/image_test1.png" alt="Very cute texture" width="300"/>
                <p>Above is a cute cat illustration.</p>
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
