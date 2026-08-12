//! Runs the page's JavaScript runtime on a dedicated background thread.
//!
//! Script evaluation, DOM events, timers, and fetch settlement are processed
//! off the UI thread. The executor maintains a private DOM mirror and sends
//! [`DomSnapshot`]s back when the DOM is mutated.
//!
//! This is not a Web Worker: scripts have full `window`/`document` access.
//! Tasks are processed FIFO, with coalescable timer wakeups.

use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

use super::{
    JsDynamicScriptRequest, JsDynamicStyleRequest, JsFetchRequest, JsFetchResponse, JsRuntime,
};
use crate::engine::layouter::dom_snapshot::DomSnapshot;

/// What the background JS thread should do next.
#[derive(Debug)]
pub enum JsTask {
    /// Set the URL exposed through `document` and related browser APIs.
    SetDocumentUrl { url: String },
    /// Execute a classic (blocking or deferred) script.
    RunScript { source: String },
    /// Dispatch `DOMContentLoaded` to document listeners.
    DispatchDomContentLoaded,
    /// Run timer callbacks whose deadlines have elapsed (coalescable).
    RunTimers,
    /// Dispatch a click on the element with the given JS-facing dom id.
    Click { dom_id: u64 },
    /// Dispatch an event on the element with the given JS-facing dom id.
    DispatchElementEvent { dom_id: u64, event_type: String },
    /// Resolve a pending JavaScript `fetch()` with a network response.
    ResolveFetch { id: u64, response: JsFetchResponse },
    /// Reject a pending JavaScript `fetch()` after a network failure.
    RejectFetch { id: u64, reason: String },
    /// Replace the JS thread's mirror DOM with the UI's tree (write-backs).
    UpdateDom { snapshot: DomSnapshot },
}

/// A task stamped with its position in the ordered stream.
#[derive(Debug)]
enum JsCommand {
    Task { task: JsTask, version: u64 },
}

/// The outcome of a JS task, ready to be applied on the UI thread.
#[derive(Debug)]
pub struct JsTaskResult {
    /// The JS thread's mirror DOM, present only when a script mutated it.
    pub dom: Option<DomSnapshot>,
    /// Whether the DOM changed and the UI needs to relayout and redraw.
    pub needs_redraw: bool,
    /// `fetch()` requests queued by scripts while running this task.
    pub fetch_requests: Vec<JsFetchRequest>,
    /// Dynamically inserted script elements discovered while running this task.
    pub(crate) dynamic_script_requests: Vec<JsDynamicScriptRequest>,
    /// Dynamically inserted stylesheet links discovered while running this task.
    pub(crate) dynamic_style_requests: Vec<JsDynamicStyleRequest>,
    /// The sequence number of the task that produced this result.
    pub version: u64,
}

/// A processor that accepts [`JsTask`]s and returns the results produced by the
/// background JS thread.
pub struct JsProcessor {
    cmd_tx: mpsc::Sender<JsCommand>,
    result_rx: mpsc::Receiver<JsTaskResult>,
    /// Latest task sequence number; shared with the background thread so it can
    /// detect and skip superseded timer pokes.
    latest: Arc<AtomicU64>,
}

impl std::fmt::Debug for JsProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JsProcessor")
    }
}

impl JsProcessor {
    /// Starts the background JS thread initialized with a snapshot of the
    /// parsed document.
    pub fn new(initial: DomSnapshot) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<JsCommand>();
        let (result_tx, result_rx) = mpsc::channel::<JsTaskResult>();

        let latest = Arc::new(AtomicU64::new(0));
        let thread_latest = Arc::clone(&latest);

        thread::spawn(move || {
            let (tree, _dom_ids) = initial.into_tree();
            let mut runtime = JsRuntime::new(Rc::new(tree));

            for cmd in cmd_rx {
                let JsCommand::Task { task, version } = cmd;
                // A newer task queued after this timer poke supersedes it.
                if matches!(task, JsTask::RunTimers)
                    && version < thread_latest.load(Ordering::SeqCst)
                {
                    continue;
                }

                run_task(&mut runtime, task);
                let needs_redraw = runtime.take_needs_redraw();
                let fetch_requests = runtime.take_fetch_requests();
                let dynamic_script_requests = runtime.take_dynamic_script_requests();
                let dynamic_style_requests = runtime.take_dynamic_style_requests();
                let dom = if needs_redraw {
                    Some(runtime.snapshot())
                } else {
                    None
                };
                let _ = result_tx.send(JsTaskResult {
                    dom,
                    needs_redraw,
                    fetch_requests,
                    dynamic_script_requests,
                    dynamic_style_requests,
                    version,
                });
            }
        });

        Self {
            cmd_tx,
            result_rx,
            latest,
        }
    }

    /// Sends a task to the background thread, stamped with a fresh sequence
    /// number, and returns that sequence number so the caller can track when
    /// the task's result has been applied.
    pub fn send(&self, task: JsTask) -> u64 {
        let version = self.latest.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.cmd_tx.send(JsCommand::Task { task, version });
        version
    }

    /// Returns a completed task result, or `None` if none is ready yet.
    pub fn try_receive(&self) -> Option<JsTaskResult> {
        self.result_rx.try_recv().ok()
    }
}

