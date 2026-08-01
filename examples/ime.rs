//! Demonstrates Windows IME composition in HTML text inputs.
//!
//! ```sh
//! cargo run --example ime
//! ```

use anyhow::Result;
use orinium_browser::ProcessHandler;
use orinium_browser::browser::{BrowserApp, BrowserUi, Tab};

fn main() -> Result<()> {
    if let Some(handler) = ProcessHandler::current() {
        handler.handle();
    }

    env_logger::init();

    let mut tab = Tab::new();
    tab.navigate("resource:///test/ime.html".parse()?);

    let mut browser = BrowserApp::new((720, 420), "Orinium IME Example".into())?;
    browser.set_default_ui(BrowserUi::with_tab(tab));
    browser.run()
}
