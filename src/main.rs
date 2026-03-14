use anyhow::Result;
use orinium_browser::browser::{BrowserApp, Tab};
use std::env;

fn main() -> Result<()> {
    let _args: Vec<String> = env::args().collect();

    env_logger::init();

    let mut browser = BrowserApp::default();

    let mut tab = Tab::new();
    tab.navigate("resource:///test/compatibility_test.html".parse()?);

    browser.add_tab(tab);

    browser.run()?;

    Ok(())
}
