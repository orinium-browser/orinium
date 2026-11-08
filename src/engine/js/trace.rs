//! JavaScript execution tracing for the DevTools pane.
//!
//! The runtime records a bounded stream of execution events (script
//! evaluations, event dispatch, timers, microtasks, network settlement and
//! `console` output). Events are drained after each JS task by the processor
//! and rendered by the browser's DevTools UI (`resource:///devtools/`).
//!
//! Tracing is intentionally coarse: it operates at the engine boundary, where
//! the runtime hands control to page JavaScript. Recording an event never
//! changes execution and is skipped entirely while tracing is disabled.

use std::any::Any;
use std::collections::{HashSet, VecDeque};
use std::rc::Rc;
use std::time::Instant;

use pixi_byte::value::jsvalue::JsValueKind;
use pixi_byte::vm::VM;
use pixi_byte::{JSEngine, JSResult, JSValue};

use super::JsHost;

/// Default number of events kept in the JS thread's ring buffer.
const CAPACITY: usize = 512;

/// The kind of JavaScript execution event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsTraceKind {
    /// A classic script (or an inline handler compiled on demand).
    Script,
    /// DOM/window event dispatch.
    Event,
    /// A timer callback.
    Timer,
    /// Promise/`fetch` settlement.
    Fetch,
    /// Microtask checkpoint.
    Microtask,
    /// `console.*` output.
    Console,
}

impl JsTraceKind {
    /// Lowercase stable name used in the DevTools JSON envelope.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Script => "script",
            Self::Event => "event",
            Self::Timer => "timer",
            Self::Fetch => "fetch",
            Self::Microtask => "microtask",
            Self::Console => "console",
        }
    }
}

/// A single traced JavaScript execution event.
#[derive(Debug, Clone)]
pub struct JsTraceEvent {
    /// Monotonic event number, reset when the trace is cleared.
    pub seq: u64,
    /// Milliseconds since the trace started (or was last cleared).
    pub at_ms: f64,
    /// Event category.
    pub kind: JsTraceKind,
    /// Short label, e.g. `DOMContentLoaded` or `setTimeout`.
    pub label: String,
    /// Human-readable detail: listener counts, URLs, console text, …
    pub detail: String,
    /// Execution time in microseconds (0 for instantaneous events).
    pub duration_us: u64,
    /// Error message when the event failed.
    pub error: Option<String>,
    /// JavaScript call stack (innermost first) captured at the event.
    pub stack: Vec<String>,
    /// Bounded source preview for scripts and inline handlers.
    pub source: Option<String>,
    /// True when source is the most recently evaluated script, used for
    /// callbacks whose VM frame does not retain source text.
    pub source_is_fallback: bool,
    /// Local variables captured when this event terminated with an error.
    pub variables: Vec<(String, String)>,
    /// Bytecode program counter captured at a terminal runtime error.
    pub program_counter: Option<usize>,
}

/// A batch of trace events drained after one JS task.
#[derive(Debug, Clone, Default)]
pub struct JsTraceBatch {
    /// Whether tracing was enabled while the task ran.
    pub enabled: bool,
    /// Events recorded since the trace was cleared.
    pub total: u64,
    /// Events evicted from the ring buffer before being drained.
    pub dropped: u64,
    /// Events recorded during this task.
    pub events: Vec<JsTraceEvent>,
    /// JavaScript operations currently in progress on the JS thread.
    pub active: Vec<JsActiveTrace>,
}

/// An in-progress traced operation.
pub(crate) struct JsTraceSpan {
    pub(crate) active_id: Option<usize>,
    pub(crate) started: Instant,
    pub(crate) kind: JsTraceKind,
    pub(crate) label: String,
    pub(crate) detail: String,
    pub(crate) source: Option<String>,
}

/// Snapshot of a JavaScript operation currently in progress.
#[derive(Debug, Clone)]
pub struct JsActiveTrace {
    pub kind: JsTraceKind,
    pub label: String,
    pub detail: String,
    pub elapsed_ms: f64,
    pub stack: Vec<String>,
    pub source: Option<String>,
}

struct ActiveSpanEntry {
    id: usize,
    started: Instant,
    kind: JsTraceKind,
    label: String,
    detail: String,
    source: Option<String>,
}

/// Bounded event stream owned by [`JsHost`].
pub(crate) struct JsTraceSession {
    enabled: bool,
    start: Instant,
    events: VecDeque<JsTraceEvent>,
    next_seq: u64,
    next_active_id: usize,
    total: u64,
    dropped: u64,
    latest_source: Option<String>,
    active_spans: Vec<ActiveSpanEntry>,
}

