use super::parser::Parser as CssParser;
use crate::engine::background_worker::BackgroundWorker;
use crate::engine::layouter::css_resolver::{CssResolver, ResolvedStyles, append_resolved_styles};

enum CssCommand {
    Process { css_sources: Vec<String> },
}

#[derive(Debug)]
pub struct CssProcessor {
    worker: BackgroundWorker<CssCommand, ResolvedStyles>,
}

impl Default for CssProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl CssProcessor {
    pub fn new() -> Self {
        Self {
            worker: BackgroundWorker::new(1, |cmd| match cmd {
                CssCommand::Process { css_sources } => Self::process_all(&css_sources),
            }),
        }
    }

    /// Send CSS source strings to the background thread for parsing and resolution.
    /// The thread will process all sources in order and return a single combined result.
    pub fn process(&self, css_sources: Vec<String>) {
        self.worker.send(CssCommand::Process { css_sources });
    }

    /// Poll for a completed result. Returns `None` if no result is ready yet.
    pub fn try_receive(&self) -> Option<ResolvedStyles> {
        self.worker.try_receive()
    }

    fn process_all(css_sources: &[String]) -> ResolvedStyles {
        let mut resolved = ResolvedStyles::default();

        for css in css_sources {
            let sheet = CssParser::new(css).parse_lossy();

            append_resolved_styles(&mut resolved, CssResolver::resolve(&sheet));
        }

        resolved
    }
}
