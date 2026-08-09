//! Minimal JS runtime backed by `pixi_byte`.
//!
//! Hosts the JS engine on the UI thread and installs a small set of DOM
//! bindings (`console`, `document.getElementById`, element properties).
//! The engine never imports `platform`; DOM access goes through the shared
//! host slot that `JsRuntime` registers on the VM.

use crate::engine::html::{DomTree, HtmlNodeType};
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
}

/// The response data exposed to a JavaScript `Response` object.
pub(crate) struct JsFetchResponse {
    pub(crate) url: String,
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
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
    document: Option<Rc<RefCell<JSObject>>>,
    document_event_listeners: HashMap<String, Vec<JSValue>>,
    element_event_listeners: HashMap<u64, HashMap<String, Vec<JSValue>>>,
    /// Keeps JS-created or removed nodes alive while their wrappers exist.
    detached_nodes: HashMap<u64, NodeRef<HtmlNodeType>>,
    timers: Vec<JsTimer>,
    fetch_requests: Vec<JsFetchRequest>,
    fetch_capabilities: HashMap<u64, JsFetchCapability>,
    constructing_fetch_capability: Option<JsFetchCapability>,
    next_fetch_id: u64,
    next_timer_id: u64,
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
        let host = Rc::new(RefCell::new(JsHost {
            dom,
            refs: HashMap::new(),
            objects: HashMap::new(),
            document: None,
            document_event_listeners: HashMap::new(),
            element_event_listeners: HashMap::new(),
            detached_nodes: HashMap::new(),
            timers: Vec::new(),
            fetch_requests: Vec::new(),
            fetch_capabilities: HashMap::new(),
            constructing_fetch_capability: None,
            next_fetch_id: 0,
            next_timer_id: 0,
            dom_content_loaded_fired: false,
            next_id: 0,
            needs_redraw: Rc::clone(&needs_redraw),
        }));

        let mut engine = pixi_byte::JSEngine::new();
        engine.set_host(host);

        install_console(&mut engine);
        install_document(&mut engine);
        install_timers(&mut engine);
        install_microtasks(&mut engine);
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
            let event = make_event("DOMContentLoaded", Rc::clone(&document));
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
    /// supported. Returns whether at least one handler ran.
    pub fn click(&mut self, node: &NodeRef<HtmlNodeType>) -> bool {
        let Some(dom_id) = with_host(self.engine.vm(), |host| host.dom_id_for_node(node)).flatten()
        else {
            return false;
        };
        let Some(obj) =
            with_host(self.engine.vm(), |host| host.objects.get(&dom_id).cloned()).flatten()
        else {
            return false;
        };

        let onclick = obj.borrow().get("onclick");
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
            return false;
        }

        let event = make_event("click", Rc::clone(&obj));
        if has_onclick {
            if let Err(err) = self.engine.call(
                onclick,
                JSValue::Object(Rc::clone(&obj)),
                vec![JSValue::Object(Rc::clone(&event))],
            ) {
                log::info!("JS error in onclick: {}", err);
            }
        }
        for listener in listeners {
            if let Err(err) = self.engine.call(
                listener,
                JSValue::Object(Rc::clone(&obj)),
                vec![JSValue::Object(Rc::clone(&event))],
            ) {
                log::info!("JS error in click listener: {}", err);
            }
        }
        self.perform_microtask_checkpoint();
        true
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

fn install_fetch(engine: &mut pixi_byte::JSEngine) {
    engine
        .global_mut()
        .borrow_mut()
        .set("fetch".to_string(), JSValue::NativeFunction(fetch));
}

fn fetch(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let url = args
        .get(1)
        .cloned()
        .unwrap_or(JSValue::Undefined)
        .to_string();
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
        host.fetch_requests.push(JsFetchRequest { id, url });
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
        "ok".to_string(),
        Property::read_only(JSValue::Boolean((200..=299).contains(&response.status))),
    );
    object.define_property(
        "status".to_string(),
        Property::read_only(JSValue::Number(response.status as f64)),
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
    Rc::new(RefCell::new(object))
}

fn fetch_response_text(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let body = match args.first() {
        Some(JSValue::Object(response)) => response.borrow().get("__orinium_response_body"),
        _ => {
            return Err(JSError::TypeError(
                "Response.text called on incompatible receiver".to_string(),
            ));
        }
    };
    settle_promise(vm, "resolve", body)
}

fn fetch_response_json(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let body = match args.first() {
        Some(JSValue::Object(response)) => response.borrow().get("__orinium_response_body"),
        _ => {
            return Err(JSError::TypeError(
                "Response.json called on incompatible receiver".to_string(),
            ));
        }
    };
    let JSValue::String(body) = body else {
        return settle_promise(
            vm,
            "reject",
            JSValue::String("Response body is unavailable".to_string()),
        );
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
            "createTextNode".to_string(),
            JSValue::NativeFunction(create_text_node),
        );
        document.set(
            "addEventListener".to_string(),
            JSValue::NativeFunction(add_document_event_listener),
        );
    }
    let _ = with_host_mut(engine.vm(), |host| {
        host.document = Some(Rc::clone(&document_obj));
    });
    engine
        .global_mut()
        .borrow_mut()
        .set("document".to_string(), JSValue::Object(document_obj));
}

