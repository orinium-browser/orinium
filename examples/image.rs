//! Displays local PNG resources through HTML `<img>` elements.
//!
//! ```sh
//! cargo run --example image
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
    tab.navigate("resource:///test/image.html".parse()?);

    let mut browser = BrowserApp::new((900, 700), "Orinium Image Example".into())?;
    browser.set_default_ui(BrowserUi::with_tab(tab));
    browser.run()
}
