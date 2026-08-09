//! Minimal JS runtime backed by `pixi_byte`.
//!
//! Hosts the JS engine on the UI thread and installs a small set of DOM
//! bindings (`console`, `document.getElementById`, element properties).
//! The engine never imports `platform`; DOM access goes through the shared
//! host slot that `JsRuntime` registers on the VM.

use crate::engine::html::{DomTree, HtmlNodeType, Parser as HtmlParser};
use crate::engine::tree::{NodeRef, TreeNode};
use pixi_byte::value::JSArray;
use pixi_byte::value::jsobject::{JSObject, Property};
use pixi_byte::vm::VM;
use pixi_byte::{JSError, JSResult, JSValue};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
const MATHML_NAMESPACE: &str = "http://www.w3.org/1998/Math/MathML";

struct JsTimer {
    id: u64,
    callback: JSValue,
    arguments: Vec<JSValue>,
    deadline: Instant,
    interval: Option<Duration>,
}

struct JsFetchCapability {
    resolve: JSValue,
    reject: JSValue,
}

/// A fetch request waiting to be dispatched by the browser network layer.
pub(crate) struct JsFetchRequest {
    pub(crate) id: u64,
    pub(crate) url: String,
    pub(crate) method: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

/// The response data exposed to a JavaScript `Response` object.
pub(crate) struct JsFetchResponse {
    pub(crate) url: String,
    pub(crate) status: u16,
    pub(crate) status_text: String,
    pub(crate) redirected: bool,
    pub(crate) body: Vec<u8>,
    pub(crate) headers: Vec<(String, String)>,
}

/// State shared between the JS natives and the browser side.
///
/// The JS-facing `u64` counter (`__orinium_dom_id`) maps to a live DOM node so
/// element handles survive relayouts: `Rc` handles on the DOM nodes are stable,
/// while snapshot ids are not.
pub struct JsHost {
    dom: Rc<DomTree>,
    refs: HashMap<
        u64,
        std::rc::Weak<std::cell::RefCell<crate::engine::tree::TreeNode<HtmlNodeType>>>,
    >,
    /// Element JS objects per DOM id, kept alive so `onclick` handlers
    /// registered on them survive and can be invoked on user clicks.
    objects: HashMap<u64, Rc<RefCell<JSObject>>>,
    /// Stable `CSSStyleDeclaration` wrappers for exposed elements.
    styles: HashMap<u64, Rc<RefCell<JSObject>>>,
    /// Explicit namespaces assigned through `document.createElementNS`.
    namespaces: HashMap<u64, String>,
    element_prototype: Rc<RefCell<JSObject>>,
    element_constructor: Rc<RefCell<JSObject>>,
    document: Option<Rc<RefCell<JSObject>>>,
    document_event_listeners: HashMap<String, Vec<JSValue>>,
    element_event_listeners: HashMap<u64, HashMap<String, Vec<JSValue>>>,
    active_element: Option<u64>,
    /// Keeps JS-created or removed nodes alive while their wrappers exist.
    detached_nodes: HashMap<u64, NodeRef<HtmlNodeType>>,
    timers: Vec<JsTimer>,
    fetch_requests: Vec<JsFetchRequest>,
    fetch_capabilities: HashMap<u64, JsFetchCapability>,
    constructing_fetch_capability: Option<JsFetchCapability>,
    next_fetch_id: u64,
    next_timer_id: u64,
    time_origin: Instant,
    dom_content_loaded_fired: bool,
    next_id: u64,
    needs_redraw: Rc<Cell<bool>>,
}

impl JsHost {
    /// Finds the JS-facing DOM id registered for a live DOM node, if any.
    fn dom_id_for_node(&self, node: &NodeRef<HtmlNodeType>) -> Option<u64> {
        self.refs.iter().find_map(|(&dom_id, weak)| {
            if std::ptr::eq(weak.as_ptr(), Rc::as_ptr(node)) {
                Some(dom_id)
            } else {
                None
            }
        })
    }
}

/// A JS engine instance with DOM bindings installed.
pub struct JsRuntime {
    engine: pixi_byte::JSEngine,
    needs_redraw: Rc<Cell<bool>>,
}

impl JsRuntime {
    /// Creates a runtime sharing the given DOM tree with the browser side.
    pub fn new(dom: Rc<DomTree>) -> Self {
        let needs_redraw = Rc::new(Cell::new(false));
        let (element_prototype, element_constructor) = make_element_interface();
        let host = Rc::new(RefCell::new(JsHost {
            dom,
            refs: HashMap::new(),
            objects: HashMap::new(),
            styles: HashMap::new(),
            namespaces: HashMap::new(),
            element_prototype,
            element_constructor,
            document: None,
            document_event_listeners: HashMap::new(),
            element_event_listeners: HashMap::new(),
            active_element: None,
            detached_nodes: HashMap::new(),
            timers: Vec::new(),
            fetch_requests: Vec::new(),
            fetch_capabilities: HashMap::new(),
            constructing_fetch_capability: None,
            next_fetch_id: 0,
            next_timer_id: 0,
            time_origin: Instant::now(),
            dom_content_loaded_fired: false,
            next_id: 0,
            needs_redraw: Rc::clone(&needs_redraw),
        }));

        let mut engine = pixi_byte::JSEngine::new();
        engine.set_host(host);

        install_console(&mut engine);
        install_document(&mut engine);
        install_timers(&mut engine);
        install_performance(&mut engine);
        install_microtasks(&mut engine);
        install_headers(&mut engine);
        install_request(&mut engine);
        install_fetch(&mut engine);
        install_global_aliases(&mut engine);

        Self {
            engine,
            needs_redraw,
        }
    }

    /// Evaluates a script, logging JS errors instead of crashing the page.
    pub fn run_script(&mut self, source: &str) {
        match self.engine.eval(source) {
            Ok(_) => {}
            Err(err) => log::info!("JS error: {}", err),
        }
        self.perform_microtask_checkpoint();
    }

    /// Dispatches `DOMContentLoaded` to document listeners once.
    ///
    /// Returns `true` only for the first dispatch attempt. Listener errors are
    /// logged and do not prevent the remaining listeners from running.
    pub fn dispatch_dom_content_loaded(&mut self) -> bool {
        let Some((document, listeners)) = with_host_mut(self.engine.vm(), |host| {
            if host.dom_content_loaded_fired {
                return None;
            }

            host.dom_content_loaded_fired = true;
            Some((
                host.document.as_ref().cloned(),
                host.document_event_listeners
                    .get("DOMContentLoaded")
                    .cloned()
                    .unwrap_or_default(),
            ))
        })
        .flatten() else {
            return false;
        };

        let Some(document) = document else {
            return true;
        };
        for listener in listeners {
            let event = make_event(
                "DOMContentLoaded",
                Rc::clone(&document),
                Rc::clone(&document),
            );
            if let Err(err) = self.engine.call(
                listener,
                JSValue::Object(Rc::clone(&document)),
                vec![JSValue::Object(event)],
            ) {
                log::info!("JS error on DOMContentLoaded: {}", err);
            }
        }
        self.perform_microtask_checkpoint();
        true
    }

    /// Runs timer callbacks whose deadlines have elapsed.
    ///
    /// Returns whether at least one callback was invoked. Repeating timers are
    /// rescheduled before invocation so they can cancel themselves.
    pub fn run_due_timers(&mut self) -> bool {
        let invocations = with_host_mut(self.engine.vm(), |host| {
            let now = Instant::now();
            let mut invocations = Vec::new();
            let mut index = 0;
            while index < host.timers.len() {
                if host.timers[index].deadline > now {
                    index += 1;
                    continue;
                }

                let callback = host.timers[index].callback.clone();
                let arguments = host.timers[index].arguments.clone();
                if let Some(interval) = host.timers[index].interval {
                    host.timers[index].deadline = now + interval;
                    index += 1;
                } else {
                    host.timers.remove(index);
                }
                invocations.push((callback, arguments));
            }
            invocations
        })
        .unwrap_or_default();

        let ran_callback = !invocations.is_empty();
        for (callback, arguments) in invocations {
            if let Err(err) = self.engine.call(callback, JSValue::Undefined, arguments) {
                log::info!("JS error in timer callback: {}", err);
            }
            self.perform_microtask_checkpoint();
        }
        ran_callback
    }

    /// Returns whether a script mutated the DOM and a relayout is needed.
    pub fn needs_redraw(&self) -> bool {
        self.needs_redraw.get()
    }

    /// Clears and returns the redraw flag.
    pub fn take_needs_redraw(&self) -> bool {
        self.needs_redraw.replace(false)
    }

    /// Takes fetch requests queued by JavaScript since the previous call.
    pub(crate) fn take_fetch_requests(&mut self) -> Vec<JsFetchRequest> {
        with_host_mut(self.engine.vm(), |host| {
            std::mem::take(&mut host.fetch_requests)
        })
        .unwrap_or_default()
    }

    /// Resolves a pending JavaScript fetch and runs its microtask checkpoint.
    pub(crate) fn resolve_fetch(&mut self, id: u64, response: JsFetchResponse) {
        let capability =
            with_host_mut(self.engine.vm(), |host| host.fetch_capabilities.remove(&id)).flatten();
        let Some(capability) = capability else {
            return;
        };
        let response = make_fetch_response(response);
        if let Err(err) = self.engine.call(
            capability.resolve,
            JSValue::Undefined,
            vec![JSValue::Object(response)],
        ) {
            log::info!("JS error while resolving fetch: {}", err);
        }
        self.perform_microtask_checkpoint();
    }

    /// Rejects a pending JavaScript fetch and runs its microtask checkpoint.
    pub(crate) fn reject_fetch(&mut self, id: u64, reason: String) {
        let capability =
            with_host_mut(self.engine.vm(), |host| host.fetch_capabilities.remove(&id)).flatten();
        let Some(capability) = capability else {
            return;
        };
        if let Err(err) = self.engine.call(
            capability.reject,
            JSValue::Undefined,
            vec![JSValue::String(reason)],
        ) {
            log::info!("JS error while rejecting fetch: {}", err);
        }
        self.perform_microtask_checkpoint();
    }

