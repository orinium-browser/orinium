//! Browser UI components.

use crate::browser::Tab;

#[derive(Debug)]
pub struct BrowserUi {
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
}

impl BrowserUi {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active_tab: 0,
        }
    }

    pub fn with_tab(tab: Tab) -> Self {
        Self {
            tabs: vec![tab],
            active_tab: 0,
        }
    }
}
