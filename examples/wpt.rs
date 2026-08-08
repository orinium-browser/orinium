//! Opens the upstream Web Platform Tests directory listing without JavaScript.
//!
//! ```sh
//! cargo run --example wpt
//! ```

use anyhow::Result;
use orinium_browser::ProcessHandler;
use orinium_browser::browser::{BrowserApp, BrowserUi, Tab};

const WPT_CSS2_INDEX: &str = "https://wpt.live/css/CSS2/";

fn main() -> Result<()> {
    if let Some(handler) = ProcessHandler::current() {
        handler.handle();
    }

    env_logger::init();

    let mut tab = Tab::default();
    tab.navigate(WPT_CSS2_INDEX.parse()?);

    let mut browser = BrowserApp::new((1200, 800), "Orinium WPT Browser".into())?;
    browser.set_default_ui(BrowserUi::with_tab(tab));
    browser.run()
}