    /// Dispatches a click to the handlers registered on `node`.
    ///
    /// Both the `onclick` property and `addEventListener("click", ...)` are
    /// supported. The event bubbles through exposed ancestor elements so
    /// delegated listeners such as React's root listener receive it.
    /// Returns whether at least one handler ran.
    pub fn click(&mut self, node: &NodeRef<HtmlNodeType>) -> bool {
        let mut path = Vec::new();
        let mut current = Some(Rc::clone(node));
        while let Some(node) = current {
            current = node.borrow().parent();
            if let Some(JSValue::Object(object)) = expose_node(self.engine.vm(), node) {
                path.push(object);
            }
        }
        let Some(target) = path.first().cloned() else {
            return false;
        };

        let mut ran_handler = false;
        for current_target in path {
            let Some(dom_id) = node_dom_id(&JSValue::Object(Rc::clone(&current_target))) else {
                continue;
            };
            let onclick = current_target.borrow().get("onclick");
            let listeners = with_host(self.engine.vm(), |host| {
                host.element_event_listeners
                    .get(&dom_id)
                    .and_then(|events| events.get("click"))
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap_or_default();
            let has_onclick = is_callable(&onclick);
            if !has_onclick && listeners.is_empty() {
                continue;
            }

            ran_handler = true;
            let event = make_event(
                "click",
                Rc::clone(&target),
                Rc::clone(&current_target),
            );
            if has_onclick {
                if let Err(err) = self.engine.call(
                    onclick,
                    JSValue::Object(Rc::clone(&current_target)),
                    vec![JSValue::Object(Rc::clone(&event))],
                ) {
                    log::info!("JS error in onclick: {}", err);
                }
            }
            if !event_flag(&event, "__orinium_immediate_propagation_stopped") {
                for listener in listeners {
                    if let Err(err) = self.engine.call(
                        listener,
                        JSValue::Object(Rc::clone(&current_target)),
                        vec![JSValue::Object(Rc::clone(&event))],
                    ) {
                        log::info!("JS error in click listener: {}", err);
                    }
                    if event_flag(&event, "__orinium_immediate_propagation_stopped") {
                        break;
                    }
                }
            }
            if event_flag(&event, "cancelBubble") {
                break;
            }
        }
        if ran_handler {
            self.perform_microtask_checkpoint();
        }
        ran_handler
    }

    /// Drains queued microtasks in FIFO order, including jobs queued by jobs.
    fn perform_microtask_checkpoint(&mut self) {
        while let Err(err) = self.engine.run_jobs() {
            // A failed callback must not prevent later jobs in the same
            // checkpoint from running. PixiByte leaves those jobs queued.
            log::info!("JS error in microtask: {}", err);
        }
    }
}

// --- helpers ---

/// Runs `f` with an immutable borrow of the host data, if set and downcastable.
fn with_host<R>(vm: &VM, f: impl FnOnce(&JsHost) -> R) -> Option<R> {
    let host = vm.host.as_ref()?;
    let host_ref = host.borrow();
    let js_host = (&*host_ref as &dyn Any).downcast_ref::<JsHost>()?;
    Some(f(js_host))
}

/// Runs `f` with a mutable borrow of the host data, if set and downcastable.
fn with_host_mut<R>(vm: &VM, f: impl FnOnce(&mut JsHost) -> R) -> Option<R> {
    let host = vm.host.as_ref()?;
    let mut host_ref = host.borrow_mut();
    let js_host = (&mut *host_ref as &mut dyn Any).downcast_mut::<JsHost>()?;
    Some(f(js_host))
}

/// Records a DOM mutation: bumps the tree version and flags a relayout.
fn mark_dom_dirty(vm: &VM) {
    if let Some(host) = vm.host.as_ref() {
        let host_ref = host.borrow();
        if let Some(js_host) = (&*host_ref as &dyn Any).downcast_ref::<JsHost>() {
            js_host.dom.mark_dirty();
            js_host.needs_redraw.set(true);
        }
    }
}

/// Extracts the hidden DOM id counter from an element `this` object.
fn node_dom_id(this: &JSValue) -> Option<u64> {
    let JSValue::Object(obj) = this else {
        return None;
    };
    let JSValue::Number(n) = obj.borrow().get("__orinium_dom_id") else {
        return None;
    };
    Some(n as u64)
}

/// Resolves the `this` element back to a live DOM node (dead node -> None).
fn dom_node(vm: &VM, this: &JSValue) -> Option<NodeRef<HtmlNodeType>> {
    let dom_id = node_dom_id(this)?;
    with_host(vm, |host| host.refs.get(&dom_id).and_then(|w| w.upgrade())).flatten()
}

// --- global aliases ---

fn install_global_aliases(engine: &mut pixi_byte::JSEngine) {
    let global = Rc::clone(engine.global_mut());
    let mut global_object = global.borrow_mut();
    global_object.set(
        "addEventListener".to_string(),
        JSValue::NativeFunction(add_document_event_listener),
    );
    global_object.set(
        "removeEventListener".to_string(),
        JSValue::NativeFunction(remove_document_event_listener),
    );
    for name in ["window", "self", "globalThis"] {
        global_object.set(name.to_string(), JSValue::Object(Rc::clone(&global)));
    }
}

// --- console ---

fn install_console(engine: &mut pixi_byte::JSEngine) {
    let console_obj = Rc::new(RefCell::new(JSObject::new()));
    {
        let mut console = console_obj.borrow_mut();
        console.set("log".to_string(), JSValue::NativeFunction(console_log));
        console.set("warn".to_string(), JSValue::NativeFunction(console_warn));
        console.set("error".to_string(), JSValue::NativeFunction(console_error));
    }
    engine
        .global_mut()
        .borrow_mut()
        .set("console".to_string(), JSValue::Object(console_obj));
}

fn console_message(vm: &mut VM, args: Vec<JSValue>, level: log::Level) -> JSResult<JSValue> {
    let message: Vec<String> = args.iter().skip(1).map(|v| v.to_console_string()).collect();
    log::log!(level, "{}", message.join(" "));
    let _ = vm;
    Ok(JSValue::Undefined)
}

fn console_log(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    console_message(vm, args, log::Level::Info)
}

fn console_warn(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    console_message(vm, args, log::Level::Warn)
}

fn console_error(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    console_message(vm, args, log::Level::Error)
}

// --- timers ---

fn install_timers(engine: &mut pixi_byte::JSEngine) {
    let mut global = engine.global_mut().borrow_mut();
    global.set(
        "setTimeout".to_string(),
        JSValue::NativeFunction(set_timeout),
    );
    global.set(
        "clearTimeout".to_string(),
        JSValue::NativeFunction(clear_timer),
    );
    global.set(
        "setInterval".to_string(),
        JSValue::NativeFunction(set_interval),
    );
    global.set(
        "clearInterval".to_string(),
        JSValue::NativeFunction(clear_timer),
    );
}

fn set_timeout(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    schedule_timer(vm, args, false)
}

fn set_interval(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    schedule_timer(vm, args, true)
}

fn schedule_timer(vm: &mut VM, args: Vec<JSValue>, repeating: bool) -> JSResult<JSValue> {
    let Some(callback) = args.get(1).filter(|value| is_callable(value)).cloned() else {
        return Ok(JSValue::Number(0.0));
    };
    let delay = timer_delay(args.get(2));
    let arguments = args.into_iter().skip(3).collect();
    let Some(id) = with_host_mut(vm, |host| {
        host.next_timer_id += 1;
        let id = host.next_timer_id;
        host.timers.push(JsTimer {
            id,
            callback,
            arguments,
            deadline: Instant::now() + delay,
            interval: repeating.then_some(delay),
        });
        id
    }) else {
        return Ok(JSValue::Number(0.0));
    };
    Ok(JSValue::Number(id as f64))
}

fn timer_delay(value: Option<&JSValue>) -> Duration {
    let milliseconds = value.map(JSValue::to_number).unwrap_or(0.0);
    if !milliseconds.is_finite() || milliseconds <= 0.0 {
        Duration::ZERO
    } else {
        let milliseconds = milliseconds.min(i32::MAX as f64);
        Duration::from_secs_f64(milliseconds / 1000.0)
    }
}

fn clear_timer(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let id = args.get(1).map(JSValue::to_number).unwrap_or(0.0) as u64;
    let _ = with_host_mut(vm, |host| {
        host.timers.retain(|timer| timer.id != id);
    });
    Ok(JSValue::Undefined)
}

// --- performance ---

fn install_performance(engine: &mut pixi_byte::JSEngine) {
    let mut performance = JSObject::new();
    performance.set(
        "now".to_string(),
        JSValue::NativeFunction(performance_now),
    );
    engine.global_mut().borrow_mut().set(
        "performance".to_string(),
        JSValue::Object(Rc::new(RefCell::new(performance))),
    );
}

fn performance_now(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let milliseconds = with_host(vm, |host| host.time_origin.elapsed().as_secs_f64() * 1_000.0)
        .unwrap_or(0.0);
    Ok(JSValue::Number(milliseconds))
}

// --- microtasks ---

fn install_microtasks(engine: &mut pixi_byte::JSEngine) {
    engine.global_mut().borrow_mut().set(
        "queueMicrotask".to_string(),
        JSValue::NativeFunction(queue_microtask),
    );
}

fn queue_microtask(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(callback) = args.get(1).filter(|value| is_callable(value)).cloned() else {
        return Err(JSError::TypeError(
            "queueMicrotask callback must be callable".to_string(),
        ));
    };
    vm.enqueue_job(callback, JSValue::Undefined, Vec::new());
    Ok(JSValue::Undefined)
}

// --- fetch ---

const HEADERS_DATA: &str = "__orinium_headers_data";
const HEADERS_IMMUTABLE: &str = "__orinium_headers_immutable";
const REQUEST_MARKER: &str = "__orinium_request";
const REQUEST_BODY: &str = "__orinium_request_body";
const RESPONSE_BODY_USED: &str = "__orinium_response_body_used";

fn install_headers(engine: &mut pixi_byte::JSEngine) {
    let mut constructor = JSObject::new();
    constructor.set(
        "__construct__".to_string(),
        JSValue::NativeFunction(headers_constructor),
    );
    engine.global_mut().borrow_mut().set(
        "Headers".to_string(),
        JSValue::Object(Rc::new(RefCell::new(constructor))),
    );
}

fn headers_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let entries = args.get(1).map(extract_header_entries).unwrap_or_default();
    Ok(JSValue::Object(make_headers(entries, false)))
}

fn make_headers(entries: Vec<(String, String)>, immutable: bool) -> Rc<RefCell<JSObject>> {
    let data = Rc::new(RefCell::new(JSObject::new()));
    for (name, value) in entries {
        append_header_value(&mut data.borrow_mut(), &name, &value);
    }

    let mut headers = JSObject::new();
    headers.set(HEADERS_DATA.to_string(), JSValue::Object(data));
    headers.set(HEADERS_IMMUTABLE.to_string(), JSValue::Boolean(immutable));
    headers.set("get".to_string(), JSValue::NativeFunction(headers_get));
    headers.set("has".to_string(), JSValue::NativeFunction(headers_has));
    headers.set("set".to_string(), JSValue::NativeFunction(headers_set));
    headers.set(
        "append".to_string(),
        JSValue::NativeFunction(headers_append),
    );
    headers.set(
        "delete".to_string(),
        JSValue::NativeFunction(headers_delete),
    );
    Rc::new(RefCell::new(headers))
}

fn extract_header_entries(value: &JSValue) -> Vec<(String, String)> {
    let JSValue::Object(object) = value else {
        return Vec::new();
    };
    let object = object.borrow();
    if let JSValue::Object(data) = object.get(HEADERS_DATA) {
        let data = data.borrow();
        return data
            .keys()
            .into_iter()
            .map(|name| {
                let value = data.get(&name).to_string();
                (name, value)
            })
            .collect();
    }
    object
        .keys()
        .into_iter()
        .map(|name| {
            let value = object.get(&name).to_string();
            (name, value)
        })
        .collect()
}

fn headers_data(args: &[JSValue]) -> JSResult<Rc<RefCell<JSObject>>> {
    let Some(JSValue::Object(headers)) = args.first() else {
        return Err(JSError::TypeError(
            "Headers method called on incompatible receiver".to_string(),
        ));
    };
    let data = headers.borrow().get(HEADERS_DATA);
    match data {
        JSValue::Object(data) => Ok(data),
        _ => Err(JSError::TypeError(
            "Headers method called on incompatible receiver".to_string(),
        )),
    }
}

fn ensure_headers_mutable(args: &[JSValue]) -> JSResult<()> {
    let Some(JSValue::Object(headers)) = args.first() else {
        return Err(JSError::TypeError(
            "Headers method called on incompatible receiver".to_string(),
        ));
    };
    if matches!(
        headers.borrow().get(HEADERS_IMMUTABLE),
        JSValue::Boolean(true)
    ) {
        return Err(JSError::TypeError(
            "Response headers are immutable".to_string(),
        ));
    }
    Ok(())
}

fn header_argument(args: &[JSValue], index: usize, label: &str) -> JSResult<String> {
    let Some(value) = args.get(index) else {
        return Err(JSError::TypeError(format!("Missing header {label}")));
    };
    Ok(value.to_string())
}

fn normalize_header_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn normalize_header_value(value: &str) -> String {
    value.trim().to_string()
}

fn append_header_value(data: &mut JSObject, name: &str, value: &str) {
    let name = normalize_header_name(name);
    if name.is_empty() {
        return;
    }
    let value = normalize_header_value(value);
    let combined = match data.get(&name) {
        JSValue::Undefined => value,
        current => format!("{}, {}", current.to_string(), value),
    };
    data.set(name, JSValue::String(combined));
}

fn headers_get(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let data = headers_data(&args)?;
    let name = normalize_header_name(&header_argument(&args, 1, "name")?);
    Ok(match data.borrow().get(&name) {
        JSValue::Undefined => JSValue::Null,
        value => value,
    })
}

fn headers_has(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let data = headers_data(&args)?;
    let name = normalize_header_name(&header_argument(&args, 1, "name")?);
    let has = data.borrow().has_own_property(&name);
    Ok(JSValue::Boolean(has))
}

fn headers_set(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    ensure_headers_mutable(&args)?;
    let data = headers_data(&args)?;
    let name = normalize_header_name(&header_argument(&args, 1, "name")?);
    let value = normalize_header_value(&header_argument(&args, 2, "value")?);
    if !name.is_empty() {
        data.borrow_mut().set(name, JSValue::String(value));
    }
    Ok(JSValue::Undefined)
}

fn headers_append(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    ensure_headers_mutable(&args)?;
    let data = headers_data(&args)?;
    let name = header_argument(&args, 1, "name")?;
    let value = header_argument(&args, 2, "value")?;
    append_header_value(&mut data.borrow_mut(), &name, &value);
    Ok(JSValue::Undefined)
}

fn headers_delete(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    ensure_headers_mutable(&args)?;
    let data = headers_data(&args)?;
    let name = normalize_header_name(&header_argument(&args, 1, "name")?);
    data.borrow_mut().delete(&name);
    Ok(JSValue::Undefined)
}

struct RequestParts {
    url: String,
    method: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn install_request(engine: &mut pixi_byte::JSEngine) {
    let mut constructor = JSObject::new();
    constructor.set(
        "__construct__".to_string(),
        JSValue::NativeFunction(request_constructor),
    );
    engine.global_mut().borrow_mut().set(
        "Request".to_string(),
        JSValue::Object(Rc::new(RefCell::new(constructor))),
    );
}

fn request_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(input) = args.get(1) else {
        return Err(JSError::TypeError(
            "Request input is required".to_string(),
        ));
    };
    let parts = request_parts(input, args.get(2));
    Ok(JSValue::Object(make_request(parts)))
}

fn make_request(parts: RequestParts) -> Rc<RefCell<JSObject>> {
    let mut request = JSObject::new();
    request.define_property(
        "url".to_string(),
        Property::read_only(JSValue::String(parts.url)),
    );
    request.define_property(
        "method".to_string(),
        Property::read_only(JSValue::String(parts.method)),
    );
    request.define_property(
        "headers".to_string(),
        Property::read_only(JSValue::Object(make_headers(parts.headers, false))),
    );
    request.set(
        REQUEST_BODY.to_string(),
        JSValue::String(String::from_utf8_lossy(&parts.body).into_owned()),
    );
    request.set(REQUEST_MARKER.to_string(), JSValue::Boolean(true));
    Rc::new(RefCell::new(request))
}

fn request_parts(input: &JSValue, init: Option<&JSValue>) -> RequestParts {
    let mut parts = match input {
        JSValue::Object(request)
            if matches!(request.borrow().get(REQUEST_MARKER), JSValue::Boolean(true)) =>
        {
            let request = request.borrow();
            RequestParts {
                url: request.get("url").to_string(),
                method: request.get("method").to_string(),
                headers: extract_header_entries(&request.get("headers")),
                body: match request.get(REQUEST_BODY) {
                    JSValue::String(body) => body.into_bytes(),
                    _ => Vec::new(),
                },
            }
        }
        value => RequestParts {
            url: value.to_string(),
            method: "GET".to_string(),
            headers: Vec::new(),
            body: Vec::new(),
        },
    };
    apply_request_init(&mut parts, init);
    parts
}

fn apply_request_init(parts: &mut RequestParts, init: Option<&JSValue>) {
    let Some(JSValue::Object(init)) = init else {
        return;
    };
    let init = init.borrow();
    if init.has_own_property("method") {
        match init.get("method") {
            JSValue::Undefined | JSValue::Null => {}
            value => parts.method = value.to_string().to_ascii_uppercase(),
        }
    }
    if init.has_own_property("headers") {
        parts.headers = extract_header_entries(&init.get("headers"));
    }
    if init.has_own_property("body") {
        parts.body = match init.get("body") {
            JSValue::Undefined | JSValue::Null => Vec::new(),
            value => value.to_string().into_bytes(),
        };
    }
}

fn install_fetch(engine: &mut pixi_byte::JSEngine) {
    engine
        .global_mut()
        .borrow_mut()
        .set("fetch".to_string(), JSValue::NativeFunction(fetch));
}

fn fetch(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let input = args.get(1).cloned().unwrap_or(JSValue::Undefined);
    let RequestParts {
        url,
        method,
        headers,
        body,
    } = request_parts(&input, args.get(2));
    let promise_constructor = vm.global_object.borrow().get("Promise");
    let JSValue::Object(constructor) = &promise_constructor else {
        return Err(JSError::InternalError(
            "Promise constructor is unavailable".to_string(),
        ));
    };
    let construct = constructor.borrow().get("__construct__");
    let _ = with_host_mut(vm, |host| host.constructing_fetch_capability = None);
    let promise = vm.call(
        construct,
        promise_constructor,
        vec![JSValue::NativeFunction(capture_fetch_capability)],
    )?;
    let capability = with_host_mut(vm, |host| host.constructing_fetch_capability.take())
        .flatten()
        .ok_or_else(|| JSError::InternalError("Failed to create fetch Promise".to_string()))?;

    let _ = with_host_mut(vm, |host| {
        host.next_fetch_id += 1;
        let id = host.next_fetch_id;
        host.fetch_capabilities.insert(id, capability);
        host.fetch_requests.push(JsFetchRequest {
            id,
            url,
            method,
            headers,
            body,
        });
    });
    Ok(promise)
}

fn capture_fetch_capability(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let resolve = args.get(1).cloned().unwrap_or(JSValue::Undefined);
    let reject = args.get(2).cloned().unwrap_or(JSValue::Undefined);
    let Some(()) = with_host_mut(vm, |host| {
        host.constructing_fetch_capability = Some(JsFetchCapability { resolve, reject });
    }) else {
        return Err(JSError::InternalError(
            "Fetch host state is unavailable".to_string(),
        ));
    };
    Ok(JSValue::Undefined)
}

fn make_fetch_response(response: JsFetchResponse) -> Rc<RefCell<JSObject>> {
    let mut object = JSObject::new();
    object.define_property(
        "headers".to_string(),
        Property::read_only(JSValue::Object(make_headers(response.headers, true))),
    );
    object.define_property(
        "ok".to_string(),
        Property::read_only(JSValue::Boolean((200..=299).contains(&response.status))),
    );
    object.define_property(
        "status".to_string(),
        Property::read_only(JSValue::Number(response.status as f64)),
    );
    object.define_property(
        "statusText".to_string(),
        Property::read_only(JSValue::String(response.status_text)),
    );
    object.define_property(
        "redirected".to_string(),
        Property::read_only(JSValue::Boolean(response.redirected)),
    );
    object.define_property(
        "bodyUsed".to_string(),
        Property {
            value: JSValue::Undefined,
            enumerable: true,
            writable: false,
            configurable: false,
            getter: Some(JSValue::NativeFunction(fetch_response_body_used)),
            setter: None,
        },
    );
    object.define_property(
        "url".to_string(),
        Property::read_only(JSValue::String(response.url)),
    );
    object.define_property(
        "text".to_string(),
        Property::read_only(JSValue::NativeFunction(fetch_response_text)),
    );
    object.define_property(
        "json".to_string(),
        Property::read_only(JSValue::NativeFunction(fetch_response_json)),
    );
    object.set(
        "__orinium_response_body".to_string(),
        JSValue::String(String::from_utf8_lossy(&response.body).into_owned()),
    );
    object.set(RESPONSE_BODY_USED.to_string(), JSValue::Boolean(false));
    Rc::new(RefCell::new(object))
}

fn fetch_response_text(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let body = match consume_response_body(vm, &args, "text")? {
        Ok(body) => body,
        Err(rejection) => return Ok(rejection),
    };
    settle_promise(vm, "resolve", JSValue::String(body))
}

fn fetch_response_json(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let body = match consume_response_body(vm, &args, "json")? {
        Ok(body) => body,
        Err(rejection) => return Ok(rejection),
    };
    match serde_json::from_str::<serde_json::Value>(&body) {
        Ok(value) => settle_promise(vm, "resolve", json_to_js_value(value)),
        Err(error) => settle_promise(
            vm,
            "reject",
            JSValue::String(format!("Failed to parse JSON: {error}")),
        ),
    }
}

fn fetch_response_body_used(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(JSValue::Object(response)) = args.first() else {
        return Err(JSError::TypeError(
            "Response.bodyUsed called on incompatible receiver".to_string(),
        ));
    };
    Ok(response.borrow().get(RESPONSE_BODY_USED))
}

fn consume_response_body(
    vm: &mut VM,
    args: &[JSValue],
    method: &str,
) -> JSResult<Result<String, JSValue>> {
    let Some(JSValue::Object(response)) = args.first() else {
        return Err(JSError::TypeError(format!(
            "Response.{method} called on incompatible receiver"
        )));
    };
    let mut response = response.borrow_mut();
    if matches!(response.get(RESPONSE_BODY_USED), JSValue::Boolean(true)) {
        let rejection = settle_promise(
            vm,
            "reject",
            JSValue::String("Response body has already been consumed".to_string()),
        )?;
        return Ok(Err(rejection));
    }
    response.set(RESPONSE_BODY_USED.to_string(), JSValue::Boolean(true));
    match response.get("__orinium_response_body") {
        JSValue::String(body) => Ok(Ok(body)),
        _ => Err(JSError::InternalError(
            "Response body is unavailable".to_string(),
        )),
    }
}

fn settle_promise(vm: &mut VM, method: &str, value: JSValue) -> JSResult<JSValue> {
    let promise = vm.global_object.borrow().get("Promise");
    let JSValue::Object(constructor) = &promise else {
        return Err(JSError::InternalError(
            "Promise constructor is unavailable".to_string(),
        ));
    };
    let settle = constructor.borrow().get(method);
    vm.call(settle, promise, vec![value])
}

fn json_to_js_value(value: serde_json::Value) -> JSValue {
    match value {
        serde_json::Value::Null => JSValue::Null,
        serde_json::Value::Bool(value) => JSValue::Boolean(value),
        serde_json::Value::Number(value) => JSValue::Number(value.as_f64().unwrap_or(f64::NAN)),
        serde_json::Value::String(value) => JSValue::String(value),
        serde_json::Value::Array(values) => {
            JSArray::from_vec(values.into_iter().map(json_to_js_value).collect()).to_object()
        }
        serde_json::Value::Object(properties) => {
            let mut object = JSObject::new();
            for (key, value) in properties {
                object.set(key, json_to_js_value(value));
            }
            JSValue::Object(Rc::new(RefCell::new(object)))
        }
    }
}

// --- document ---

fn install_document(engine: &mut pixi_byte::JSEngine) {
    let document_obj = Rc::new(RefCell::new(JSObject::new()));
    {
        let mut document = document_obj.borrow_mut();
        document.define_property(
            "nodeType".to_string(),
            Property::read_only(JSValue::Number(9.0)),
        );
        document.define_property(
            "nodeName".to_string(),
            Property::read_only(JSValue::String("#document".to_string())),
        );
        document.define_property(
            "documentElement".to_string(),
            read_only_accessor_property(get_document_element),
        );
        document.define_property(
            "body".to_string(),
            read_only_accessor_property(get_document_body),
        );
        document.define_property(
            "activeElement".to_string(),
            read_only_accessor_property(get_active_element),
        );
        document.set(
            "hasFocus".to_string(),
            JSValue::NativeFunction(document_has_focus),
        );
        document.set(
            "getElementById".to_string(),
            JSValue::NativeFunction(get_element_by_id),
        );
        document.set(
            "querySelector".to_string(),
            JSValue::NativeFunction(document_query_selector),
        );
        document.set(
            "querySelectorAll".to_string(),
            JSValue::NativeFunction(document_query_selector_all),
        );
        document.set(
            "createElement".to_string(),
            JSValue::NativeFunction(create_element),
        );
        document.set(
            "createElementNS".to_string(),
            JSValue::NativeFunction(create_element_ns),
        );
        document.set(
            "createTextNode".to_string(),
            JSValue::NativeFunction(create_text_node),
        );
        document.set(
            "addEventListener".to_string(),
            JSValue::NativeFunction(add_document_event_listener),
        );
        document.set(
            "removeEventListener".to_string(),
            JSValue::NativeFunction(remove_document_event_listener),
        );
    }
    let _ = with_host_mut(engine.vm(), |host| {
        host.document = Some(Rc::clone(&document_obj));
    });
    engine
        .global_mut()
        .borrow_mut()
        .set("document".to_string(), JSValue::Object(document_obj));

    if let Some(element_constructor) = with_host(engine.vm(), |host| {
        Rc::clone(&host.element_constructor)
    }) {
        let mut global = engine.global_mut().borrow_mut();
        global.set(
            "Element".to_string(),
            JSValue::Object(Rc::clone(&element_constructor)),
        );
        global.set(
            "HTMLElement".to_string(),
            JSValue::Object(element_constructor),
        );
    }

    let mut iframe_constructor = JSObject::new();
    iframe_constructor.set(
        "__host_has_instance__".to_string(),
        JSValue::NativeFunction(html_iframe_element_has_instance),
    );
    engine.global_mut().borrow_mut().set(
        "HTMLIFrameElement".to_string(),
        JSValue::Object(Rc::new(RefCell::new(iframe_constructor))),
    );
}

fn html_iframe_element_has_instance(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = args.get(1).and_then(|value| dom_node(vm, value)) else {
        return Ok(JSValue::Boolean(false));
    };
    let is_iframe = node.borrow().value.tag_name() == Some("iframe");
    Ok(JSValue::Boolean(is_iframe))
}

fn add_document_event_listener(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(JSValue::String(event_type)) = args.get(1) else {
        return Ok(JSValue::Undefined);
    };
    let Some(listener) = args.get(2).filter(|value| is_callable(value)).cloned() else {
        return Ok(JSValue::Undefined);
    };

    let _ = with_host_mut(vm, |host| {
        let listeners = host
            .document_event_listeners
            .entry(event_type.clone())
            .or_default();
        if !listeners
            .iter()
            .any(|candidate| candidate.strict_equals(&listener))
        {
            listeners.push(listener);
        }
    });
    Ok(JSValue::Undefined)
}

fn remove_document_event_listener(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(JSValue::String(event_type)) = args.get(1) else {
        return Ok(JSValue::Undefined);
    };
    let Some(listener) = args.get(2) else {
        return Ok(JSValue::Undefined);
    };
    let _ = with_host_mut(vm, |host| {
        if let Some(listeners) = host.document_event_listeners.get_mut(event_type) {
            listeners.retain(|candidate| !candidate.strict_equals(listener));
        }
    });
    Ok(JSValue::Undefined)
}

fn is_callable(value: &JSValue) -> bool {
    matches!(
        value,
        JSValue::Function(..)
            | JSValue::ArrowFunction(..)
            | JSValue::NativeFunction(_)
            | JSValue::BoundFunction(..)
    )
}

fn make_event(
    event_type: &str,
    target: Rc<RefCell<JSObject>>,
    current_target: Rc<RefCell<JSObject>>,
) -> Rc<RefCell<JSObject>> {
    let mut event = JSObject::new();
    event.define_property(
        "type".to_string(),
        Property::read_only(JSValue::String(event_type.to_string())),
    );
    event.define_property(
        "target".to_string(),
        Property::read_only(JSValue::Object(Rc::clone(&target))),
    );
    event.define_property(
        "currentTarget".to_string(),
        Property::read_only(JSValue::Object(current_target)),
    );
    event.define_property(
        "bubbles".to_string(),
        Property::read_only(JSValue::Boolean(true)),
    );
    event.define_property(
        "cancelable".to_string(),
        Property::read_only(JSValue::Boolean(true)),
    );
    event.set("defaultPrevented".to_string(), JSValue::Boolean(false));
    event.set("cancelBubble".to_string(), JSValue::Boolean(false));
    event.set(
        "preventDefault".to_string(),
        JSValue::NativeFunction(event_prevent_default),
    );
    event.set(
        "stopPropagation".to_string(),
        JSValue::NativeFunction(event_stop_propagation),
    );
    event.set(
        "stopImmediatePropagation".to_string(),
        JSValue::NativeFunction(event_stop_immediate_propagation),
    );
    Rc::new(RefCell::new(event))
}

fn event_flag(event: &Rc<RefCell<JSObject>>, name: &str) -> bool {
    matches!(event.borrow().get(name), JSValue::Boolean(true))
}

fn event_prevent_default(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    if let Some(JSValue::Object(event)) = args.first() {
        event
            .borrow_mut()
            .set("defaultPrevented".to_string(), JSValue::Boolean(true));
    }
    Ok(JSValue::Undefined)
}

fn event_stop_propagation(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    if let Some(JSValue::Object(event)) = args.first() {
        event
            .borrow_mut()
            .set("cancelBubble".to_string(), JSValue::Boolean(true));
    }
    Ok(JSValue::Undefined)
}

fn event_stop_immediate_propagation(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    event_stop_propagation(vm, args.clone())?;
    if let Some(JSValue::Object(event)) = args.first() {
        event.borrow_mut().set(
            "__orinium_immediate_propagation_stopped".to_string(),
            JSValue::Boolean(true),
        );
    }
    Ok(JSValue::Undefined)
}

fn get_element_by_id(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(JSValue::String(id)) = args.get(1) else {
        return Ok(JSValue::Null);
    };

    let Some(node) = with_host(vm, |host| host.dom.get_element_by_id(id)).flatten() else {
        return Ok(JSValue::Null);
    };
    Ok(expose_node(vm, node).unwrap_or(JSValue::Null))
}

fn document_query_selector(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(JSValue::String(selector)) = args.get(1) else {
        return Ok(JSValue::Null);
    };
    let Some(node) = with_host(vm, |host| host.dom.query_selector(selector)).flatten() else {
        return Ok(JSValue::Null);
    };
    Ok(expose_node(vm, node).unwrap_or(JSValue::Null))
}

fn document_query_selector_all(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(JSValue::String(selector)) = args.get(1) else {
        return Ok(JSArray::new().to_object());
    };
    let nodes = with_host(vm, |host| host.dom.query_selector_all(selector)).unwrap_or_default();
    Ok(expose_node_list(vm, nodes))
}

fn create_element(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(JSValue::String(tag_name)) = args.get(1) else {
        return Ok(JSValue::Null);
    };
    let tag_name = tag_name.trim().to_ascii_lowercase();
    if tag_name.is_empty() {
        return Ok(JSValue::Null);
    }
    let node = TreeNode::new(HtmlNodeType::Element {
        tag_name,
        attributes: Vec::new(),
    });
    Ok(expose_detached_node(vm, node).unwrap_or(JSValue::Null))
}

fn create_element_ns(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let namespace = match args.get(1) {
        Some(JSValue::String(namespace)) => namespace.clone(),
        Some(JSValue::Null) | Some(JSValue::Undefined) | None => String::new(),
        Some(value) => value.to_console_string(),
    };
    let Some(JSValue::String(qualified_name)) = args.get(2) else {
        return Ok(JSValue::Null);
    };
    let tag_name = qualified_name.trim().to_ascii_lowercase();
    if tag_name.is_empty() {
        return Ok(JSValue::Null);
    }

    let node = TreeNode::new(HtmlNodeType::Element {
        tag_name,
        attributes: Vec::new(),
    });
    let value = expose_detached_node(vm, node).unwrap_or(JSValue::Null);
    if let Some(dom_id) = node_dom_id(&value) {
        let _ = with_host_mut(vm, |host| {
            host.namespaces.insert(dom_id, namespace);
        });
    }
    Ok(value)
}

fn create_text_node(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let text = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    let node = TreeNode::new(HtmlNodeType::Text(text));
    Ok(expose_detached_node(vm, node).unwrap_or(JSValue::Null))
}

fn get_document_element(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let node = with_host(vm, |host| {
        host.dom
            .root
            .borrow()
            .children()
            .iter()
            .find(|child| matches!(child.borrow().value, HtmlNodeType::Element { .. }))
            .cloned()
    })
    .flatten();
    Ok(node
        .and_then(|node| expose_node(vm, node))
        .unwrap_or(JSValue::Null))
}

fn get_document_body(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let node = with_host(vm, |host| host.dom.query_selector("body")).flatten();
    Ok(node
        .and_then(|node| expose_node(vm, node))
        .unwrap_or(JSValue::Null))
}

fn get_active_element(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let active = with_host(vm, |host| {
        host.active_element
            .and_then(|dom_id| host.objects.get(&dom_id).cloned())
    })
    .flatten();
    if let Some(active) = active {
        return Ok(JSValue::Object(active));
    }
    get_document_body(vm, Vec::new())
}

fn document_has_focus(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::Boolean(true))
}

fn expose_detached_node(vm: &mut VM, node: NodeRef<HtmlNodeType>) -> Option<JSValue> {
    let value = expose_node(vm, Rc::clone(&node))?;
    let dom_id = node_dom_id(&value)?;
    with_host_mut(vm, |host| {
        host.detached_nodes.insert(dom_id, node);
    })?;
    Some(value)
}

fn expose_node(vm: &mut VM, node: NodeRef<HtmlNodeType>) -> Option<JSValue> {
    let node_kind = {
        let node = node.borrow();
        match &node.value {
            HtmlNodeType::Element { tag_name, .. } => Some((
                tag_name.clone(),
                node.value.get_attr("id").unwrap_or("").to_string(),
            )),
            HtmlNodeType::Text(_) => None,
            HtmlNodeType::Document => {
                return with_host(vm, |host| host.document.as_ref().cloned())
                    .flatten()
                    .map(JSValue::Object);
            }
            _ => return None,
        }
    };

    // Register the live node so later property access can resolve it. Reuse
    // the existing id and element object when this node was already exposed.
    let dom_id = with_host_mut(vm, |host| {
        if let Some(dom_id) = host.dom_id_for_node(&node) {
            return dom_id;
        }
        host.next_id += 1;
        let dom_id = host.next_id;
        host.refs.insert(dom_id, Rc::downgrade(&node));
        dom_id
    })?;

    let obj = with_host_mut(vm, |host| {
        if let Some(existing) = host.objects.get(&dom_id) {
            return Rc::clone(existing);
        }
        let obj = match node_kind {
            Some((tag_name, attr_id)) => make_element(
                tag_name,
                attr_id,
                dom_id,
                Rc::clone(&host.element_prototype),
                Rc::clone(&host.element_constructor),
            ),
            None => make_text_node(dom_id),
        };
        host.objects.insert(dom_id, Rc::clone(&obj));
        obj
    })?;

    Some(JSValue::Object(obj))
}

fn expose_node_list(vm: &mut VM, nodes: Vec<NodeRef<HtmlNodeType>>) -> JSValue {
    let values = nodes
        .into_iter()
        .filter_map(|node| expose_node(vm, node))
        .collect();
    JSArray::from_vec(values).to_object()
}

// --- Element ---

fn make_element_interface() -> (Rc<RefCell<JSObject>>, Rc<RefCell<JSObject>>) {
    let mut prototype = JSObject::new();
    prototype.define_property(
        "value".to_string(),
        accessor_property(get_element_value, set_element_value),
    );
    prototype.define_property(
        "checked".to_string(),
        accessor_property(get_element_checked, set_element_checked),
    );
    prototype.define_property(
        "selected".to_string(),
        accessor_property(get_element_selected, set_element_selected),
    );
    prototype.define_property(
        "disabled".to_string(),
        accessor_property(get_element_disabled, set_element_disabled),
    );
    prototype.define_property(
        "multiple".to_string(),
        accessor_property(get_element_multiple, set_element_multiple),
    );
    let prototype = Rc::new(RefCell::new(prototype));
    let mut constructor = JSObject::new();
    constructor.define_property(
        "prototype".to_string(),
        Property::read_only(JSValue::Object(Rc::clone(&prototype))),
    );
    (prototype, Rc::new(RefCell::new(constructor)))
}

fn make_element(
    tag_name: String,
    _attr_id: String,
    dom_id: u64,
    prototype: Rc<RefCell<JSObject>>,
    constructor: Rc<RefCell<JSObject>>,
) -> Rc<RefCell<JSObject>> {
    let mut obj = JSObject::with_prototype(Some(prototype));
    define_node_id(&mut obj, dom_id);
    obj.define_property(
        "constructor".to_string(),
        Property::read_only(JSValue::Object(constructor)),
    );
    let html_name = tag_name.to_ascii_uppercase();
    obj.define_property(
        "nodeType".to_string(),
        Property::read_only(JSValue::Number(1.0)),
    );
    obj.define_property(
        "nodeName".to_string(),
        Property::read_only(JSValue::String(html_name.clone())),
    );
    obj.define_property(
        "tagName".to_string(),
        Property::read_only(JSValue::String(html_name)),
    );
    obj.define_property(
        "id".to_string(),
        accessor_property(get_element_id, set_element_id),
    );
    obj.define_property(
        "textContent".to_string(),
        accessor_property(get_text_content, set_text_content),
    );
    obj.define_property(
        "innerText".to_string(),
        accessor_property(get_inner_text, set_inner_text),
    );
    obj.define_property(
        "innerHTML".to_string(),
        accessor_property(get_inner_html, set_inner_html),
    );
    obj.define_property(
        "parentNode".to_string(),
        read_only_accessor_property(get_parent_node),
    );
    obj.define_property(
        "ownerDocument".to_string(),
        read_only_accessor_property(get_owner_document),
    );
    obj.define_property(
        "namespaceURI".to_string(),
        read_only_accessor_property(get_namespace_uri),
    );
    obj.define_property(
        "childNodes".to_string(),
        read_only_accessor_property(get_child_nodes),
    );
    obj.define_property(
        "firstChild".to_string(),
        read_only_accessor_property(get_first_child),
    );
    obj.define_property(
        "lastChild".to_string(),
        read_only_accessor_property(get_last_child),
    );
    obj.define_property(
        "nextSibling".to_string(),
        read_only_accessor_property(get_next_sibling),
    );
    obj.define_property(
        "previousSibling".to_string(),
        read_only_accessor_property(get_previous_sibling),
    );
    obj.define_property(
        "children".to_string(),
        read_only_accessor_property(get_element_children),
    );
    obj.define_property(
        "classList".to_string(),
        read_only_accessor_property(get_class_list),
    );
    obj.define_property(
        "className".to_string(),
        accessor_property(get_class_name, set_class_name),
    );
    obj.define_property(
        "style".to_string(),
        read_only_accessor_property(get_style),
    );
    obj.set(
        "getAttribute".to_string(),
        JSValue::NativeFunction(get_attribute),
    );
    obj.set(
        "hasAttribute".to_string(),
        JSValue::NativeFunction(has_attribute),
    );
    obj.set(
        "setAttribute".to_string(),
        JSValue::NativeFunction(set_attribute),
    );
    obj.set(
        "setAttributeNS".to_string(),
        JSValue::NativeFunction(set_attribute_ns),
    );
    obj.set(
        "removeAttribute".to_string(),
        JSValue::NativeFunction(remove_attribute),
    );
    obj.set(
        "addEventListener".to_string(),
        JSValue::NativeFunction(add_element_event_listener),
    );
    obj.set(
        "removeEventListener".to_string(),
        JSValue::NativeFunction(remove_element_event_listener),
    );
    obj.set(
        "querySelector".to_string(),
        JSValue::NativeFunction(element_query_selector),
    );
    obj.set(
        "querySelectorAll".to_string(),
        JSValue::NativeFunction(element_query_selector_all),
    );
    obj.set("contains".to_string(), JSValue::NativeFunction(element_contains));
    obj.set("focus".to_string(), JSValue::NativeFunction(focus_element));
    obj.set("blur".to_string(), JSValue::NativeFunction(blur_element));
    obj.set(
        "appendChild".to_string(),
        JSValue::NativeFunction(append_child),
    );
    obj.set(
        "insertBefore".to_string(),
        JSValue::NativeFunction(insert_before),
    );
    obj.set(
        "removeChild".to_string(),
        JSValue::NativeFunction(remove_child),
    );
    obj.set("remove".to_string(), JSValue::NativeFunction(remove_node));
    Rc::new(RefCell::new(obj))
}

fn make_text_node(dom_id: u64) -> Rc<RefCell<JSObject>> {
    let mut obj = JSObject::new();
    define_node_id(&mut obj, dom_id);
    obj.define_property(
        "nodeType".to_string(),
        Property::read_only(JSValue::Number(3.0)),
    );
    obj.define_property(
        "nodeName".to_string(),
        Property::read_only(JSValue::String("#text".to_string())),
    );
    obj.define_property(
        "textContent".to_string(),
        accessor_property(get_text_content, set_text_content),
    );
    obj.define_property(
        "nodeValue".to_string(),
        accessor_property(get_text_content, set_text_content),
    );
    obj.define_property(
        "data".to_string(),
        accessor_property(get_text_content, set_text_content),
    );
    obj.define_property(
        "parentNode".to_string(),
        read_only_accessor_property(get_parent_node),
    );
    obj.define_property(
        "ownerDocument".to_string(),
        read_only_accessor_property(get_owner_document),
    );
    obj.define_property(
        "childNodes".to_string(),
        read_only_accessor_property(get_child_nodes),
    );
    obj.define_property(
        "firstChild".to_string(),
        read_only_accessor_property(get_first_child),
    );
    obj.define_property(
        "lastChild".to_string(),
        read_only_accessor_property(get_last_child),
    );
    obj.define_property(
        "nextSibling".to_string(),
        read_only_accessor_property(get_next_sibling),
    );
    obj.define_property(
        "previousSibling".to_string(),
        read_only_accessor_property(get_previous_sibling),
    );
    obj.set("remove".to_string(), JSValue::NativeFunction(remove_node));
    Rc::new(RefCell::new(obj))
}

fn define_node_id(obj: &mut JSObject, dom_id: u64) {
    obj.define_property(
        "__orinium_dom_id".to_string(),
        Property {
            value: JSValue::Number(dom_id as f64),
            enumerable: false,
            writable: false,
            configurable: false,
            getter: None,
            setter: None,
        },
    );
}

fn append_child(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(parent) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Null);
    };
    if !matches!(parent.borrow().value, HtmlNodeType::Element { .. }) {
        return Ok(JSValue::Null);
    }
    let Some(child_value) = args.get(1).cloned() else {
        return Ok(JSValue::Null);
    };
    let Some(child) = dom_node(vm, &child_value) else {
        return Ok(JSValue::Null);
    };
    if !TreeNode::append_child(&parent, child) {
        return Ok(JSValue::Null);
    }

    if let Some(dom_id) = node_dom_id(&child_value) {
        let _ = with_host_mut(vm, |host| {
            host.detached_nodes.remove(&dom_id);
        });
    }
    mark_dom_dirty(vm);
    Ok(child_value)
}

