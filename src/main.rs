use anyhow::Result;
use orinium_browser::ProcessHandler;
use orinium_browser::browser::{BrowserApp, Tab};

fn main() -> Result<()> {
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();

    if let Some(handler) = ProcessHandler::current() {
        handler.handle();
    }

    env_logger::init();

    let mut browser = BrowserApp::default();

    let mut tab = Tab::new();
    tab.navigate("resource:///test/compatibility_test.html".parse()?);

    browser.add_tab(tab);

    browser.run()?;

    Ok(())
}
