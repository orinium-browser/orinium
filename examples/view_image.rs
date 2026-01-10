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

    // テスト用のHTML
    let html = r#"
        <!DOCTYPE html>
        <html>
            <head>
                <title>ごてごてした画像表示</title>
            </head>
            <body>
                <h1>Image draw test</h1>
                <img src="resource:///image/test1.png" />
                <img src="resource:///image/test2.png" />
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