fn insert_before(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(parent) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Null);
    };
    let Some(child_value) = args.get(1).cloned() else {
        return Ok(JSValue::Null);
    };
    let Some(child) = dom_node(vm, &child_value) else {
        return Ok(JSValue::Null);
    };

    let inserted = match args.get(2) {
        None | Some(JSValue::Null) | Some(JSValue::Undefined) => {
            TreeNode::append_child(&parent, child)
        }
        Some(reference_value) => {
            let Some(reference) = dom_node(vm, reference_value) else {
                return Ok(JSValue::Null);
            };
            TreeNode::insert_before(&parent, child, &reference)
        }
    };
    if !inserted {
        return Ok(JSValue::Null);
    }
    if let Some(dom_id) = node_dom_id(&child_value) {
        let _ = with_host_mut(vm, |host| {
            host.detached_nodes.remove(&dom_id);
        });
    }
    mark_dom_dirty(vm);
    Ok(child_value)
}

fn remove_child(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(parent) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Null);
    };
    let Some(child_value) = args.get(1).cloned() else {
        return Ok(JSValue::Null);
    };
    let Some(child) = dom_node(vm, &child_value) else {
        return Ok(JSValue::Null);
    };
    let Some(detached) = TreeNode::remove_child(&parent, &child) else {
        return Ok(JSValue::Null);
    };
    if let Some(dom_id) = node_dom_id(&child_value) {
        let _ = with_host_mut(vm, |host| {
            host.detached_nodes.insert(dom_id, detached);
        });
    }
    mark_dom_dirty(vm);
    Ok(child_value)
}

