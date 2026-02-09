use anyhow::Result;
use orinium_browser::browser::{BrowserApp, Tab};
use std::env;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let _font_path = if args.len() > 1 {
        Some(args[1].clone())
    } else {
        None
    };

    env_logger::init();

    let mut browser = BrowserApp::default();

    let mut tab = Tab::new();
    tab.navigate("resource:///test/css_apply.html".parse()?);

    browser.add_tab(tab);

    browser.run()?;

    Ok(())
}
