use anyhow::Result;
use orinium_browser::ProcessHandler;
use orinium_browser::browser::{BrowserApp, BrowserUi, Tab};

fn main() -> Result<()> {
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();

    // Initialize before dispatching to a child-process handler: child
    // processes (e.g. --type=network) never return from handle(), and
    // without this their log macros would all be silent no-ops.
    env_logger::init();

    if let Some(handler) = ProcessHandler::current() {
        handler.handle();
    }

    let url = std::env::args()
        .nth(1)
        .filter(|arg| !arg.starts_with('-'))
        .unwrap_or_else(|| "resource:///test/test.html".to_string());

    let mut tab = Tab::default();
    tab.navigate(url.parse()?);

    let mut browser = BrowserApp::default();
    browser.set_default_ui(BrowserUi::with_tab(tab));
    browser.run()?;

    Ok(())
}