fn remove_node(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let this = args.first().unwrap_or(&JSValue::Undefined);
    let Some(node) = dom_node(vm, this) else {
        return Ok(JSValue::Undefined);
    };
    if TreeNode::detach(&node) {
        if let Some(dom_id) = node_dom_id(this) {
            let _ = with_host_mut(vm, |host| {
                host.detached_nodes.insert(dom_id, node);
            });
        }
        mark_dom_dirty(vm);
    }
    Ok(JSValue::Undefined)
}

fn element_query_selector(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(scope) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Null);
    };
    let Some(JSValue::String(selector)) = args.get(1) else {
        return Ok(JSValue::Null);
    };
    let Some(node) = DomTree::query_selector_within(&scope, selector) else {
        return Ok(JSValue::Null);
    };
    Ok(expose_node(vm, node).unwrap_or(JSValue::Null))
}

fn element_query_selector_all(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(scope) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSArray::new().to_object());
    };
    let Some(JSValue::String(selector)) = args.get(1) else {
        return Ok(JSArray::new().to_object());
    };
    let nodes = DomTree::query_selector_all_within(&scope, selector);
    Ok(expose_node_list(vm, nodes))
}

fn element_contains(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(container) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Boolean(false));
    };
    let Some(mut candidate) = args.get(1).and_then(|value| dom_node(vm, value)) else {
        return Ok(JSValue::Boolean(false));
    };

    loop {
        if Rc::ptr_eq(&container, &candidate) {
            return Ok(JSValue::Boolean(true));
        }
        let parent = { candidate.borrow().parent() };
        let Some(parent) = parent else {
            return Ok(JSValue::Boolean(false));
        };
        candidate = parent;
    }
}

fn focus_element(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(dom_id) = node_dom_id(args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Undefined);
    };
    let _ = with_host_mut(vm, |host| {
        host.active_element = Some(dom_id);
    });
    Ok(JSValue::Undefined)
}

fn blur_element(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(dom_id) = node_dom_id(args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Undefined);
    };
    let _ = with_host_mut(vm, |host| {
        if host.active_element == Some(dom_id) {
            host.active_element = None;
        }
    });
    Ok(JSValue::Undefined)
}

fn add_element_event_listener(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(dom_id) = node_dom_id(args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Undefined);
    };
    let Some(JSValue::String(event_type)) = args.get(1) else {
        return Ok(JSValue::Undefined);
    };
    let Some(listener) = args.get(2).filter(|value| is_callable(value)).cloned() else {
        return Ok(JSValue::Undefined);
    };

    let _ = with_host_mut(vm, |host| {
        let listeners = host
            .element_event_listeners
            .entry(dom_id)
            .or_default()
            .entry(event_type.clone())
            .or_default();
        if !listeners
            .iter()
            .any(|candidate| candidate.strict_equals(&listener))
        {
            listeners.push(listener);
        }
    });
    Ok(JSValue::Undefined)
}

fn remove_element_event_listener(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(dom_id) = node_dom_id(args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Undefined);
    };
    let Some(JSValue::String(event_type)) = args.get(1) else {
        return Ok(JSValue::Undefined);
    };
    let Some(listener) = args.get(2) else {
        return Ok(JSValue::Undefined);
    };
    let _ = with_host_mut(vm, |host| {
        if let Some(listeners) = host
            .element_event_listeners
            .get_mut(&dom_id)
            .and_then(|events| events.get_mut(event_type))
        {
            listeners.retain(|candidate| !candidate.strict_equals(listener));
        }
    });
    Ok(JSValue::Undefined)
}

fn accessor_property(
    getter: pixi_byte::NativeFunctionType,
    setter: pixi_byte::NativeFunctionType,
) -> Property {
    Property {
        value: JSValue::Undefined,
        enumerable: true,
        writable: false,
        configurable: false,
        getter: Some(JSValue::NativeFunction(getter)),
        setter: Some(JSValue::NativeFunction(setter)),
    }
}

fn read_only_accessor_property(getter: pixi_byte::NativeFunctionType) -> Property {
    Property {
        value: JSValue::Undefined,
        enumerable: true,
        writable: false,
        configurable: false,
        getter: Some(JSValue::NativeFunction(getter)),
        setter: None,
    }
}

fn get_parent_node(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Null);
    };
    let Some(parent) = node.borrow().parent() else {
        return Ok(JSValue::Null);
    };
    Ok(expose_node(vm, parent).unwrap_or(JSValue::Null))
}

fn get_owner_document(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(with_host(vm, |host| host.document.as_ref().cloned())
        .flatten()
        .map(JSValue::Object)
        .unwrap_or(JSValue::Null))
}

fn get_namespace_uri(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let this = args.first().unwrap_or(&JSValue::Undefined);
    if let Some(dom_id) = node_dom_id(this) {
        if let Some(namespace) = with_host(vm, |host| host.namespaces.get(&dom_id).cloned()).flatten()
        {
            return if namespace.is_empty() {
                Ok(JSValue::Null)
            } else {
                Ok(JSValue::String(namespace))
            };
        }
    }

    let Some(mut node) = dom_node(vm, this) else {
        return Ok(JSValue::Null);
    };
    loop {
        let (tag_name, parent) = {
            let node = node.borrow();
            (
                node.value.tag_name().map(str::to_ascii_lowercase),
                node.parent(),
            )
        };
        match tag_name.as_deref() {
            Some("svg") => return Ok(JSValue::String(SVG_NAMESPACE.to_string())),
            Some("math") => return Ok(JSValue::String(MATHML_NAMESPACE.to_string())),
            _ => {}
        }
        let Some(parent) = parent else {
            return Ok(JSValue::String(HTML_NAMESPACE.to_string()));
        };
        node = parent;
    }
}

