//! A processor that runs the layout builder on a background thread.
//!
//! `build_layout_and_info_*` is expensive (CSS cascade, text measurement,
//! layout-tree construction), so it is offloaded to a background thread to
//! keep the UI thread responsive. The builder walks a `Send` [`DomSnapshot`],
//! so the snapshot is the only input that needs to be handed to the thread.
//!
//! Per-frame lightweight layout (`LayoutEngine::layout`) still runs on the UI
//! thread as before. This processor only handles the heavier tree build.
//!
//! Tasks are coalesced: each task carries a monotonic sequence number, and the
//! thread skips a task as soon as a newer one has been queued. The newest task
//! always captures the latest DOM/styles/images, so the skipped build's work
//! would be wasted anyway.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ui_layout::LayoutNode;

use super::builder::{InheritedCss, build_layout_and_info_from_snapshot};
use super::css_resolver::{MediaEnvironment, ResolvedStyles};
use super::dom_snapshot::{DomSnapshot, NodeId};
use super::types::InfoNode;
use crate::background_worker::BackgroundWorker;
use crate::bridge::text::TextMeasurer;
use crate::css::matcher::ElementChain;
use crate::html::ScriptingMode;
use crate::layouter::css_resolver::RuleSet;
use crate::layouter::types::ColorScheme;
use crate::renderer_model::Image;
use crate::ui::registry::DomWriteBack;
use crate::profile_log;

/// The complete set of inputs the builder needs to run on the background thread.
pub struct LayoutTask {
    pub snapshot: Arc<DomSnapshot>,
    pub root: NodeId,
    pub resolved_styles: Arc<ResolvedStyles>,
    pub media_environment: MediaEnvironment,
    pub measurer: Arc<dyn TextMeasurer>,
    pub system_color_scheme: ColorScheme,
    pub scripting_mode: ScriptingMode,
    pub images: HashMap<String, Image>,
    pub audio: HashMap<String, Arc<[u8]>>,
    pub parent: InheritedCss,
    pub chain: ElementChain,
    pub write_back_sender: Option<DomWriteBack>,
    /// Monotonic sequence number used to coalesce stale tasks. Assigned by
    /// [`LayoutProcessor::send`], ignore when constructing a task.
    pub version: u64,
}

/// The layout the builder finished on the background thread.
pub struct LayoutResult {
    pub layout: LayoutNode,
    pub info: InfoNode,
    /// Sequence number of the task that produced this result.
    pub version: u64,
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
/// - The `Box` is allocated by the thread and leaked with `into_raw`.
/// - The raw pointer is transferred through an `mpsc` channel (which
///   establishes a happens-before relationship between send and receive).
/// - The receiving (UI) thread rebuilds the `Box` exactly once, so there is
///   exactly one owner at any point in time.
struct SendableResult(*mut LayoutResult);

// SAFETY: the pointed-to `Box` is accessed only by the receiving side after
// the channel delivers it; no other thread touches it.
unsafe impl Send for SendableResult {}

/// A processor that accepts a [`DomSnapshot`] and returns the layout result
/// produced by the thread.
pub struct LayoutProcessor {
    worker: BackgroundWorker<LayoutCommand, Option<SendableResult>>,
    /// Latest task sequence number; shared with the thread so it can detect
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
        let latest = Arc::new(AtomicU64::new(0));
        let latest_clone = Arc::clone(&latest);

        let worker = BackgroundWorker::new(1, move |cmd: LayoutCommand| {
            match cmd {
                LayoutCommand::Build(task) => {
                    // A newer task queued after this one supersedes it: the
                    // newest task's snapshot/styles/images include every
                    // change made up to that point, so skip the build.
                    if task.version < latest_clone.load(Ordering::SeqCst) {
                        return None;
                    }
                    let version = task.version;
                    let rule_set =
                        RuleSet::from_declarations(&task.resolved_styles, &task.media_environment);
                    let (layout, info) = build_layout_and_info_from_snapshot(
                        &task.snapshot,
                        task.root,
                        &rule_set,
                        task.measurer,
                        task.parent,
                        task.chain,
                        task.system_color_scheme,
                        task.scripting_mode,
                        &task.images,
                        &task.audio,
                        task.write_back_sender,
                    );
                    let result = LayoutResult {
                        layout,
                        info,
                        version,
                    };

                    profile_log!(
                        target: "LayoutRun",
                        log::Level::Info,
                        "build_layout_and_info done."
                    );
                    Some(SendableResult(Box::into_raw(Box::new(result))))
                }
            }
        });

        Self { worker, latest }
    }

    /// Sends a layout task to the thread.
    ///
    /// The task is stamped with a fresh sequence number; tasks that fall behind
    /// the newest one are skipped by the thread.
    pub fn send(&self, task: LayoutTask) -> u64 {
        let mut task = task;
        task.version = self.latest.fetch_add(1, Ordering::SeqCst) + 1;
        let version = task.version;
        self.worker.send(LayoutCommand::Build(task));
        version
    }

    /// Returns a completed layout result, or `None` if none is ready yet.
    pub fn try_receive(&self) -> Option<LayoutResult> {
        let inner = self.worker.try_receive()?;
        let SendableResult(ptr) = inner?;
        // SAFETY: `ptr` was produced by the thread with `Box::into_raw` and
        // has been delivered over the channel. We are the sole owner here.
        let boxed = unsafe { Box::from_raw(ptr) };
        Some(*boxed)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::bridge::text::FallbackTextMeasurer;
    use crate::html::parser::Parser as HtmlParser;
    use crate::layouter::types::NodeKind;
    use crate::ui::custom_node::CustomNode;
    use crate::ui::input_text_types::InputTextEvent;

    fn sample_task(write_back_sender: Option<DomWriteBack>) -> LayoutTask {
        let html = "<html><body><p>hello</p><input value='a'></body></html>";
        let dom = HtmlParser::new(html).parse();
        let (snapshot, _dom_refs) = DomSnapshot::from_tree(&dom.root);
        let root = snapshot.roots()[0];
        LayoutTask {
            snapshot: Arc::new(snapshot),
            root,
            resolved_styles: Arc::new(ResolvedStyles::default()),
            media_environment: MediaEnvironment::new((0.0, 0.0), ColorScheme::Light),
            measurer: Arc::new(FallbackTextMeasurer),
            system_color_scheme: ColorScheme::Light,
            scripting_mode: ScriptingMode::default(),
            images: HashMap::new(),
            audio: HashMap::new(),
            parent: InheritedCss::default(),
            chain: ElementChain::default(),
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
    fn layout_task_round_trips_through_background_thread() {
        let processor = LayoutProcessor::new();
        let requested_version = processor.send(sample_task(None));

        let result = wait_for_result(&processor);
        assert_eq!(result.version, requested_version);
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