impl JsTraceSession {
    pub(crate) fn new() -> Self {
        Self {
            enabled: default_enabled(),
            start: Instant::now(),
            events: VecDeque::with_capacity(CAPACITY),
            next_seq: 1,
            next_active_id: 1,
            total: 0,
            dropped: 0,
            latest_source: None,
            active_spans: Vec::new(),
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn clear(&mut self) {
        self.events.clear();
        self.active_spans.clear();
        self.start = Instant::now();
        self.next_seq = 1;
        self.total = 0;
        self.dropped = 0;
        self.latest_source = None;
    }

    fn begin_active(
        &mut self,
        kind: JsTraceKind,
        label: String,
        detail: String,
        source: Option<String>,
    ) -> Option<usize> {
        if !self.enabled {
            return None;
        }
        let id = self.next_active_id;
        self.next_active_id += 1;
        if let Some(source) = source.as_ref() {
            self.latest_source = Some(source.clone());
        }
        self.active_spans.push(ActiveSpanEntry {
            id,
            started: Instant::now(),
            kind,
            label,
            detail,
            source,
        });
        Some(id)
    }

    fn end_active(&mut self, id: Option<usize>) {
        let Some(id) = id else {
            return;
        };
        if let Some(index) = self.active_spans.iter().rposition(|span| span.id == id) {
            self.active_spans.remove(index);
        }
    }

    fn snapshot_active(&self, vm: &VM) -> Vec<JsActiveTrace> {
        if !self.enabled || self.active_spans.is_empty() {
            return Vec::new();
        }
        let stack = stack_from_vm(vm);
        self.active_spans
            .iter()
            .rev()
            .map(|span| JsActiveTrace {
                kind: span.kind,
                label: span.label.clone(),
                detail: span.detail.clone(),
                elapsed_ms: span.started.elapsed().as_secs_f64() * 1000.0,
                stack: stack.clone(),
                source: span.source.clone().or_else(|| self.latest_source.clone()),
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn record(
        &mut self,
        kind: JsTraceKind,
        label: String,
        detail: String,
        duration_us: u64,
        error: Option<String>,
        stack: Vec<String>,
        source: Option<String>,
        variables: Vec<(String, String)>,
        program_counter: Option<usize>,
    ) {
        if !self.enabled {
            return;
        }
        self.total += 1;
        if let Some(source) = source.as_ref() {
            self.latest_source = Some(source.clone());
        }
        // Event and timer callbacks usually do not carry their own source
        // text. Attach the latest evaluated script to failures for inspection.
        let source_is_fallback =
            source.is_none() && error.is_some() && self.latest_source.is_some();
        let source = source.or_else(|| error.as_ref().and(self.latest_source.clone()));
        if self.events.len() == CAPACITY {
            self.events.pop_front();
            self.dropped += 1;
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.events.push_back(JsTraceEvent {
            seq,
            at_ms: self.start.elapsed().as_secs_f64() * 1000.0,
            kind,
            label,
            detail,
            duration_us,
            error,
            stack,
            source,
            source_is_fallback,
            variables,
            program_counter,
        });
    }

    fn take_batch(&mut self, vm: &VM) -> JsTraceBatch {
        JsTraceBatch {
            enabled: self.enabled,
            total: self.total,
            dropped: self.dropped,
            events: self.events.drain(..).collect(),
            active: self.snapshot_active(vm),
        }
    }
}

/// Whether tracing starts enabled for a fresh runtime.
///
/// The event buffer is bounded, so recording starts before document scripts in
/// every build. Opening DevTools after navigation can then inspect startup.
pub fn default_enabled() -> bool {
    true
}

/// Installs an internal hook used by lowered async `catch` blocks to retain
/// errors that page code would otherwise swallow.
pub(crate) fn install_caught_exception_hook(engine: &mut JSEngine) {
    engine.global_mut().borrow_mut().set(
        "__orinium_trace_caught_error".to_string(),
        JSValue::from_native_function(trace_caught_exception),
    );
}

fn trace_caught_exception(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let error = args
        .get(1)
        .cloned()
        .unwrap_or_else(JSValue::undefined)
        .to_console_string();
    let catch_name = args
        .get(2)
        .map(JSValue::to_console_string)
        .unwrap_or_else(|| "error".to_string());
    let stack = stack_from_vm(vm);
    record(
        vm,
        JsTraceKind::Event,
        "caught async exception".to_string(),
        error.clone(),
        0,
        Some(error.clone()),
        stack,
        None,
        vec![(catch_name, error.clone())],
        None,
    );
    Ok(JSValue::undefined())
}

/// Captures the current JavaScript call stack, innermost function first.
pub(crate) fn stack_from_vm(vm: &VM) -> Vec<String> {
    vm.frames
        .iter()
        .rev()
        .take(24)
        .map(|frame| {
            frame
                .function_name
                .as_ref()
                .map(|name| name.as_str())
                .unwrap_or_else(|| "<anonymous>".to_string())
        })
        .collect()
}

/// Records a trace event if tracing is enabled.
#[allow(clippy::too_many_arguments)]
pub(crate) fn record(
    vm: &VM,
    kind: JsTraceKind,
    label: String,
    detail: String,
    duration_us: u64,
    error: Option<String>,
    stack: Vec<String>,
    source: Option<String>,
    variables: Vec<(String, String)>,
    program_counter: Option<usize>,
) {
    with_session_mut(vm, |session| {
        session.record(
            kind,
            label,
            detail,
            duration_us,
            error,
            stack,
            source,
            variables,
            program_counter,
        )
    });
}

/// Records a `console.*` call, including the stack that produced it.
pub(crate) fn record_console(vm: &VM, level: log::Level, message: &str) {
    let label = match level {
        log::Level::Error => "console.error",
        log::Level::Warn => "console.warn",
        log::Level::Info => "console.log",
        log::Level::Debug => "console.debug",
        log::Level::Trace => "console.trace",
    };
    let stack = stack_from_vm(vm);
    record(
        vm,
        JsTraceKind::Console,
        label.to_string(),
        message.to_string(),
        0,
        None,
        stack,
        None,
        Vec::new(),
        None,
    );
}

/// Formats JavaScript values for the DevTools console without invoking getters.
pub(crate) fn format_console_value(value: &JSValue) -> String {
    format_console_value_inner(value, 0, &mut HashSet::new(), false)
}

fn format_console_value_inner(
    value: &JSValue,
    depth: usize,
    seen: &mut HashSet<usize>,
    quote_strings: bool,
) -> String {
    match value.kind() {
        JsValueKind::String if quote_strings => {
            serde_json::to_string(value.as_string().unwrap_or("")).unwrap_or_default()
        }
        JsValueKind::Object => {
            let Some(object) = value.as_object() else {
                return value.to_console_string();
            };
            let identity = Rc::as_ptr(&object) as usize;
            if !seen.insert(identity) {
                return "[Circular]".to_string();
            }
            if depth >= 4 {
                seen.remove(&identity);
                return "[Object]".to_string();
            }
            let (is_array, keys, length) = {
                let object = object.borrow();
                (
                    object.is_array(),
                    object.keys(),
                    object.get("length").to_number().max(0.0) as usize,
                )
            };
            let rendered = if is_array {
                let values = (0..length.min(40))
                    .map(|index| {
                        let item = object.borrow().get_index(index);
                        format_console_value_inner(&item, depth + 1, seen, true)
                    })
                    .collect::<Vec<_>>();
                let mut values = values;
                if length > 40 {
                    values.push(format!("… {} more", length - 40));
                }
                format!("[{}]", values.join(", "))
            } else {
                let mut properties = keys
                    .into_iter()
                    .take(40)
                    .map(|key| {
                        let item = object.borrow().get(&key);
                        format!(
                            "{}: {}",
                            serde_json::to_string(&key).unwrap_or_default(),
                            format_console_value_inner(&item, depth + 1, seen, true)
                        )
                    })
                    .collect::<Vec<_>>();
                if object.borrow().keys().len() > 40 {
                    properties.push("… more properties".to_string());
                }
                format!("{{{}}}", properties.join(", "))
            };
            seen.remove(&identity);
            rendered
        }
        _ => value.to_console_string(),
    }
}

/// Returns whether tracing is currently enabled.
pub(crate) fn is_enabled(vm: &VM) -> bool {
    let Some(host) = vm.host.as_ref() else {
        return false;
    };
    let Ok(host_ref) = host.try_borrow() else {
        return false;
    };
    let Some(js_host) = (&*host_ref as &dyn Any).downcast_ref::<JsHost>() else {
        return false;
    };
    js_host.trace.enabled()
}

/// Enables or disables tracing on the JS thread.
pub(crate) fn set_enabled(vm: &VM, enabled: bool) {
    with_session_mut(vm, |session| session.set_enabled(enabled));
}

/// Clears the buffered trace and resets its counters.
pub(crate) fn clear(vm: &VM) {
    with_session_mut(vm, JsTraceSession::clear);
}

/// Drains the buffered events into a batch for the browser thread.
pub(crate) fn take_batch(vm: &VM) -> JsTraceBatch {
    let Some(host) = vm.host.as_ref() else {
        return JsTraceBatch::default();
    };
    let Ok(mut host_ref) = host.try_borrow_mut() else {
        return JsTraceBatch::default();
    };
    let Some(js_host) = (&mut *host_ref as &mut dyn Any).downcast_mut::<JsHost>() else {
        return JsTraceBatch::default();
    };
    js_host.trace.take_batch(vm)
}

pub(crate) fn begin_active(
    vm: &VM,
    kind: JsTraceKind,
    label: String,
    detail: String,
    source: Option<String>,
) -> Option<usize> {
    let Some(host) = vm.host.as_ref() else {
        return None;
    };
    let Ok(mut host_ref) = host.try_borrow_mut() else {
        return None;
    };
    let Some(js_host) = (&mut *host_ref as &mut dyn Any).downcast_mut::<JsHost>() else {
        return None;
    };
    js_host.trace.begin_active(kind, label, detail, source)
}

pub(crate) fn end_active(vm: &VM, id: Option<usize>) {
    let Some(host) = vm.host.as_ref() else {
        return;
    };
    let Ok(mut host_ref) = host.try_borrow_mut() else {
        return;
    };
    if let Some(js_host) = (&mut *host_ref as &mut dyn Any).downcast_mut::<JsHost>() {
        js_host.trace.end_active(id);
    }
}

fn with_session_mut(vm: &VM, f: impl FnOnce(&mut JsTraceSession)) {
    let Some(host) = vm.host.as_ref() else {
        return;
    };
    // Trace calls can be reentrant (a console call while another native holds
    // the host). Skipping an event is preferable to panicking.
    let Ok(mut host_ref) = host.try_borrow_mut() else {
        return;
    };
    if let Some(js_host) = (&mut *host_ref as &mut dyn Any).downcast_mut::<JsHost>() {
        f(&mut js_host.trace);
    }
}

/// Short single-line preview of a script source.
pub(crate) fn source_summary(source: &str) -> String {
    let line = source
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    truncate_chars(line, 100)
}

/// Bounded multi-line source excerpt retained for DevTools inspection.
pub(crate) fn source_preview(source: &str) -> String {
    truncate_chars(source.trim(), 240)
}

fn truncate_chars(text: &str, limit: usize) -> String {
    let mut out = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index == limit {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_keeps_only_the_newest_events() {
        let mut session = JsTraceSession::new();
        session.set_enabled(true);
        for index in 0..(CAPACITY + 3) {
            session.record(
                JsTraceKind::Script,
                format!("script-{index}"),
                String::new(),
                0,
                None,
                Vec::new(),
                None,
                Vec::new(),
                None,
            );
        }
        let batch = session.take_batch(&VM::new());
        assert_eq!(batch.total as usize, CAPACITY + 3);
        assert_eq!(batch.dropped, 3);
        assert_eq!(batch.events.len(), CAPACITY);
        assert_eq!(batch.events[0].label, "script-3");
    }

    #[test]
    fn disabled_session_records_nothing_and_clear_resets_counters() {
        let mut session = JsTraceSession::new();
        session.set_enabled(false);
        session.record(
            JsTraceKind::Script,
            "ignored".to_string(),
            String::new(),
            0,
            None,
            Vec::new(),
            None,
            Vec::new(),
            None,
        );
        assert!(session.take_batch(&VM::new()).events.is_empty());

        session.set_enabled(true);
        session.record(
            JsTraceKind::Event,
            "click".to_string(),
            String::new(),
            0,
            None,
            Vec::new(),
            None,
            Vec::new(),
            None,
        );
        session.clear();
        let batch = session.take_batch(&VM::new());
        assert_eq!(batch.total, 0);
        assert!(batch.events.is_empty());
        assert!(batch.active.is_empty());
    }

    #[test]
    fn source_previews_are_bounded_and_char_safe() {
        let summary = source_summary("\n\n  const answer = 42;  \n");
        assert_eq!(summary, "const answer = 42;");
        let preview = source_preview(&"あ".repeat(300));
        assert_eq!(preview.chars().count(), 241);
    }
}