fn get_child_nodes(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSArray::new().to_object());
    };
    let children = node.borrow().children().to_vec();
    Ok(expose_node_list(vm, children))
}

fn get_first_child(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    get_edge_child(vm, &args, true)
}

fn get_last_child(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    get_edge_child(vm, &args, false)
}

fn get_edge_child(vm: &mut VM, args: &[JSValue], first: bool) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Null);
    };
    let child = if first {
        node.borrow().children().first().cloned()
    } else {
        node.borrow().children().last().cloned()
    };
    Ok(child
        .and_then(|child| expose_node(vm, child))
        .unwrap_or(JSValue::Null))
}

fn get_next_sibling(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    get_sibling(vm, &args, 1)
}

fn get_previous_sibling(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    get_sibling(vm, &args, -1)
}

fn get_sibling(vm: &mut VM, args: &[JSValue], offset: isize) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Null);
    };
    let Some(parent) = node.borrow().parent() else {
        return Ok(JSValue::Null);
    };
    let sibling = {
        let parent = parent.borrow();
        let Some(index) = parent
            .children()
            .iter()
            .position(|candidate| Rc::ptr_eq(candidate, &node))
        else {
            return Ok(JSValue::Null);
        };
        let sibling_index = index as isize + offset;
        (sibling_index >= 0)
            .then(|| parent.children().get(sibling_index as usize).cloned())
            .flatten()
    };
    Ok(sibling
        .and_then(|sibling| expose_node(vm, sibling))
        .unwrap_or(JSValue::Null))
}

fn get_element_children(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSArray::new().to_object());
    };
    let children = node
        .borrow()
        .children()
        .iter()
        .filter(|child| matches!(child.borrow().value, HtmlNodeType::Element { .. }))
        .cloned()
        .collect();
    Ok(expose_node_list(vm, children))
}

fn get_class_list(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(dom_id) = node_dom_id(args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Null);
    };
    let mut class_list = JSObject::new();
    define_node_id(&mut class_list, dom_id);
    class_list.set(
        "contains".to_string(),
        JSValue::NativeFunction(class_list_contains),
    );
    class_list.set("add".to_string(), JSValue::NativeFunction(class_list_add));
    class_list.set(
        "remove".to_string(),
        JSValue::NativeFunction(class_list_remove),
    );
    class_list.set(
        "toggle".to_string(),
        JSValue::NativeFunction(class_list_toggle),
    );
    Ok(JSValue::Object(Rc::new(RefCell::new(class_list))))
}

fn get_class_name(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::String(String::new()));
    };
    let value = node
        .borrow()
        .value
        .get_attr("class")
        .unwrap_or("")
        .to_string();
    Ok(JSValue::String(value))
}

fn get_element_id(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::String(String::new()));
    };
    let id = node.borrow().value.get_attr("id").unwrap_or("").to_string();
    Ok(JSValue::String(id))
}

fn set_element_id(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Undefined);
    };
    let id = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    node.borrow_mut().value.set_attr("id", id);
    mark_dom_dirty(vm);
    Ok(JSValue::Undefined)
}

fn set_class_name(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Undefined);
    };
    let value = args.get(1).map(JSValue::to_string).unwrap_or_default();
    node.borrow_mut().value.set_attr("class", value);
    mark_dom_dirty(vm);
    Ok(JSValue::Undefined)
}

fn get_style(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(dom_id) = node_dom_id(args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Null);
    };
    let style = with_host_mut(vm, |host| {
        if let Some(style) = host.styles.get(&dom_id) {
            return Rc::clone(style);
        }

        let style = make_style_declaration(dom_id);
        host.styles.insert(dom_id, Rc::clone(&style));
        style
    })
    .ok_or_else(|| JSError::InternalError("JS host is unavailable".to_string()))?;
    Ok(JSValue::Object(style))
}

fn make_style_declaration(dom_id: u64) -> Rc<RefCell<JSObject>> {
    let mut style = JSObject::new();
    define_node_id(&mut style, dom_id);
    style.define_property(
        "cssText".to_string(),
        accessor_property(get_style_css_text, set_style_css_text),
    );
    style.set(
        "setProperty".to_string(),
        JSValue::NativeFunction(style_set_property),
    );
    style.set(
        "getPropertyValue".to_string(),
        JSValue::NativeFunction(style_get_property_value),
    );
    style.set(
        "removeProperty".to_string(),
        JSValue::NativeFunction(style_remove_property),
    );
    style.set(
        "__host_get_property__".to_string(),
        JSValue::NativeFunction(style_host_get_property),
    );
    style.set(
        "__host_set_property__".to_string(),
        JSValue::NativeFunction(style_host_set_property),
    );
    Rc::new(RefCell::new(style))
}

fn get_style_css_text(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::String(String::new()));
    };
    let css_text = node
        .borrow()
        .value
        .get_attr("style")
        .unwrap_or("")
        .to_string();
    Ok(JSValue::String(css_text))
}

fn set_style_css_text(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Undefined);
    };
    let css_text = args.get(1).map(JSValue::to_string).unwrap_or_default();
    if css_text.is_empty() {
        node.borrow_mut().value.remove_attr("style");
    } else {
        node.borrow_mut().value.set_attr("style", css_text);
    }
    mark_dom_dirty(vm);
    Ok(JSValue::Undefined)
}

fn style_set_property(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(name) = args.get(1).map(JSValue::to_string) else {
        return Ok(JSValue::Undefined);
    };
    let value = args.get(2).map(JSValue::to_string).unwrap_or_default();
    let priority = args.get(3).map(JSValue::to_string).unwrap_or_default();
    set_style_property(vm, &args, &name, &value, &priority)?;
    Ok(JSValue::Undefined)
}

fn style_get_property_value(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(name) = args.get(1).map(JSValue::to_string) else {
        return Ok(JSValue::String(String::new()));
    };
    Ok(JSValue::String(read_style_property(vm, &args, &name)))
}

fn style_remove_property(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(name) = args.get(1).map(JSValue::to_string) else {
        return Ok(JSValue::String(String::new()));
    };
    let previous = read_style_property(vm, &args, &name);
    set_style_property(vm, &args, &name, "", "")?;
    Ok(JSValue::String(previous))
}

fn style_host_get_property(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(name) = args.get(1).map(JSValue::to_string) else {
        return Ok(JSValue::Undefined);
    };
    Ok(JSValue::String(read_style_property(
        vm,
        &args,
        &style_property_name(&name),
    )))
}

fn style_host_set_property(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(name) = args.get(1).map(JSValue::to_string) else {
        return Ok(JSValue::Undefined);
    };
    let value = args.get(2).map(JSValue::to_string).unwrap_or_default();
    set_style_property(vm, &args, &style_property_name(&name), &value, "")?;
    Ok(JSValue::Undefined)
}

fn read_style_property(vm: &mut VM, args: &[JSValue], name: &str) -> String {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return String::new();
    };
    let style = node
        .borrow()
        .value
        .get_attr("style")
        .unwrap_or("")
        .to_string();
    parse_style_declarations(&style)
        .into_iter()
        .rev()
        .find(|(property, _)| property.eq_ignore_ascii_case(name))
        .map(|(_, value)| strip_important(&value).to_string())
        .unwrap_or_default()
}

fn set_style_property(
    vm: &mut VM,
    args: &[JSValue],
    name: &str,
    value: &str,
    priority: &str,
) -> JSResult<()> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(());
    };
    let style = node
        .borrow()
        .value
        .get_attr("style")
        .unwrap_or("")
        .to_string();
    let mut declarations = parse_style_declarations(&style);
    declarations.retain(|(property, _)| !property.eq_ignore_ascii_case(name));
    if !value.is_empty() {
        let value = if priority.eq_ignore_ascii_case("important") {
            format!("{} !important", value.trim())
        } else {
            value.trim().to_string()
        };
        declarations.push((name.to_string(), value));
    }

    let css_text = serialize_style_declarations(&declarations);
    if css_text.is_empty() {
        node.borrow_mut().value.remove_attr("style");
    } else {
        node.borrow_mut().value.set_attr("style", css_text);
    }
    mark_dom_dirty(vm);
    Ok(())
}

fn parse_style_declarations(css_text: &str) -> Vec<(String, String)> {
    css_text
        .split(';')
        .filter_map(|declaration| {
            let (name, value) = declaration.split_once(':')?;
            let name = name.trim();
            (!name.is_empty()).then(|| (name.to_string(), value.trim().to_string()))
        })
        .collect()
}

fn serialize_style_declarations(declarations: &[(String, String)]) -> String {
    declarations
        .iter()
        .map(|(name, value)| format!("{name}: {value};"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn strip_important(value: &str) -> &str {
    value
        .strip_suffix("!important")
        .map(str::trim_end)
        .unwrap_or(value)
}

fn style_property_name(name: &str) -> String {
    if name.starts_with("--") {
        return name.to_string();
    }
    if name == "cssFloat" {
        return "float".to_string();
    }

    let mut result = String::new();
    if name.starts_with("ms") && name.chars().nth(2).is_some_and(char::is_uppercase) {
        result.push('-');
    }
    for character in name.chars() {
        if character.is_uppercase() {
            result.push('-');
            result.extend(character.to_lowercase());
        } else {
            result.push(character);
        }
    }
    result
}

fn class_list_contains(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Boolean(false));
    };
    let Some(token) = class_token(args.get(1)) else {
        return Ok(JSValue::Boolean(false));
    };
    Ok(JSValue::Boolean(
        class_tokens(&node).iter().any(|class| class == token),
    ))
}

fn class_list_add(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Undefined);
    };
    let mut classes = class_tokens(&node);
    let mut changed = false;
    for value in args.iter().skip(1) {
        let Some(token) = class_token(Some(value)) else {
            continue;
        };
        if !classes.iter().any(|class| class == token) {
            classes.push(token.to_string());
            changed = true;
        }
    }
    if changed {
        set_class_tokens(&node, &classes);
        mark_dom_dirty(vm);
    }
    Ok(JSValue::Undefined)
}

fn class_list_remove(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Undefined);
    };
    let removals: Vec<&str> = args
        .iter()
        .skip(1)
        .filter_map(|value| class_token(Some(value)))
        .collect();
    let mut classes = class_tokens(&node);
    let old_len = classes.len();
    classes.retain(|class| !removals.iter().any(|removal| class == removal));
    if classes.len() != old_len {
        set_class_tokens(&node, &classes);
        mark_dom_dirty(vm);
    }
    Ok(JSValue::Undefined)
}

fn class_list_toggle(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Boolean(false));
    };
    let Some(token) = class_token(args.get(1)) else {
        return Ok(JSValue::Boolean(false));
    };
    let mut classes = class_tokens(&node);
    let position = classes.iter().position(|class| class == token);
    let should_have = args
        .get(2)
        .map(JSValue::to_boolean)
        .unwrap_or(position.is_none());

    let changed = match (position, should_have) {
        (Some(position), false) => {
            classes.remove(position);
            true
        }
        (None, true) => {
            classes.push(token.to_string());
            true
        }
        _ => false,
    };
    if changed {
        set_class_tokens(&node, &classes);
        mark_dom_dirty(vm);
    }
    Ok(JSValue::Boolean(should_have))
}

fn class_token(value: Option<&JSValue>) -> Option<&str> {
    let JSValue::String(token) = value? else {
        return None;
    };
    (!token.is_empty() && !token.chars().any(char::is_whitespace)).then_some(token)
}

fn class_tokens(node: &NodeRef<HtmlNodeType>) -> Vec<String> {
    node.borrow()
        .value
        .get_attr("class")
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

fn set_class_tokens(node: &NodeRef<HtmlNodeType>, classes: &[String]) {
    node.borrow_mut().value.set_attr("class", classes.join(" "));
}

fn get_text_content(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Null);
    };
    Ok(JSValue::String(DomTree::inner_text(&node)))
}

fn set_text_content(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Undefined);
    };
    let new_text = args
        .get(1)
        .map(|v| v.to_console_string())
        .unwrap_or_default();
    DomTree::set_text_content(&node, &new_text);
    mark_dom_dirty(vm);
    Ok(JSValue::Undefined)
}

fn get_inner_text(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    get_text_content(vm, args)
}

fn set_inner_text(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    set_text_content(vm, args)
}

fn escape_html_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_html_attribute(value: &str) -> String {
    escape_html_text(value).replace('"', "&quot;")
}

fn is_void_html_element(tag_name: &str) -> bool {
    matches!(
        tag_name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn serialize_html_node(node: &NodeRef<HtmlNodeType>) -> String {
    let (value, children) = {
        let node = node.borrow();
        (node.value.clone(), node.children().to_vec())
    };
    match value {
        HtmlNodeType::Text(text) => escape_html_text(&text),
        HtmlNodeType::Comment(comment) => format!("<!--{comment}-->"),
        HtmlNodeType::Element {
            tag_name,
            attributes,
        } => {
            let mut html = format!("<{tag_name}");
            for attribute in attributes {
                html.push(' ');
                html.push_str(&attribute.name);
                html.push_str("=\"");
                html.push_str(&escape_html_attribute(&attribute.value));
                html.push('"');
            }
            html.push('>');
            if !is_void_html_element(&tag_name) {
                for child in children {
                    html.push_str(&serialize_html_node(&child));
                }
                html.push_str("</");
                html.push_str(&tag_name);
                html.push('>');
            }
            html
        }
        _ => String::new(),
    }
}

fn get_inner_html(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Null);
    };
    let children = node.borrow().children().to_vec();
    Ok(JSValue::String(
        children
            .iter()
            .map(serialize_html_node)
            .collect::<String>(),
    ))
}

fn set_inner_html(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Undefined);
    };
    if !matches!(node.borrow().value, HtmlNodeType::Element { .. }) {
        return Ok(JSValue::Undefined);
    }
    let html = args.get(1).map(JSValue::to_string).unwrap_or_default();
    let old_children = node.borrow().children().to_vec();
    let _ = with_host_mut(vm, |host| {
        for child in &old_children {
            if let Some(dom_id) = host.dom_id_for_node(child) {
                host.detached_nodes.insert(dom_id, Rc::clone(child));
            }
        }
    });
    node.borrow_mut().clear_children();

    let mut parser = HtmlParser::new(&html);
    let fragment = parser.parse();
    if let Some(body) = fragment.get_elements_by_tag_name("body").into_iter().next() {
        let children = body.borrow().children().to_vec();
        for child in children {
            TreeNode::append_child(&node, child);
        }
    }
    mark_dom_dirty(vm);
    Ok(JSValue::Undefined)
}

fn reflected_string_property(
    vm: &mut VM,
    args: &[JSValue],
    name: &str,
) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::String(String::new()));
    };
    let value = node
        .borrow()
        .value
        .get_attr(name)
        .unwrap_or("")
        .to_string();
    Ok(JSValue::String(value))
}

fn set_reflected_string_property(
    vm: &mut VM,
    args: &[JSValue],
    name: &str,
) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Undefined);
    };
    let value = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    node.borrow_mut().value.set_attr(name, value);
    mark_dom_dirty(vm);
    Ok(JSValue::Undefined)
}

