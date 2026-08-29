use super::parser::Parser as CssParser;
use crate::engine::background_worker::BackgroundWorker;
use crate::engine::layouter::css_resolver::{CssResolver, ResolvedStyles, append_resolved_styles};
use crate::{perf_scope, profile_log};

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
        perf_scope!(total);
        let mut resolved = ResolvedStyles::default();

        #[cfg(any(feature = "profile", debug_assertions))]
        let mut parse_time = std::time::Duration::ZERO;
        #[cfg(any(feature = "profile", debug_assertions))]
        let mut resolve_time = std::time::Duration::ZERO;

        for css in css_sources {
            perf_scope!(parse);
            let sheet = CssParser::new(css).parse_lossy();
            #[cfg(any(feature = "profile", debug_assertions))]
            {
                parse_time += parse.elapsed();
            }

            perf_scope!(resolve);
            let resolved_sheet = CssResolver::resolve(&sheet);
            #[cfg(any(feature = "profile", debug_assertions))]
            {
                resolve_time += resolve.elapsed();
            }

            append_resolved_styles(&mut resolved, resolved_sheet);
        }

        profile_log!(
            target: "CssRun",
            log::Level::Info,
            "[CssResolve] sources: {} | total: {:?} | parse: {:?} | resolve: {:?}",
            css_sources.len(),
            total.elapsed(),
            parse_time,
            resolve_time,
        );
        resolved
    }
}
