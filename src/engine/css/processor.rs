use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use super::parser::Parser as CssParser;
use crate::engine::layouter::css_resolver::{CssResolver, ResolvedStyles};

enum CssCommand {
    Process { css_sources: Vec<String> },
}

pub struct CssProcessor {
    cmd_tx: Sender<CssCommand>,
    result_rx: Receiver<ResolvedStyles>,
}

impl CssProcessor {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<CssCommand>();
        let (result_tx, result_rx) = mpsc::channel::<ResolvedStyles>();

        thread::spawn(move || {
            for cmd in cmd_rx {
                match cmd {
                    CssCommand::Process { css_sources } => {
                        let resolved = Self::process_all(&css_sources);
                        let _ = result_tx.send(resolved);
                    }
                }
            }
        });

        Self { cmd_tx, result_rx }
    }

    /// Send CSS source strings to the background thread for parsing and resolution.
    /// The thread will process all sources in order and return a single combined result.
    pub fn process(&self, css_sources: Vec<String>) {
        let _ = self.cmd_tx.send(CssCommand::Process { css_sources });
    }

    /// Poll for a completed result. Returns `None` if no result is ready yet.
    pub fn try_receive(&self) -> Option<ResolvedStyles> {
        self.result_rx.try_recv().ok()
    }

    fn process_all(css_sources: &[String]) -> ResolvedStyles {
        let mut resolved = ResolvedStyles::default();

        for css in css_sources {
            let sheet = match CssParser::new(css).parse() {
                Ok(sheet) => sheet,
                Err(err) => {
                    log::error!("[CssProcessor] Failed to parse CSS: {}", err);
                    continue;
                }
            };

            resolved.extend(CssResolver::resolve(&sheet));
        }

        resolved
    }
}