fn reflected_boolean_property(
    vm: &mut VM,
    args: &[JSValue],
    name: &str,
) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Boolean(false));
    };
    Ok(JSValue::Boolean(node.borrow().value.get_attr(name).is_some()))
}

fn set_reflected_boolean_property(
    vm: &mut VM,
    args: &[JSValue],
    name: &str,
) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Undefined);
    };
    let enabled = args.get(1).map(JSValue::to_boolean).unwrap_or(false);
    if enabled {
        node.borrow_mut().value.set_attr(name, String::new());
        mark_dom_dirty(vm);
    } else if node.borrow_mut().value.remove_attr(name).is_some() {
        mark_dom_dirty(vm);
    }
    Ok(JSValue::Undefined)
}

fn get_element_value(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    reflected_string_property(vm, &args, "value")
}

fn set_element_value(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    set_reflected_string_property(vm, &args, "value")
}

macro_rules! reflected_boolean_accessors {
    ($getter:ident, $setter:ident, $name:literal) => {
        fn $getter(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
            reflected_boolean_property(vm, &args, $name)
        }

        fn $setter(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
            set_reflected_boolean_property(vm, &args, $name)
        }
    };
}

reflected_boolean_accessors!(get_element_checked, set_element_checked, "checked");
reflected_boolean_accessors!(get_element_selected, set_element_selected, "selected");
reflected_boolean_accessors!(get_element_disabled, set_element_disabled, "disabled");
reflected_boolean_accessors!(get_element_multiple, set_element_multiple, "multiple");

fn get_attribute(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Null);
    };
    let Some(JSValue::String(name)) = args.get(1) else {
        return Ok(JSValue::Undefined);
    };
    match node.borrow().value.get_attr(name) {
        Some(value) => Ok(JSValue::String(value.to_string())),
        None => Ok(JSValue::Null),
    }
}

fn has_attribute(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Boolean(false));
    };
    let Some(JSValue::String(name)) = args.get(1) else {
        return Ok(JSValue::Boolean(false));
    };
    Ok(JSValue::Boolean(node.borrow().value.get_attr(name).is_some()))
}

fn set_attribute(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Undefined);
    };
    let Some(JSValue::String(name)) = args.get(1) else {
        return Ok(JSValue::Undefined);
    };
    let value = args
        .get(2)
        .map(|v| v.to_console_string())
        .unwrap_or_default();
    node.borrow_mut().value.set_attr(name, value);
    mark_dom_dirty(vm);
    Ok(JSValue::Undefined)
}

fn set_attribute_ns(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let forwarded = vec![
        args.first().cloned().unwrap_or(JSValue::Undefined),
        args.get(2).cloned().unwrap_or(JSValue::Undefined),
        args.get(3).cloned().unwrap_or(JSValue::Undefined),
    ];
    set_attribute(vm, forwarded)
}

