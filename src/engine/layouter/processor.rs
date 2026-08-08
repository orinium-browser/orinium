//! A processor that runs the layout builder on a background thread.
//!
//! `build_layout_and_info_*` is expensive (CSS cascade, text measurement,
//! layout-tree construction), so it is offloaded to a worker thread to keep
//! the UI thread responsive. The builder walks a `Send` [`DomSnapshot`], so
//! the snapshot is the only input that needs to be handed to the worker.
//!
//! Per-frame lightweight layout (`LayoutEngine::layout`) still runs on the UI
//! thread as before. This processor only handles the heavier tree build.
//!
//! Tasks are coalesced: each task carries a monotonic sequence number, and the
//! worker skips a task as soon as a newer one has been queued. The newest task
//! always captures the latest DOM/styles/images, so the skipped build's work
//! would be wasted anyway.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

use ui_layout::LayoutNode;

use super::builder::{InheritedCss, build_layout_and_info_from_snapshot};
use super::css_resolver::ResolvedStyles;
use super::dom_snapshot::{DomSnapshot, NodeId};
use super::types::{InfoNode, TextStyle};
use crate::engine::bridge::text::TextMeasurer;
use crate::engine::css::matcher::ElementChain;
use crate::engine::layouter::types::ColorScheme;
use crate::engine::renderer_model::Image;
use crate::engine::ui::registry::DomWriteBack;

/// The complete set of inputs the builder needs to run on the worker thread.
pub struct LayoutTask {
    pub snapshot: Arc<DomSnapshot>,
    pub root: NodeId,
    pub resolved_styles: Arc<ResolvedStyles>,
    pub measurer: Arc<dyn TextMeasurer<TextStyle>>,
    pub system_color_scheme: ColorScheme,
    pub images: HashMap<String, Image>,
    pub audio: HashMap<String, Arc<[u8]>>,
    pub parent: InheritedCss,
    pub chain: ElementChain,
    pub write_back_sender: Option<DomWriteBack>,
    /// Monotonic sequence number used to coalesce stale tasks. Assigned by
    /// [`LayoutProcessor::send`], ignore when constructing a task.
    pub version: u64,
}

/// The layout the builder finished on the worker thread.
pub struct LayoutResult {
    pub layout: LayoutNode,
    pub info: InfoNode,
}

enum LayoutCommand {
    Build(LayoutTask),
}

/// A pointer wrapper that transfers a [`LayoutResult`] across a channel.
///
/// `LayoutNode` contains ui_layout's `Box<dyn CustomLayouter>`, which is not
/// `Send`. The result is therefore leaked onto the heap with `Box::into_raw`
/// and sent as a pointer; the receiving side rebuilds it with `Box::from_raw`
/// as the single owner.
///
/// Safety is guaranteed by the command/response protocol:
/// - The `Box` is allocated by the worker and leaked with `into_raw`.
/// - The raw pointer is transferred through an `mpsc` channel (which
///   establishes a happens-before relationship between send and receive).
/// - The receiving (UI) thread rebuilds the `Box` exactly once, so there is
///   exactly one owner at any point in time.
struct SendableResult(*mut LayoutResult);

// SAFETY: the pointed-to `Box` is accessed only by the receiving side after
// the channel delivers it; no other thread touches it.
unsafe impl Send for SendableResult {}

/// A processor that accepts a [`DomSnapshot`] and returns the layout result
/// produced by the worker.
pub struct LayoutProcessor {
    cmd_tx: mpsc::Sender<LayoutCommand>,
    result_rx: mpsc::Receiver<SendableResult>,
    /// Latest task sequence number; shared with the worker so it can detect
    /// and skip superseded tasks.
    latest: Arc<AtomicU64>,
}

impl std::fmt::Debug for LayoutProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LayoutProcessor")
    }
}

impl Default for LayoutProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutProcessor {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<LayoutCommand>();
        let (result_tx, result_rx) = mpsc::channel::<SendableResult>();

        let latest = Arc::new(AtomicU64::new(0));
        let worker_latest = Arc::clone(&latest);