fn add_document_event_listener(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(JSValue::String(event_type)) = args.get(1) else {
        return Ok(JSValue::Undefined);
    };
    let Some(listener) = args.get(2).filter(|value| is_callable(value)).cloned() else {
        return Ok(JSValue::Undefined);
    };

    let _ = with_host_mut(vm, |host| {
        host.document_event_listeners
            .entry(event_type.clone())
            .or_default()
            .push(listener);
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

fn make_event(event_type: &str, target: Rc<RefCell<JSObject>>) -> Rc<RefCell<JSObject>> {
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
        Property::read_only(JSValue::Object(target)),
    );
    Rc::new(RefCell::new(event))
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

fn create_text_node(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let text = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    let node = TreeNode::new(HtmlNodeType::Text(text));
    Ok(expose_detached_node(vm, node).unwrap_or(JSValue::Null))
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
            Some((tag_name, attr_id)) => make_element(tag_name, attr_id, dom_id),
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

fn make_element(tag_name: String, attr_id: String, dom_id: u64) -> Rc<RefCell<JSObject>> {
    let mut obj = JSObject::new();
    define_node_id(&mut obj, dom_id);
    obj.define_property(
        "tagName".to_string(),
        Property::read_only(JSValue::String(tag_name)),
    );
    obj.define_property(
        "id".to_string(),
        Property::read_only(JSValue::String(attr_id)),
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
        "parentNode".to_string(),
        read_only_accessor_property(get_parent_node),
    );
    obj.define_property(
        "children".to_string(),
        read_only_accessor_property(get_element_children),
    );
    obj.define_property(
        "classList".to_string(),
        read_only_accessor_property(get_class_list),
    );
    obj.set(
        "getAttribute".to_string(),
        JSValue::NativeFunction(get_attribute),
    );
    obj.set(
        "setAttribute".to_string(),
        JSValue::NativeFunction(set_attribute),
    );
    obj.set(
        "addEventListener".to_string(),
        JSValue::NativeFunction(add_element_event_listener),
    );
    obj.set(
        "querySelector".to_string(),
        JSValue::NativeFunction(element_query_selector),
    );
    obj.set(
        "querySelectorAll".to_string(),
        JSValue::NativeFunction(element_query_selector_all),
    );
    obj.set(
        "appendChild".to_string(),
        JSValue::NativeFunction(append_child),
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
        "parentNode".to_string(),
        read_only_accessor_property(get_parent_node),
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
        host.element_event_listeners
            .entry(dom_id)
            .or_default()
            .entry(event_type.clone())
            .or_default()
            .push(listener);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::html::parser::Parser as HtmlParser;

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
                result.setAttribute("data-url", response.url);
                return response.text();
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
                body: b"hello".to_vec(),
            },
        );

        let result = dom.get_element_by_id("result").unwrap();
        let result = result.borrow();
        assert_eq!(result.value.get_attr("data-ok"), Some("true"));
        assert_eq!(result.value.get_attr("data-status"), Some("200"));
        assert_eq!(
            result.value.get_attr("data-url"),
            Some("data:text/plain,hello")
        );
        assert_eq!(result.value.get_attr("data-text"), Some("hello"));
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
                body: br#"{"name":"Orinium","items":[1,2],"enabled":true,"empty":null}"#.to_vec(),
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
                body: b"not json".to_vec(),
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
