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

    let mut browser = BrowserApp::default();

    let mut tab = Tab::new(browser.network());
    tab.load_from_url("resource:///test/compatibility_test.html")
        .await?;

    browser.add_tab(tab);

    browser.run().await?;

    Ok(())
}