fn run_task(runtime: &mut JsRuntime, task: JsTask) {
    match task {
        JsTask::SetDocumentUrl { url } => runtime.set_document_url(&url),
        JsTask::RunScript { source } => runtime.run_script(&source),
        JsTask::DispatchDomContentLoaded => {
            runtime.dispatch_dom_content_loaded();
        }
        JsTask::RunTimers => {
            runtime.run_due_timers();
        }
        JsTask::Click { dom_id } => {
            runtime.click_dom_id(dom_id);
        }
        JsTask::DispatchElementEvent { dom_id, event_type } => {
            runtime.dispatch_element_event(dom_id, &event_type);
        }
        JsTask::ResolveFetch { id, response } => runtime.resolve_fetch(id, response),
        JsTask::RejectFetch { id, reason } => runtime.reject_fetch(id, reason),
        JsTask::UpdateDom { snapshot } => runtime.apply_dom(&snapshot),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::engine::html::parser::Parser as HtmlParser;
    use crate::engine::layouter::NodeId;

    fn snapshot_of(html: &str) -> DomSnapshot {
        let dom = HtmlParser::new(html).parse();
        let (snapshot, _) = DomSnapshot::from_tree(&dom.root);
        snapshot
    }

    fn wait_for_result(processor: &JsProcessor) -> JsTaskResult {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = processor.try_receive() {
                return result;
            }
            assert!(
                Instant::now() < deadline,
                "JS result did not arrive before the timeout"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn find_element<'i>(snapshot: &'i DomSnapshot, id: NodeId, tag: &str) -> Option<NodeId> {
        if snapshot.node(id).kind.tag_name() == Some(tag) {
            return Some(id);
        }
        snapshot
            .children(id)
            .iter()
            .find_map(|&c| find_element(snapshot, c, tag))
    }

    fn get_attr(snapshot: &DomSnapshot, id: NodeId, name: &str) -> Option<String> {
        snapshot.node(id).kind.get_attr(name).map(str::to_string)
    }

    #[test]
    fn script_dom_changes_are_reported_in_the_result_snapshot() {
        let processor =
            JsProcessor::new(snapshot_of("<html><body><div id='x'></div></body></html>"));
        processor.send(JsTask::RunScript {
            source: r#"document.getElementById("x").setAttribute("data-a", "1");"#.to_string(),
        });

        let result = wait_for_result(&processor);
        assert!(result.needs_redraw);
        let snapshot = result.dom.expect("mutating script must commit a snapshot");
        let root = snapshot.roots()[0];
        let div = find_element(&snapshot, root, "div").unwrap();
        assert_eq!(get_attr(&snapshot, div, "data-a").as_deref(), Some("1"));
    }

    #[test]
    fn ordered_scripts_run_in_send_order() {
        let processor =
            JsProcessor::new(snapshot_of("<html><body><div id='x'></div></body></html>"));
        processor.send(JsTask::RunScript {
            source: r#"globalThis.__s = "a";"#.to_string(),
        });
        processor.send(JsTask::RunScript {
            source: r#"
                globalThis.__s += "b";
                document.getElementById("x").setAttribute("data-s", globalThis.__s);
            "#
            .to_string(),
        });

        // Only the last script mutates the DOM; its snapshot must reflect both.
        let mut result = None;
        for _ in 0..2 {
            let received = wait_for_result(&processor);
            if received.needs_redraw {
                result = Some(received);
            }
        }
        let snapshot = result
            .expect("the second script must commit a snapshot")
            .dom
            .unwrap();
        let root = snapshot.roots()[0];
        let div = find_element(&snapshot, root, "div").unwrap();
        assert_eq!(get_attr(&snapshot, div, "data-s").as_deref(), Some("ab"));
    }

    #[test]
    fn update_dom_replaces_the_mirror_before_later_tasks() {
        let processor =
            JsProcessor::new(snapshot_of("<html><body><div id='x'></div></body></html>"));
        // The UI changes the value of #x and pushes the real tree over.
        processor.send(JsTask::UpdateDom {
            snapshot: snapshot_of("<html><body><div id='x' data-v='ui'></div></body></html>"),
        });
        processor.send(JsTask::RunScript {
            source: r#"
                const el = document.getElementById("x");
                el.setAttribute("data-read", el.getAttribute("data-v"));
            "#
            .to_string(),
        });

        // UpdateDom applies first (FIFO), so the script reads the new value.
        let update = wait_for_result(&processor);
        assert!(!update.needs_redraw);
        assert!(update.dom.is_none());

        let result = wait_for_result(&processor);
        assert!(result.needs_redraw);
        let snapshot = result.dom.unwrap();
        let root = snapshot.roots()[0];
        let div = find_element(&snapshot, root, "div").unwrap();
        assert_eq!(get_attr(&snapshot, div, "data-read").as_deref(), Some("ui"));
    }
}