fn remove_attribute(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Undefined);
    };
    let Some(JSValue::String(name)) = args.get(1) else {
        return Ok(JSValue::Undefined);
    };
    if node.borrow_mut().value.remove_attr(name).is_some() {
        mark_dom_dirty(vm);
    }
    Ok(JSValue::Undefined)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_from_html(html: &str) -> (JsRuntime, Rc<DomTree>) {
        let mut parser = HtmlParser::new(html);
        let dom = Rc::new(parser.parse());
        let runtime = JsRuntime::new(Rc::clone(&dom));
        (runtime, dom)
    }

    #[test]
    fn set_text_content_mutates_dom_and_marks_dirty() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="hello">before</div>"#);
        runtime.run_script(
            r#"const el = document.getElementById("hello"); el.textContent = "hello from js";"#,
        );
        assert!(runtime.needs_redraw());
        assert!(runtime.take_needs_redraw());

        let node = dom.get_element_by_id("hello").unwrap();
        assert_eq!(DomTree::inner_text(&node), "hello from js");
    }

    #[test]
    fn set_attribute_mutates_dom() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="hello"></div>"#);
        runtime.run_script(
            r#"const el = document.getElementById("hello"); el.setAttribute("data-run", "1");"#,
        );

        let node = dom.get_element_by_id("hello").unwrap();
        assert_eq!(node.borrow().value.get_attr("data-run"), Some("1"));
    }

    #[test]
    fn element_id_is_a_live_reflected_property() {
        let (mut runtime, dom) = runtime_from_html(r#"<main id="root"></main>"#);
        runtime.run_script(
            r#"
            const child = document.createElement("button");
            child.id = "first";
            document.getElementById("root").appendChild(child);
            child.setAttribute("id", "second");
            child.setAttribute("data-current-id", child.id);
            "#,
        );

        let child = dom.get_element_by_id("second").unwrap();
        assert_eq!(child.borrow().value.get_attr("data-current-id"), Some("second"));
        assert!(runtime.needs_redraw());
    }

    #[test]
    fn form_properties_reflect_to_dom_attributes() {
        let (mut runtime, dom) = runtime_from_html(
            r#"<input id="field"><option id="option"></option><select id="select"></select>"#,
        );
        runtime.run_script(
            r#"
            const field = document.getElementById("field");
            field.value = "hello";
            field.checked = true;
            field.disabled = true;
            field.checked = false;
            const option = document.getElementById("option");
            option.selected = true;
            const select = document.getElementById("select");
            select.multiple = true;
            "#,
        );

        let field = dom.get_element_by_id("field").unwrap();
        let field = field.borrow();
        assert_eq!(field.value.get_attr("value"), Some("hello"));
        assert_eq!(field.value.get_attr("checked"), None);
        assert_eq!(field.value.get_attr("disabled"), Some(""));
        drop(field);
        let option = dom.get_element_by_id("option").unwrap();
        assert_eq!(option.borrow().value.get_attr("selected"), Some(""));
        let select = dom.get_element_by_id("select").unwrap();
        assert_eq!(select.borrow().value.get_attr("multiple"), Some(""));
        assert!(runtime.needs_redraw());
    }

    #[test]
    fn form_properties_are_accessors_on_the_element_prototype() {
        let (mut runtime, dom) = runtime_from_html(r#"<input id="field">"#);
        runtime.run_script(
            r#"
            const field = document.getElementById("field");
            const prototype = field.constructor.prototype;
            const descriptor = Object.getOwnPropertyDescriptor(prototype, "value");
            field.setAttribute("data-prototype", Object.getPrototypeOf(field) === prototype);
            field.setAttribute("data-interface", field.constructor === HTMLElement && field instanceof Element);
            field.setAttribute("data-own-value", field.hasOwnProperty("value"));
            field.setAttribute("data-accessor", typeof descriptor.get + ":" + typeof descriptor.set);
            descriptor.set.call(field, "tracked");
            field.setAttribute("data-read", descriptor.get.call(field));
            "#,
        );

        let field = dom.get_element_by_id("field").unwrap();
        let field = field.borrow();
        assert_eq!(field.value.get_attr("data-prototype"), Some("true"));
        assert_eq!(field.value.get_attr("data-interface"), Some("true"));
        assert_eq!(field.value.get_attr("data-own-value"), Some("false"));
        assert_eq!(field.value.get_attr("data-accessor"), Some("function:function"));
        assert_eq!(field.value.get_attr("data-read"), Some("tracked"));
        assert_eq!(field.value.get_attr("value"), Some("tracked"));
    }

    #[test]
    fn style_declaration_mutates_inline_style() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="target"></div>"#);
        runtime.run_script(
            r#"
            const target = document.getElementById("target");
            target.style.backgroundColor = "red";
            target.style.setProperty("--accent", "blue");
            target.style.marginTop = "4px";
            target.style.removeProperty("background-color");
            "#,
        );

        let node = dom.get_element_by_id("target").unwrap();
        assert_eq!(
            node.borrow().value.get_attr("style"),
            Some("--accent: blue; margin-top: 4px;")
        );
        assert!(runtime.needs_redraw());
    }

    #[test]
    fn inner_html_parses_replaces_and_serializes_children() {
        let (mut runtime, dom) = runtime_from_html(
            r#"<div id="target"><em id="old">old</em></div><div id="result"></div>"#,
        );
        runtime.run_script(
            r#"
            const target = document.getElementById("target");
            const old = document.getElementById("old");
            target.innerHTML = '<span id="child" data-label="a&b">hello</span><br>';
            old.setAttribute("data-detached", "yes");
            document.getElementById("result").setAttribute("data-html", target.innerHTML);
            "#,
        );

        let target = dom.get_element_by_id("target").unwrap();
        assert_eq!(target.borrow().children().len(), 2);
        assert!(dom.get_element_by_id("old").is_none());
        assert_eq!(
            dom.get_element_by_id("result")
                .unwrap()
                .borrow()
                .value
                .get_attr("data-html"),
            Some("<span id=\"child\" data-label=\"a&amp;b\">hello</span><br>")
        );
        assert!(runtime.needs_redraw());
    }

    #[test]
    fn style_property_names_follow_cssom_spelling() {
        assert_eq!(style_property_name("backgroundColor"), "background-color");
        assert_eq!(style_property_name("msTransition"), "-ms-transition");
        assert_eq!(style_property_name("WebkitTransform"), "-webkit-transform");
        assert_eq!(style_property_name("cssFloat"), "float");
        assert_eq!(style_property_name("--accent"), "--accent");
    }

    #[test]
    fn get_attribute_reads_dom() {
        let (mut runtime, _dom) = runtime_from_html(r#"<div id="hello" data-x="v"></div>"#);
        runtime.run_script(
            r#"const el = document.getElementById("hello"); if (el.getAttribute("data-x") !== "v") { throw new Error("mismatch"); }"#,
        );
    }

    #[test]
    fn missing_id_returns_null() {
        let (mut runtime, _dom) = runtime_from_html(r#"<div id="hello"></div>"#);
        runtime.run_script(
            r#"const el = document.getElementById("missing"); if (el !== null) { throw new Error("expected null"); }"#,
        );
    }

    #[test]
    fn console_log_does_not_panic() {
        let (mut runtime, _dom) = runtime_from_html(r#"<html></html>"#);
        runtime.run_script(
            r#"console.log("a", 1, undefined); console.warn("w"); console.error("e");"#,
        );
    }

    #[test]
    fn syntax_error_is_logged_not_panicked() {
        let (mut runtime, _dom) = runtime_from_html(r#"<html></html>"#);
        runtime.run_script("this is not valid js ((");
    }

    #[test]
    fn accessor_reads_text_content() {
        let (mut runtime, _dom) = runtime_from_html(r#"<div id="hello">hi</div>"#);
        runtime.run_script(
            r#"const el = document.getElementById("hello"); if (el.textContent !== "hi") { throw new Error("mismatch"); }"#,
        );
    }

    #[test]
    fn click_invokes_onclick_and_mutates_dom() {
        let (mut runtime, dom) =
            runtime_from_html(r#"<button id="b">click me</button><p id="result">not clicked</p>"#);
        runtime.run_script(
            r#"
            const button = document.getElementById("b");
            const result = document.getElementById("result");
            button.onclick = function () {
                result.textContent = "clicked!";
                result.setAttribute("data-clicked", "true");
            };
            "#,
        );

        let button = dom.get_element_by_id("b").unwrap();
        assert!(runtime.click(&button));
        assert!(runtime.needs_redraw());
        assert!(runtime.take_needs_redraw());

        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(DomTree::inner_text(&result), "clicked!");
        assert_eq!(result.borrow().value.get_attr("data-clicked"), Some("true"));
    }

    #[test]
    fn click_without_handler_is_noop() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="x"></div>"#);
        runtime.run_script(r#"document.getElementById("x");"#);
        let node = dom.get_element_by_id("x").unwrap();
        assert!(!runtime.click(&node));
        assert!(!runtime.needs_redraw());
    }

    #[test]
    fn click_invokes_element_event_listeners_in_registration_order() {
        let (mut runtime, dom) =
            runtime_from_html(r#"<button id="button">click</button><div id="result"></div>"#);
        runtime.run_script(
            r#"
            const button = document.getElementById("button");
            const result = document.getElementById("result");
            let order = "";
            button.addEventListener("click", function (event) {
                order = order + "a";
                result.setAttribute("data-event-type", event.type);
            });
            button.addEventListener("click", function () {
                order = order + "b";
                result.setAttribute("data-order", order);
            });
            "#,
        );

        let button = dom.get_element_by_id("button").unwrap();
        assert!(runtime.click(&button));

        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(result.borrow().value.get_attr("data-order"), Some("ab"));
        assert_eq!(
            result.borrow().value.get_attr("data-event-type"),
            Some("click")
        );
        assert!(runtime.needs_redraw());
    }

    #[test]
    fn event_listeners_are_deduplicated_and_removable() {
        let (mut runtime, dom) =
            runtime_from_html(r#"<button id="button">click</button><div id="result"></div>"#);
        runtime.run_script(
            r#"
            const button = document.getElementById("button");
            const result = document.getElementById("result");
            function listener() {
                const count = result.getAttribute("data-count");
                result.setAttribute("data-count", count === null ? 1 : Number(count) + 1);
            }
            button.addEventListener("click", listener);
            button.addEventListener("click", listener);
            "#,
        );

        let button = dom.get_element_by_id("button").unwrap();
        assert!(runtime.click(&button));
        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(result.borrow().value.get_attr("data-count"), Some("1"));

        runtime.run_script(
            r#"
            button.removeEventListener("click", listener);
            window.addEventListener("test", listener);
            window.removeEventListener("test", listener);
            "#,
        );
        assert!(!runtime.click(&button));
        assert_eq!(result.borrow().value.get_attr("data-count"), Some("1"));
    }

    #[test]
    fn document_event_listeners_can_be_removed() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r#"
            const result = document.getElementById("result");
            function listener() { result.setAttribute("data-ran", "yes"); }
            document.addEventListener("DOMContentLoaded", listener);
            document.removeEventListener("DOMContentLoaded", listener);
            "#,
        );

        assert!(runtime.dispatch_dom_content_loaded());
        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(result.borrow().value.get_attr("data-ran"), None);
    }

    #[test]
    fn click_bubbles_to_delegated_ancestor_listeners() {
        let (mut runtime, dom) = runtime_from_html(
            r#"<main id="root"><button id="button">click</button></main><div id="result"></div>"#,
        );
        runtime.run_script(
            r#"
            const root = document.getElementById("root");
            const result = document.getElementById("result");
            root.addEventListener("click", function (event) {
                result.setAttribute("data-target", event.target.id);
                result.setAttribute("data-current", event.currentTarget.id);
                result.setAttribute("data-this", this.id);
                event.preventDefault();
                result.setAttribute("data-prevented", event.defaultPrevented);
            });
            "#,
        );

        let button = dom.get_element_by_id("button").unwrap();
        assert!(runtime.click(&button));
        let result = dom.get_element_by_id("result").unwrap();
        let result = result.borrow();
        assert_eq!(result.value.get_attr("data-target"), Some("button"));
        assert_eq!(result.value.get_attr("data-current"), Some("root"));
        assert_eq!(result.value.get_attr("data-this"), Some("root"));
        assert_eq!(result.value.get_attr("data-prevented"), Some("true"));
    }

    #[test]
    fn click_propagation_can_be_stopped() {
        let (mut runtime, dom) = runtime_from_html(
            r#"<main id="root"><button id="button">click</button></main><div id="result"></div>"#,
        );
        runtime.run_script(
            r#"
            const root = document.getElementById("root");
            const button = document.getElementById("button");
            const result = document.getElementById("result");
            button.addEventListener("click", function (event) {
                result.setAttribute("data-child", "ran");
                event.stopPropagation();
            });
            root.addEventListener("click", function () {
                result.setAttribute("data-root", "ran");
            });
            "#,
        );

        let button = dom.get_element_by_id("button").unwrap();
        assert!(runtime.click(&button));
        let result = dom.get_element_by_id("result").unwrap();
        let result = result.borrow();
        assert_eq!(result.value.get_attr("data-child"), Some("ran"));
        assert_eq!(result.value.get_attr("data-root"), None);
    }

    #[test]
    fn click_does_not_invoke_other_event_types() {
        let (mut runtime, dom) = runtime_from_html(r#"<button id="button">click</button>"#);
        runtime.run_script(
            r#"
            const button = document.getElementById("button");
            button.addEventListener("mouseover", function () {
                button.setAttribute("data-ran", "yes");
            });
            "#,
        );

        let button = dom.get_element_by_id("button").unwrap();
        assert!(!runtime.click(&button));
        assert_eq!(button.borrow().value.get_attr("data-ran"), None);
    }

    #[test]
    fn get_element_by_id_reuses_the_same_object() {
        let (mut runtime, _dom) = runtime_from_html(r#"<div id="x"></div>"#);
        runtime.run_script(
            r#"
            const a = document.getElementById("x");
            const b = document.getElementById("x");
            a.onclick = function () {};
            if (a !== b) { throw new Error("expected the same object"); }
            "#,
        );
    }

    #[test]
    fn dom_content_loaded_listener_runs_when_dispatched() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r#"
            document.addEventListener("DOMContentLoaded", function (event) {
                const result = document.getElementById("result");
                result.setAttribute("data-ready", "yes");
                result.setAttribute("data-event-type", event.type);
            });
            "#,
        );

        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(result.borrow().value.get_attr("data-ready"), None);
        assert!(runtime.dispatch_dom_content_loaded());
        assert_eq!(result.borrow().value.get_attr("data-ready"), Some("yes"));
        assert_eq!(
            result.borrow().value.get_attr("data-event-type"),
            Some("DOMContentLoaded")
        );
        assert!(runtime.needs_redraw());
    }

    #[test]
    fn dom_content_loaded_is_dispatched_only_once() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r#"
            let dispatchCount = 0;
            document.addEventListener("DOMContentLoaded", function () {
                dispatchCount = dispatchCount + 1;
                document.getElementById("result").setAttribute("data-count", dispatchCount);
            });
            "#,
        );

        assert!(runtime.dispatch_dom_content_loaded());
        assert!(!runtime.dispatch_dom_content_loaded());
        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(result.borrow().value.get_attr("data-count"), Some("1"));
    }

    #[test]
    fn document_query_selector_and_query_selector_all_expose_elements() {
        let (mut runtime, dom) = runtime_from_html(
            r#"
            <div id="result"></div>
            <main><p class="item">first</p><p class="item featured">second</p></main>
            "#,
        );
        runtime.run_script(
            r#"
            const featured = document.querySelector("main > p.featured");
            featured.setAttribute("data-selected", "yes");
            const items = document.querySelectorAll("p.item");
            items[0].setAttribute("data-first", "yes");
            document.getElementById("result").setAttribute("data-count", items.length);
            "#,
        );

        let items = dom.get_elements_by_class_name("item");
        assert_eq!(items[0].borrow().value.get_attr("data-first"), Some("yes"));
        assert_eq!(
            items[1].borrow().value.get_attr("data-selected"),
            Some("yes")
        );
        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(result.borrow().value.get_attr("data-count"), Some("2"));
    }

    #[test]
    fn element_query_selectors_are_scoped_to_descendants() {
        let (mut runtime, dom) = runtime_from_html(
            r#"
            <section id="scope"><span class="item">one</span><span class="item">two</span></section>
            <span class="item" id="outside">outside</span>
            "#,
        );
        runtime.run_script(
            r##"
            const scope = document.querySelector("#scope");
            scope.querySelector(".item").setAttribute("data-first", "yes");
            const items = scope.querySelectorAll(".item");
            items[1].setAttribute("data-second", "yes");
            scope.setAttribute("data-count", items.length);
            "##,
        );

        let scope = dom.get_element_by_id("scope").unwrap();
        assert_eq!(scope.borrow().value.get_attr("data-count"), Some("2"));
        let items = DomTree::query_selector_all_within(&scope, ".item");
        assert_eq!(items[0].borrow().value.get_attr("data-first"), Some("yes"));
        assert_eq!(items[1].borrow().value.get_attr("data-second"), Some("yes"));
        let outside = dom.get_element_by_id("outside").unwrap();
        assert_eq!(outside.borrow().value.get_attr("data-first"), None);
        assert_eq!(outside.borrow().value.get_attr("data-second"), None);
    }

    #[test]
    fn create_and_append_element_and_text_nodes() {
        let (mut runtime, dom) = runtime_from_html(r#"<ul id="list"></ul>"#);
        runtime.run_script(
            r##"
            const item = document.createElement("li");
            item.setAttribute("class", "dynamic");
            const text = document.createTextNode("created by JavaScript");
            item.appendChild(text);
            document.querySelector("#list").appendChild(item);
            "##,
        );

        let item = dom.query_selector("li.dynamic").unwrap();
        assert_eq!(DomTree::inner_text(&item), "created by JavaScript");
        assert!(runtime.needs_redraw());
    }

    #[test]
    fn namespace_dom_apis_create_svg_elements_and_attributes() {
        let (mut runtime, dom) =
            runtime_from_html(r#"<main id="root"></main><div id="result"></div>"#);
        runtime.run_script(
            r##"
            const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
            const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
            path.setAttributeNS("http://www.w3.org/1999/xlink", "xlink:href", "#shape");
            svg.appendChild(path);
            document.querySelector("#root").appendChild(svg);

            const result = document.querySelector("#result");
            result.setAttribute("data-svg-ns", svg.namespaceURI);
            result.setAttribute("data-path-ns", path.namespaceURI);
            result.setAttribute("data-html-ns", result.namespaceURI);
            "##,
        );

        let path = dom.query_selector("path").unwrap();
        assert_eq!(path.borrow().value.get_attr("xlink:href"), Some("#shape"));
        let result = dom.get_element_by_id("result").unwrap();
        let result = result.borrow();
        assert_eq!(result.value.get_attr("data-svg-ns"), Some(SVG_NAMESPACE));
        assert_eq!(result.value.get_attr("data-path-ns"), Some(SVG_NAMESPACE));
        assert_eq!(result.value.get_attr("data-html-ns"), Some(HTML_NAMESPACE));
        assert!(runtime.needs_redraw());
    }

    #[test]
    fn element_contains_checks_self_descendants_and_unrelated_nodes() {
        let (mut runtime, dom) = runtime_from_html(
            r#"<main id="root"><section id="child"><span id="nested"></span></section></main><aside id="other"></aside>"#,
        );
        runtime.run_script(
            r##"
            const root = document.querySelector("#root");
            const child = document.querySelector("#child");
            const nested = document.querySelector("#nested");
            const other = document.querySelector("#other");
            root.setAttribute("data-self", root.contains(root));
            root.setAttribute("data-child", root.contains(child));
            root.setAttribute("data-nested", root.contains(nested));
            root.setAttribute("data-other", root.contains(other));
            root.setAttribute("data-null", root.contains(null));
            "##,
        );

        let root = dom.get_element_by_id("root").unwrap();
        let root = root.borrow();
        assert_eq!(root.value.get_attr("data-self"), Some("true"));
        assert_eq!(root.value.get_attr("data-child"), Some("true"));
        assert_eq!(root.value.get_attr("data-nested"), Some("true"));
        assert_eq!(root.value.get_attr("data-other"), Some("false"));
        assert_eq!(root.value.get_attr("data-null"), Some("false"));
    }

    #[test]
    fn document_tracks_the_focused_element() {
        let (mut runtime, dom) = runtime_from_html(
            r#"<body><input id="field"><button id="other"></button><div id="result"></div></body>"#,
        );
        runtime.run_script(
            r##"
            const field = document.querySelector("#field");
            const other = document.querySelector("#other");
            const result = document.querySelector("#result");
            result.setAttribute("data-initial", document.activeElement === document.body);
            field.focus();
            result.setAttribute("data-field", document.activeElement === field);
            other.focus();
            result.setAttribute("data-other", document.activeElement === other);
            other.blur();
            result.setAttribute("data-blurred", document.activeElement === document.body);
            result.setAttribute("data-has-focus", document.hasFocus());
            "##,
        );

        let result = dom.get_element_by_id("result").unwrap();
        let result = result.borrow();
        assert_eq!(result.value.get_attr("data-initial"), Some("true"));
        assert_eq!(result.value.get_attr("data-field"), Some("true"));
        assert_eq!(result.value.get_attr("data-other"), Some("true"));
        assert_eq!(result.value.get_attr("data-blurred"), Some("true"));
        assert_eq!(result.value.get_attr("data-has-focus"), Some("true"));
    }

    #[test]
    fn react_dom_node_primitives_identify_and_reorder_nodes() {
        let (mut runtime, dom) =
            runtime_from_html(r#"<main id="root"></main><div id="result"></div>"#);
        runtime.run_script(
            r##"
            const root = document.querySelector("#root");
            const first = document.createElement("span");
            first.setAttribute("data-name", "first");
            const second = document.createElement("span");
            second.setAttribute("data-name", "second");
            const text = document.createTextNode("before");
            text.nodeValue = "after";
            second.appendChild(text);
            root.appendChild(first);
            root.insertBefore(second, first);
            root.removeChild(first);
            second.className = "react-node";
            second.setAttribute("data-remove", "yes");
            second.removeAttribute("data-remove");

            const result = document.querySelector("#result");
            result.setAttribute("data-document-type", document.nodeType);
            result.setAttribute("data-root-type", root.nodeType);
            result.setAttribute("data-root-name", root.nodeName);
            result.setAttribute("data-owner", root.ownerDocument === document);
            result.setAttribute("data-first", root.firstChild.getAttribute("data-name"));
            result.setAttribute("data-last", root.lastChild.getAttribute("data-name"));
            result.setAttribute("data-count", root.childNodes.length);
            result.setAttribute("data-text", root.firstChild.firstChild.data);
            result.setAttribute("data-class", root.firstChild.className);
            result.setAttribute("data-removed", root.firstChild.hasAttribute("data-remove"));
            "##,
        );

        let result = dom.get_element_by_id("result").unwrap();
        let result = result.borrow();
        assert_eq!(result.value.get_attr("data-document-type"), Some("9"));
        assert_eq!(result.value.get_attr("data-root-type"), Some("1"));
        assert_eq!(result.value.get_attr("data-root-name"), Some("MAIN"));
        assert_eq!(result.value.get_attr("data-owner"), Some("true"));
        assert_eq!(result.value.get_attr("data-first"), Some("second"));
        assert_eq!(result.value.get_attr("data-last"), Some("second"));
        assert_eq!(result.value.get_attr("data-count"), Some("1"));
        assert_eq!(result.value.get_attr("data-text"), Some("after"));
        assert_eq!(result.value.get_attr("data-class"), Some("react-node"));
        assert_eq!(result.value.get_attr("data-removed"), Some("false"));

        let root = dom.get_element_by_id("root").unwrap();
        assert_eq!(root.borrow().children().len(), 1);
        assert_eq!(root.borrow().children()[0].borrow().value.get_attr("data-remove"), None);
    }

    #[test]
    fn html_iframe_element_supports_host_instance_checks() {
        let (mut runtime, dom) = runtime_from_html(
            r#"<body><iframe id="frame"></iframe><div id="result"></div></body>"#,
        );
        runtime.run_script(
            r#"
            const frame = document.getElementById("frame");
            const result = document.getElementById("result");
            result.setAttribute("data-frame", frame instanceof HTMLIFrameElement);
            result.setAttribute("data-body", document.body instanceof HTMLIFrameElement);
            "#,
        );

        let result = dom.get_element_by_id("result").unwrap();
        let result = result.borrow();
        assert_eq!(result.value.get_attr("data-frame"), Some("true"));
        assert_eq!(result.value.get_attr("data-body"), Some("false"));
    }

    #[test]
    fn remove_detaches_node_but_keeps_it_available_for_reappend() {
        let (mut runtime, dom) = runtime_from_html(
            r#"<div id="first"><span id="moving">move</span></div><div id="second"></div>"#,
        );
        runtime.run_script(
            r##"
            const moving = document.querySelector("#moving");
            moving.remove();
            document.querySelector("#second").appendChild(moving);
            "##,
        );

        let first = dom.get_element_by_id("first").unwrap();
        let second = dom.get_element_by_id("second").unwrap();
        assert!(DomTree::query_selector_within(&first, "#moving").is_none());
        assert!(DomTree::query_selector_within(&second, "#moving").is_some());
        assert!(runtime.needs_redraw());
    }

    #[test]
    fn parent_node_and_children_expose_tree_relationships() {
        let (mut runtime, dom) = runtime_from_html(
            r#"<div id="parent">text<span id="first"></span><span id="second"></span></div>"#,
        );
        runtime.run_script(
            r##"
            const first = document.querySelector("#first");
            first.parentNode.setAttribute("data-parent", "yes");
            const children = first.parentNode.children;
            children[1].setAttribute("data-second", "yes");
            first.parentNode.setAttribute("data-child-count", children.length);

            const text = document.createTextNode("dynamic");
            first.appendChild(text);
            text.parentNode.setAttribute("data-text-parent", "yes");
            "##,
        );

        let parent = dom.get_element_by_id("parent").unwrap();
        assert_eq!(parent.borrow().value.get_attr("data-parent"), Some("yes"));
        assert_eq!(
            parent.borrow().value.get_attr("data-child-count"),
            Some("2")
        );
        let first = dom.get_element_by_id("first").unwrap();
        assert_eq!(
            first.borrow().value.get_attr("data-text-parent"),
            Some("yes")
        );
        let second = dom.get_element_by_id("second").unwrap();
        assert_eq!(second.borrow().value.get_attr("data-second"), Some("yes"));
    }

    #[test]
    fn class_list_mutates_class_attribute_and_reports_membership() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="target" class="one two"></div>"#);
        runtime.run_script(
            r##"
            const target = document.querySelector("#target");
            target.classList.add("two", "three");
            target.classList.remove("one", "missing");
            target.setAttribute("data-has-three", target.classList.contains("three"));
            target.setAttribute("data-removed-three", target.classList.toggle("three"));
            target.setAttribute("data-added-four", target.classList.toggle("four"));
            target.setAttribute("data-forced-off", target.classList.toggle("four", false));
            target.setAttribute("data-forced-on", target.classList.toggle("five", true));
            "##,
        );

        let target = dom.get_element_by_id("target").unwrap();
        let target = target.borrow();
        assert_eq!(target.value.get_attr("class"), Some("two five"));
        assert_eq!(target.value.get_attr("data-has-three"), Some("true"));
        assert_eq!(target.value.get_attr("data-removed-three"), Some("false"));
        assert_eq!(target.value.get_attr("data-added-four"), Some("true"));
        assert_eq!(target.value.get_attr("data-forced-off"), Some("false"));
        assert_eq!(target.value.get_attr("data-forced-on"), Some("true"));
        assert!(runtime.needs_redraw());
    }

    #[test]
    fn timeout_runs_once_with_additional_arguments_and_can_be_cancelled() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r##"
            setTimeout(function (value) {
                document.querySelector("#result").setAttribute("data-value", value);
            }, 0, "done");
            const cancelled = setTimeout(function () {
                document.querySelector("#result").setAttribute("data-cancelled", "no");
            }, 0);
            clearTimeout(cancelled);
            "##,
        );

        assert!(runtime.run_due_timers());
        assert!(!runtime.run_due_timers());
        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(result.borrow().value.get_attr("data-value"), Some("done"));
        assert_eq!(result.borrow().value.get_attr("data-cancelled"), None);
    }

    #[test]
    fn interval_can_clear_itself() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r##"
            const intervalId = setInterval(function () {
                document.querySelector("#result").setAttribute("data-ran", "once");
                clearInterval(intervalId);
            }, 0);
            "##,
        );

        assert!(runtime.run_due_timers());
        assert!(!runtime.run_due_timers());
        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(result.borrow().value.get_attr("data-ran"), Some("once"));
    }

    #[test]
    fn performance_now_exposes_monotonic_runtime_time() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r#"
            const first = performance.now();
            const second = performance.now();
            document.getElementById("result").setAttribute(
                "data-monotonic",
                typeof first === "number" && second >= first
            );
            "#,
        );

        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(result.borrow().value.get_attr("data-monotonic"), Some("true"));
    }

    #[test]
    fn microtasks_run_in_fifo_order_after_script_evaluation() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r##"
            const result = document.querySelector("#result");
            queueMicrotask(function () {
                result.setAttribute("data-order", result.getAttribute("data-order") + "-first");
                queueMicrotask(function () {
                    result.setAttribute("data-order", result.getAttribute("data-order") + "-second");
                });
            });
            result.setAttribute("data-order", "sync");
            "##,
        );

        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(
            result.borrow().value.get_attr("data-order"),
            Some("sync-first-second")
        );
    }

    #[test]
    fn timer_microtasks_run_before_the_next_timer_task() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r##"
            const result = document.querySelector("#result");
            setTimeout(function () {
                result.setAttribute("data-order", "timer");
                queueMicrotask(function () {
                    result.setAttribute("data-order", "timer-microtask");
                });
            }, 0);
            setTimeout(function () {
                result.setAttribute("data-observed", result.getAttribute("data-order"));
            }, 0);
            "##,
        );

        assert!(runtime.run_due_timers());
        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(
            result.borrow().value.get_attr("data-observed"),
            Some("timer-microtask")
        );
    }

    #[test]
    fn promise_reactions_share_fifo_order_with_queued_microtasks() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r##"
            const result = document.querySelector("#result");
            result.setAttribute("data-order", "sync");
            queueMicrotask(function () {
                result.setAttribute("data-order", result.getAttribute("data-order") + "-first");
            });
            new Promise(function (resolve) {
                resolve("promise");
            }).then(function (value) {
                result.setAttribute("data-order", result.getAttribute("data-order") + "-" + value);
            });
            queueMicrotask(function () {
                result.setAttribute("data-order", result.getAttribute("data-order") + "-last");
            });
            "##,
        );

        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(
            result.borrow().value.get_attr("data-order"),
            Some("sync-first-promise-last")
        );
    }

    #[test]
    fn a_failed_microtask_does_not_block_later_jobs() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r##"
            queueMicrotask(function () {
                missingFunction();
            });
            queueMicrotask(function () {
                document.querySelector("#result").setAttribute("data-ran", "yes");
            });
            "##,
        );

        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(result.borrow().value.get_attr("data-ran"), Some("yes"));
    }

    #[test]
    fn promise_static_methods_complete_during_script_checkpoint() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r##"
            const result = document.querySelector("#result");
            Promise.all([Promise.resolve("first"), "second"])
                .then(function (values) {
                    result.setAttribute("data-all", values[0] + "-" + values[1]);
                    return Promise.reject("expected");
                })
                .catch(function (reason) {
                    result.setAttribute("data-catch", reason);
                });
            "##,
        );

        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(
            result.borrow().value.get_attr("data-all"),
            Some("first-second")
        );
        assert_eq!(
            result.borrow().value.get_attr("data-catch"),
            Some("expected")
        );
    }

    #[test]
    fn arrow_callbacks_work_with_promises_and_lexical_this() {
        let (mut runtime, dom) =
            runtime_from_html(r#"<button id="target"></button><div id="other"></div>"#);
        runtime.run_script(
            r##"
            const target = document.querySelector("#target");
            Promise.resolve("promise").then(value => {
                target.setAttribute("data-promise", value);
            });
            target.addEventListener("click", function () {
                const update = () => this.setAttribute("data-this", "target");
                update.call(document.querySelector("#other"));
            });
            "##,
        );

        let target = dom.get_element_by_id("target").unwrap();
        assert!(runtime.click(&target));
        assert_eq!(
            target.borrow().value.get_attr("data-promise"),
            Some("promise")
        );
        assert_eq!(target.borrow().value.get_attr("data-this"), Some("target"));
        let other = dom.get_element_by_id("other").unwrap();
        assert_eq!(other.borrow().value.get_attr("data-this"), None);
    }

    #[test]
    fn queue_microtask_accepts_an_arrow_callback() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r##"
            const result = document.querySelector("#result");
            queueMicrotask(() => {
                result.setAttribute("data-microtask", "yes");
            });
            "##,
        );

        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(
            result.borrow().value.get_attr("data-microtask"),
            Some("yes")
        );
    }

    #[test]
    fn browser_global_aliases_share_window_properties() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r##"
            const result = window.document.querySelector("#result");
            result.setAttribute("data-same-self", window === self);
            result.setAttribute("data-same-global", window === globalThis);
            result.setAttribute("data-document", window.document === document);
            window.queueMicrotask(() => {
                result.setAttribute("data-microtask", "yes");
            });
            "##,
        );

        let result = dom.get_element_by_id("result").unwrap();
        let result = result.borrow();
        assert_eq!(result.value.get_attr("data-same-self"), Some("true"));
        assert_eq!(result.value.get_attr("data-same-global"), Some("true"));
        assert_eq!(result.value.get_attr("data-document"), Some("true"));
        assert_eq!(result.value.get_attr("data-microtask"), Some("yes"));
    }

    #[test]
    fn fetch_resolves_response_metadata_and_text_promise() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r##"
            fetch("data:text/plain,hello").then(response => {
                const result = document.querySelector("#result");
                result.setAttribute("data-ok", response.ok);
                result.setAttribute("data-status", response.status);
                result.setAttribute("data-status-text", response.statusText);
                result.setAttribute("data-url", response.url);
                result.setAttribute("data-redirected", response.redirected);
                result.setAttribute("data-body-used-before", response.bodyUsed);
                const body = response.text();
                result.setAttribute("data-body-used-after", response.bodyUsed);
                return body;
            }).then(text => {
                document.querySelector("#result").setAttribute("data-text", text);
            });
            "##,
        );

        let requests = runtime.take_fetch_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, "data:text/plain,hello");
        runtime.resolve_fetch(
            requests[0].id,
            JsFetchResponse {
                url: "data:text/plain,hello".to_string(),
                status: 200,
                status_text: "All Good".to_string(),
                redirected: true,
                body: b"hello".to_vec(),
                headers: Vec::new(),
            },
        );

        let result = dom.get_element_by_id("result").unwrap();
        let result = result.borrow();
        assert_eq!(result.value.get_attr("data-ok"), Some("true"));
        assert_eq!(result.value.get_attr("data-status"), Some("200"));
        assert_eq!(
            result.value.get_attr("data-status-text"),
            Some("All Good")
        );
        assert_eq!(result.value.get_attr("data-redirected"), Some("true"));
        assert_eq!(
            result.value.get_attr("data-body-used-before"),
            Some("false")
        );
        assert_eq!(
            result.value.get_attr("data-body-used-after"),
            Some("true")
        );
        assert_eq!(
            result.value.get_attr("data-url"),
            Some("data:text/plain,hello")
        );
        assert_eq!(result.value.get_attr("data-text"), Some("hello"));
    }

    #[test]
    fn response_body_cannot_be_consumed_twice() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r##"
            fetch("data:text/plain,hello").then(response => {
                return response.text().then(() => response.text());
            }).catch(reason => {
                document.querySelector("#result").setAttribute("data-error", reason);
            });
            "##,
        );

        let requests = runtime.take_fetch_requests();
        runtime.resolve_fetch(
            requests[0].id,
            JsFetchResponse {
                url: "data:text/plain,hello".to_string(),
                status: 200,
                status_text: "OK".to_string(),
                redirected: false,
                body: b"hello".to_vec(),
                headers: Vec::new(),
            },
        );

        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(
            result.borrow().value.get_attr("data-error"),
            Some("Response body has already been consumed")
        );
    }

    #[test]
    fn fetch_captures_method_headers_and_body() {
        let (mut runtime, _dom) = runtime_from_html("<div></div>");
        runtime.run_script(
            r#"
            const headers = {};
            headers["Content-Type"] = "application/json";
            headers["X-Test"] = "yes";
            fetch("https://example.test/messages", {
                method: "post",
                headers: headers,
                body: "{\"message\":\"hello\"}"
            });
            "#,
        );

        let requests = runtime.take_fetch_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "POST");
        assert!(
            requests[0]
                .headers
                .contains(&("Content-Type".to_string(), "application/json".to_string()))
        );
        assert!(
            requests[0]
                .headers
                .contains(&("X-Test".to_string(), "yes".to_string()))
        );
        assert_eq!(requests[0].body, br#"{"message":"hello"}"#);
    }

    #[test]
    fn headers_are_case_insensitive_and_mutable() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r##"
            const headers = new Headers({ Accept: "application/json" });
            headers.append("X-Test", "one");
            headers.append("x-test", "two");
            headers.set("X-Replace", "before");
            headers.set("x-replace", "after");
            headers.delete("ACCEPT");

            const result = document.querySelector("#result");
            result.setAttribute("data-test", headers.get("X-TEST"));
            result.setAttribute("data-replace", headers.get("X-Replace"));
            result.setAttribute("data-has-accept", headers.has("accept"));
            result.setAttribute("data-missing", headers.get("missing") === null);

            fetch("https://example.test/", { headers: headers });
            "##,
        );

        let result = dom.get_element_by_id("result").unwrap();
        let result = result.borrow();
        assert_eq!(result.value.get_attr("data-test"), Some("one, two"));
        assert_eq!(result.value.get_attr("data-replace"), Some("after"));
        assert_eq!(result.value.get_attr("data-has-accept"), Some("false"));
        assert_eq!(result.value.get_attr("data-missing"), Some("true"));

        let requests = runtime.take_fetch_requests();
        assert!(
            requests[0]
                .headers
                .contains(&("x-test".to_string(), "one, two".to_string()))
        );
        assert!(
            requests[0]
                .headers
                .contains(&("x-replace".to_string(), "after".to_string()))
        );
    }

    #[test]
    fn response_exposes_read_only_headers() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r##"
            fetch("data:text/plain,hello").then(response => {
                const result = document.querySelector("#result");
                result.setAttribute("data-type", response.headers.get("Content-Type"));
                result.setAttribute("data-has", response.headers.has("X-Test"));
            });
            "##,
        );

        let requests = runtime.take_fetch_requests();
        runtime.resolve_fetch(
            requests[0].id,
            JsFetchResponse {
                url: "data:text/plain,hello".to_string(),
                status: 200,
                status_text: "OK".to_string(),
                redirected: false,
                body: b"hello".to_vec(),
                headers: vec![
                    ("content-type".to_string(), "text/plain".to_string()),
                    ("X-Test".to_string(), "yes".to_string()),
                ],
            },
        );

        let result = dom.get_element_by_id("result").unwrap();
        let result = result.borrow();
        assert_eq!(result.value.get_attr("data-type"), Some("text/plain"));
        assert_eq!(result.value.get_attr("data-has"), Some("true"));
    }

    #[test]
    fn request_objects_can_be_copied_and_passed_to_fetch() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r##"
            const headers = new Headers({ Accept: "application/json" });
            const original = new Request("https://example.test/messages", {
                method: "post",
                headers: headers,
                body: "hello"
            });
            const copied = new Request(original);
            const result = document.querySelector("#result");
            result.setAttribute("data-url", copied.url);
            result.setAttribute("data-method", copied.method);
            result.setAttribute("data-accept", copied.headers.get("accept"));
            fetch(copied, { method: "put" });
            "##,
        );

        let result = dom.get_element_by_id("result").unwrap();
        let result = result.borrow();
        assert_eq!(
            result.value.get_attr("data-url"),
            Some("https://example.test/messages")
        );
        assert_eq!(result.value.get_attr("data-method"), Some("POST"));
        assert_eq!(
            result.value.get_attr("data-accept"),
            Some("application/json")
        );

        let requests = runtime.take_fetch_requests();
        assert_eq!(requests[0].url, "https://example.test/messages");
        assert_eq!(requests[0].method, "PUT");
        assert_eq!(requests[0].body, b"hello");
        assert!(
            requests[0]
                .headers
                .contains(&("accept".to_string(), "application/json".to_string()))
        );
    }

    #[test]
    fn response_json_resolves_objects_and_arrays() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r##"
            fetch("data:application/json,pending").then(response => response.json()).then(value => {
                const result = document.querySelector("#result");
                result.setAttribute("data-name", value.name);
                result.setAttribute("data-second", value.items[1]);
                result.setAttribute("data-enabled", value.enabled);
                result.setAttribute("data-empty", value.empty === null);
            });
            "##,
        );

        let requests = runtime.take_fetch_requests();
        runtime.resolve_fetch(
            requests[0].id,
            JsFetchResponse {
                url: "data:application/json,pending".to_string(),
                status: 200,
                status_text: "OK".to_string(),
                redirected: false,
                body: br#"{"name":"Orinium","items":[1,2],"enabled":true,"empty":null}"#.to_vec(),
                headers: Vec::new(),
            },
        );

        let result = dom.get_element_by_id("result").unwrap();
        let result = result.borrow();
        assert_eq!(result.value.get_attr("data-name"), Some("Orinium"));
        assert_eq!(result.value.get_attr("data-second"), Some("2"));
        assert_eq!(result.value.get_attr("data-enabled"), Some("true"));
        assert_eq!(result.value.get_attr("data-empty"), Some("true"));
    }

    #[test]
    fn response_json_rejects_invalid_json() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r##"
            fetch("data:application/json,invalid")
                .then(response => response.json())
                .catch(reason => {
                    document.querySelector("#result").setAttribute("data-error", reason);
                });
            "##,
        );

        let requests = runtime.take_fetch_requests();
        runtime.resolve_fetch(
            requests[0].id,
            JsFetchResponse {
                url: "data:application/json,invalid".to_string(),
                status: 200,
                status_text: "OK".to_string(),
                redirected: false,
                body: b"not json".to_vec(),
                headers: Vec::new(),
            },
        );

        let result = dom.get_element_by_id("result").unwrap();
        assert!(
            result
                .borrow()
                .value
                .get_attr("data-error")
                .unwrap()
                .starts_with("Failed to parse JSON:")
        );
    }

    #[test]
    fn fetch_rejection_runs_catch_reaction() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r##"
            fetch("https://invalid.test/").catch(reason => {
                document.querySelector("#result").setAttribute("data-error", reason);
            });
            "##,
        );

        let requests = runtime.take_fetch_requests();
        runtime.reject_fetch(requests[0].id, "network failed".to_string());
        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(
            result.borrow().value.get_attr("data-error"),
            Some("network failed")
        );
    }
}