        thread::spawn(move || {
            for cmd in cmd_rx {
                match cmd {
                    LayoutCommand::Build(task) => {
                        // A newer task queued after this one supersedes it: the
                        // newest task's snapshot/styles/images include every
                        // change made up to that point, so skip the build.
                        if task.version < worker_latest.load(Ordering::SeqCst) {
                            continue;
                        }
                        let (layout, info) = build_layout_and_info_from_snapshot(
                            &task.snapshot,
                            task.root,
                            &task.resolved_styles,
                            task.measurer,
                            task.parent,
                            task.chain,
                            task.system_color_scheme,
                            &task.images,
                            &task.audio,
                            task.write_back_sender,
                        );
                        let result = LayoutResult { layout, info };
                        let _ = result_tx.send(SendableResult(Box::into_raw(Box::new(result))));
                    }
                }
            }
        });

        Self {
            cmd_tx,
            result_rx,
            latest,
        }
    }

    /// Sends a layout task to the worker.
    ///
    /// The task is stamped with a fresh sequence number; tasks that fall behind
    /// the newest one are skipped by the worker.
    pub fn send(&self, task: LayoutTask) {
        let mut task = task;
        task.version = self.latest.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.cmd_tx.send(LayoutCommand::Build(task));
    }

    /// Returns a completed layout result, or `None` if none is ready yet.
    pub fn try_receive(&self) -> Option<LayoutResult> {
        self.result_rx.try_recv().ok().map(|SendableResult(ptr)| {
            // SAFETY: `ptr` was produced by the worker with `Box::into_raw` and
            // has been delivered over the channel. We are the sole owner here.
            let boxed = unsafe { Box::from_raw(ptr) };
            *boxed
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::engine::bridge::text::FallbackTextMeasurer;
    use crate::engine::html::parser::Parser as HtmlParser;
    use crate::engine::layouter::types::NodeKind;
    use crate::engine::ui::custom_node::CustomNode;
    use crate::engine::ui::input_text_types::InputTextEvent;

    fn sample_task(write_back_sender: Option<DomWriteBack>) -> LayoutTask {
        let html = "<html><body><p>hello</p><input value='a'></body></html>";
        let dom = HtmlParser::new(html).parse();
        let (snapshot, _dom_refs) = DomSnapshot::from_tree(&dom.root);
        let root = snapshot.roots()[0];
        LayoutTask {
            snapshot: Arc::new(snapshot),
            root,
            resolved_styles: Arc::new(ResolvedStyles::default()),
            measurer: Arc::new(FallbackTextMeasurer),
            system_color_scheme: ColorScheme::Light,
            images: HashMap::new(),
            audio: HashMap::new(),
            parent: InheritedCss {
                text_style: TextStyle::default(),
                color_scheme: ColorScheme::Light,
            },
            chain: Vec::new(),
            write_back_sender,
            version: 0,
        }
    }

    fn wait_for_result(processor: &LayoutProcessor) -> LayoutResult {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = processor.try_receive() {
                return result;
            }
            assert!(
                Instant::now() < deadline,
                "layout result did not arrive before the timeout"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn find_custom<'i>(info: &'i InfoNode, role: &str) -> Option<&'i dyn CustomNode> {
        if let NodeKind::Custom { node, .. } = &info.kind
            && node.role() == Some(role)
        {
            return Some(&**node);
        }
        info.children.iter().find_map(|c| find_custom(c, role))
    }

    #[test]
    fn layout_task_round_trips_through_worker() {
        let processor = LayoutProcessor::new();
        processor.send(sample_task(None));

        let result = wait_for_result(&processor);
        assert!(
            !result.info.children.is_empty(),
            "the built layout must not be empty"
        );
    }

    #[test]
    fn input_edits_are_reported_through_write_back_channel() {
        let (tx, rx) = mpsc::channel::<(u32, String)>();
        let processor = LayoutProcessor::new();
        processor.send(sample_task(Some(tx)));

        let result = wait_for_result(&processor);
        let input = find_custom(&result.info, "textbox")
            .expect("input component must exist in the Info tree");

        input.handle_text_input(InputTextEvent::Insert("hello".into()));

        let (node_id, value) = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("write-back was never sent");
        assert!(node_id > 0, "input node id is invalid");
        assert_eq!(value, "ahello");
    }
}
