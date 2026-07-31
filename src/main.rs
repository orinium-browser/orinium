use anyhow::Result;
use orinium_browser::ProcessHandler;
use orinium_browser::browser::{BrowserApp, BrowserUi, Tab};

fn main() -> Result<()> {
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();

    if let Some(handler) = ProcessHandler::current() {
        handler.handle();
    }

    env_logger::init();

    let mut tab = Tab::new();
    tab.navigate("resource:///test/test.html".parse()?);

    let mut browser = BrowserApp::default();
    browser.set_default_ui(BrowserUi::with_tab(tab));
    browser.run()?;

    Ok(())
}
