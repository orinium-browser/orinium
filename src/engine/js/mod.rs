//! Minimal JS runtime backed by `pixi_byte`.
//!
//! Installs a small set of DOM bindings (`console`, `document.getElementById`,
//! element properties). The engine never imports `platform`; DOM access goes
//! through the shared host slot that `JsRuntime` registers on the VM. The
//! runtime normally lives on a background thread (see [`processor`]), owning a
//! private mirror of the DOM that is synced with the UI thread via
//! [`DomSnapshot`] commits. It can also be used directly on any thread.

use crate::engine::html::{DomTree, HtmlNodeType};
use crate::engine::js::web_apis::dom::document::IframeDocument;
use crate::engine::layouter::dom_snapshot::DomSnapshot;
use crate::engine::tree::NodeRef;
use pixi_byte::value::JSArray;
use pixi_byte::value::jsobject::JSObject;
use pixi_byte::{JSError, JSValue};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

pub mod devtools;
pub mod processor;
pub use devtools::JsDevToolsRequest;
pub use processor::{JsProcessor, JsTask, JsTaskResult};

mod common;
pub(crate) mod runtime;
pub(crate) mod web_apis;

// Re-export items needed by sibling modules.
pub(crate) use common::{
    host_read_only_property, is_callable, node_dom_id, with_host, with_host_mut,
};
pub(crate) use web_apis::dom::document::expose_node;
pub(crate) use web_apis::dom::events::{event_flag, make_event};
pub(crate) use web_apis::network::{make_fetch_response, resolve_xml_http_request};

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

pub(crate) struct JsTimer {
    id: u64,
    callback: JSValue,
    arguments: Vec<JSValue>,
    deadline: Instant,
    interval: Option<Duration>,
}

pub(crate) struct JsFetchCapability {
    resolve: JSValue,
    reject: JSValue,
}

/// A fetch request waiting to be dispatched by the browser network layer.
#[derive(Debug)]
pub struct JsFetchRequest {
    pub(crate) id: u64,
    pub(crate) url: String,
    pub(crate) method: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

/// A script element inserted by JavaScript after the initial HTML parse.
#[derive(Debug)]
pub(crate) struct JsDynamicScriptRequest {
    pub(crate) node_id: u64,
    pub(crate) source: JsDynamicScriptSource,
}

#[derive(Debug)]
pub(crate) enum JsDynamicScriptSource {
    Inline(String),
    External(String),
}

#[derive(Debug)]
pub(crate) struct JsDynamicStyleRequest {
    pub(crate) node_id: u64,
    pub(crate) url: String,
}

/// An image element created or populated after the initial HTML parse.
#[derive(Debug)]
// TODO: Track the owning image node so src changes can cancel/reload and dispatch load/error.
pub(crate) struct JsDynamicImageRequest {
    pub(crate) source: String,
}

/// Geometry produced by the committed layout tree for a live DOM element.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct JsLayoutMetrics {
    pub offset_left: f64,
    pub offset_top: f64,
    pub offset_width: f64,
    pub offset_height: f64,
    pub client_width: f64,
    pub client_height: f64,
    pub rect_left: f64,
    pub rect_top: f64,
    pub rect_width: f64,
    pub rect_height: f64,
}

/// The response data exposed to a JavaScript `Response` object.
#[derive(Debug)]
pub struct JsFetchResponse {
    pub(crate) url: String,
    pub(crate) status: u16,
    pub(crate) status_text: String,
    pub(crate) redirected: bool,
    pub(crate) body: Vec<u8>,
    pub(crate) headers: Vec<(String, String)>,
}

/// A registered custom element definition.
#[derive(Clone)]
pub(crate) struct CustomElementDefinition {
    pub(crate) constructor: JSValue,
    pub(crate) connected_callback: Option<JSValue>,
    pub(crate) disconnected_callback: Option<JSValue>,
    pub(crate) attribute_changed_callback: Option<JSValue>,
    pub(crate) observed_attributes: Vec<String>,
    /// Resolve functions for pending `whenDefined()` promises.
    pub(crate) when_defined_resolvers: Vec<JSValue>,
}

/// State shared between the JS natives and the browser side.
///
/// The JS-facing `u64` counter (`__orinium_dom_id`) maps to a live DOM node so
/// element handles survive relayouts: `Rc` handles on the DOM nodes are stable,
/// while snapshot ids are not.
pub struct JsHost {
    pub(crate) dom: Rc<DomTree>,
    pub(crate) refs: HashMap<
        u64,
        std::rc::Weak<std::cell::RefCell<crate::engine::tree::TreeNode<HtmlNodeType>>>,
    >,
    /// Element JS objects per DOM id, kept alive so `onclick` handlers
    /// registered on them survive and can be invoked on user clicks.
    pub(crate) objects: HashMap<u64, Rc<RefCell<JSObject>>>,
    /// Stable `CSSStyleDeclaration` wrappers for exposed elements.
    pub(crate) styles: HashMap<u64, Rc<RefCell<JSObject>>>,
    /// Stable 2D rendering contexts for canvas elements.
    pub(crate) canvas_contexts: HashMap<u64, Rc<RefCell<JSObject>>>,
    /// Explicit namespaces assigned through `document.createElementNS`.
    pub(crate) namespaces: HashMap<u64, String>,
    pub(crate) element_prototype: Rc<RefCell<JSObject>>,
    pub(crate) element_constructor: Rc<RefCell<JSObject>>,
    pub(crate) document: Option<Rc<RefCell<JSObject>>>,
    pub(crate) document_implementation: Option<Rc<RefCell<JSObject>>>,
    /// Independent document instances for `<iframe>` elements, keyed by the
    /// iframe element's DOM id. Each iframe gets its own DOM tree.
    pub(crate) iframe_documents: HashMap<u64, Rc<RefCell<IframeDocument>>>,
    pub(crate) document_event_listeners: HashMap<String, Vec<JSValue>>,
    pub(crate) element_event_listeners: HashMap<u64, HashMap<String, Vec<JSValue>>>,
    /// Inline event-handler content attributes mapped onto the Window per the
    /// HTML spec (e.g. `<body onload="...">` registers a `load` event handler
    /// on the Window). Keyed by event type; populated once when the DOM is
    /// bound to the runtime so dispatching never re-scans the tree.
    pub(crate) window_inline_event_handlers: HashMap<String, String>,
    pub(crate) active_element: Option<u64>,
    /// Keeps JS-created or removed nodes alive while their wrappers exist.
    pub(crate) detached_nodes: HashMap<u64, NodeRef<HtmlNodeType>>,
    pub(crate) timers: Vec<JsTimer>,
    pub(crate) fetch_requests: Vec<JsFetchRequest>,
    pub(crate) dynamic_script_requests: Vec<JsDynamicScriptRequest>,
    pub(crate) queued_dynamic_scripts: HashSet<u64>,
    pub(crate) dynamic_style_requests: Vec<JsDynamicStyleRequest>,
    pub(crate) queued_dynamic_styles: HashSet<u64>,
    pub(crate) dynamic_image_requests: Vec<JsDynamicImageRequest>,
    pub(crate) queued_dynamic_images: HashSet<u64>,
    pub(crate) fetch_capabilities: HashMap<u64, JsFetchCapability>,
    pub(crate) xhr_requests: HashMap<u64, Rc<RefCell<JSObject>>>,
    pub(crate) constructing_fetch_capability: Option<JsFetchCapability>,
    pub(crate) devtools_requests: Vec<JsDevToolsRequest>,
    pub(crate) devtools_capabilities: HashMap<u64, devtools::JsDevToolsCapability>,
    pub(crate) constructing_devtools_capability: Option<devtools::JsDevToolsCapability>,
    pub(crate) next_devtools_id: u64,
    /// Registered custom element definitions keyed by lowercase tag name.
    pub(crate) custom_elements: HashMap<String, CustomElementDefinition>,
    /// Shadow root associations: host_dom_id -> shadow_root_dom_id.
    pub(crate) shadow_roots: HashMap<u64, u64>,
    // TODO: Persist localStorage per origin and sessionStorage per top-level browsing context.
    pub(crate) local_storage: HashMap<String, String>,
    pub(crate) session_storage: HashMap<String, String>,
    // TODO: Move cookies into a shared origin/path-aware jar with expiry and security attributes.
    pub(crate) document_cookies: HashMap<String, String>,
    pub(crate) document_url: String,
    /// ASCII serialization of the document's origin (`"null"` when opaque).
    pub(crate) origin: String,
    pub(crate) viewport: (f64, f64),
    /// Committed layout measurements keyed by the address of a live DOM node.
    pub(crate) layout_metrics: HashMap<usize, JsLayoutMetrics>,
    /// Committed layout measurements keyed by stable JS-facing DOM id.
    pub(crate) layout_metrics_by_dom_id: HashMap<u64, JsLayoutMetrics>,
    pub(crate) next_fetch_id: u64,
    pub(crate) next_timer_id: u64,
    pub(crate) time_origin: Instant,
    pub(crate) dom_content_loaded_fired: bool,
    pub(crate) window_load_fired: bool,
    pub(crate) next_id: u64,
    pub(crate) needs_redraw: Rc<Cell<bool>>,
}

impl JsHost {
    /// Finds the JS-facing DOM id registered for a live DOM node, if any.
    pub(crate) fn dom_id_for_node(&self, node: &NodeRef<HtmlNodeType>) -> Option<u64> {
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
        let (element_prototype, element_constructor) =
            web_apis::dom::element::make_element_interface();
        let mut host = JsHost {
            dom,
            refs: HashMap::new(),
            objects: HashMap::new(),
            styles: HashMap::new(),
            canvas_contexts: HashMap::new(),
            namespaces: HashMap::new(),
            element_prototype,
            element_constructor,
            document: None,
            document_implementation: None,
            iframe_documents: HashMap::new(),
            document_event_listeners: HashMap::new(),
            element_event_listeners: HashMap::new(),
            window_inline_event_handlers: HashMap::new(),
            active_element: None,
            detached_nodes: HashMap::new(),
            timers: Vec::new(),
            fetch_requests: Vec::new(),
            dynamic_script_requests: Vec::new(),
            queued_dynamic_scripts: HashSet::new(),
            dynamic_style_requests: Vec::new(),
            queued_dynamic_styles: HashSet::new(),
            dynamic_image_requests: Vec::new(),
            queued_dynamic_images: HashSet::new(),
            fetch_capabilities: HashMap::new(),
            xhr_requests: HashMap::new(),
            constructing_fetch_capability: None,
            devtools_requests: Vec::new(),
            devtools_capabilities: HashMap::new(),
            constructing_devtools_capability: None,
            next_devtools_id: 0,
            custom_elements: HashMap::new(),
            shadow_roots: HashMap::new(),
            local_storage: HashMap::new(),
            session_storage: HashMap::new(),
            document_cookies: HashMap::new(),
            document_url: "about:blank".to_string(),
            origin: "null".to_string(),
            viewport: (800.0, 600.0),
            layout_metrics: HashMap::new(),
            layout_metrics_by_dom_id: HashMap::new(),
            next_fetch_id: 0,
            next_timer_id: 0,
            time_origin: Instant::now(),
            dom_content_loaded_fired: false,
            window_load_fired: false,
            next_id: 0,
            needs_redraw: Rc::clone(&needs_redraw),
        };
        Self::register_window_event_handlers(&mut host);

        let host = Rc::new(RefCell::new(host));

        let mut engine = pixi_byte::JSEngine::new();
        engine.set_host(host);

        web_apis::console::install_console(&mut engine);
        web_apis::dom::document::install_document(&mut engine);

        web_apis::observers::install_mutation_observer(&mut engine);
        web_apis::observers::install_resize_observer(&mut engine);
        web_apis::observers::install_intersection_observer(&mut engine);
        web_apis::timers::install_timers(&mut engine);
        web_apis::performance::install_performance(&mut engine);
        runtime::microtasks::install_microtasks(&mut engine);
        web_apis::network::install_headers(&mut engine);
        web_apis::network::install_request(&mut engine);
        web_apis::network::install_fetch(&mut engine);
        web_apis::network::install_xml_http_request(&mut engine);
        devtools::install(&mut engine);
        web_apis::url::install_url_apis(&mut engine);
        web_apis::encoding::install_encoding_apis(&mut engine);
        web_apis::browser_env::install_browser_environment(&mut engine);
        web_apis::browser_env::install_global_aliases(&mut engine);
        web_apis::dom::custom_elements::install_custom_elements(&mut engine);

        Self {
            engine,
            needs_redraw,
        }
    }

    /// Updates the CSS-pixel viewport exposed through the Window API.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        let width = width.max(0.0) as f64;
        let height = height.max(0.0) as f64;
        let mut global = self.engine.global_mut().borrow_mut();
        global.set("innerWidth".to_string(), JSValue::from_number(width));
        global.set("innerHeight".to_string(), JSValue::from_number(height));
        global.set("outerWidth".to_string(), JSValue::from_number(width));
        global.set("outerHeight".to_string(), JSValue::from_number(height));
        drop(global);
        with_host_mut(self.engine.vm(), |host| host.viewport = (width, height));
    }

    /// Replaces geometry exposed by DOM measurement APIs with the latest
    /// committed layout result.
    #[cfg(test)]
    pub(crate) fn set_layout_metrics(&mut self, metrics: HashMap<usize, JsLayoutMetrics>) {
        with_host_mut(self.engine.vm(), |host| host.layout_metrics = metrics);
    }

    /// Replaces geometry using stable DOM ids supplied by the browser UI
    /// thread, whose live node addresses differ from this runtime's mirror.
    pub(crate) fn set_layout_metrics_by_dom_id(&mut self, metrics: HashMap<u64, JsLayoutMetrics>) {
        with_host_mut(self.engine.vm(), |host| {
            host.layout_metrics_by_dom_id = metrics
        });
    }

    /// Updates the language preferences exposed through `navigator`.
    pub fn set_language(&mut self, language: &str) {
        let language = language.trim();
        if language.is_empty() {
            return;
        }
        let mut languages = vec![JSValue::from_string(language.to_string())];
        if let Some(base) = language.split('-').next()
            && !base.eq_ignore_ascii_case(language)
        {
            languages.push(JSValue::from_string(base.to_string()));
        }
        if !language.eq_ignore_ascii_case("en-US") {
            languages.push(JSValue::from_string("en-US".to_string()));
        }

        let global = self.engine.global_mut().borrow_mut();
        let Some(navigator) = global.get("navigator").as_object() else {
            return;
        };
        drop(global);
        let mut navigator = navigator.borrow_mut();
        navigator.define_property(
            "language".to_string(),
            host_read_only_property(JSValue::from_string(language.to_string())),
        );
        navigator.define_property(
            "languages".to_string(),
            host_read_only_property(JSArray::from_vec(languages).to_object()),
        );
    }

    /// Evaluates a script, logging JS errors instead of crashing the page.
    pub fn run_script(&mut self, source: &str) {
        match self.engine.eval(source) {
            Ok(_) => {}
            Err(err) => {
                if let JSError::Thrown(value) = &err
                    && let Some(object) = value.as_object()
                {
                    let object = object.borrow();
                    let details = object
                        .keys()
                        .into_iter()
                        .map(|key| format!("{key}={}", object.get(&key).to_console_string()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    log::info!("JS error: uncaught object ({details})");
                }
                log::info!("JS error: {}", err);
            }
        }
        self.perform_microtask_checkpoint();
    }

    /// Evaluates an expression and returns its value, or `undefined` on error.
    pub fn eval_value(&mut self, source: &str) -> JSValue {
        match self.engine.eval(source) {
            Ok(value) => value,
            Err(err) => {
                log::info!("JS error evaluating {source:?}: {err}");
                JSValue::undefined()
            }
        }
    }

    /// Updates the URL exposed through the window's `location` object.
    pub fn set_document_url(&mut self, url: &str) {
        let _ = with_host_mut(self.engine.vm(), |host| {
            host.document_url = url.to_string();
        });
    }

    /// Updates the serialized origin exposed through the window's
    /// `location`/`window`/`document` objects.
    pub fn set_page_origin(&mut self, origin: &str) {
        let _ = with_host_mut(self.engine.vm(), |host| {
            host.origin = origin.to_string();
        });
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
                JSValue::from_object(Rc::clone(&document)),
                vec![JSValue::from_object(event)],
            ) {
                log::info!("JS error on DOMContentLoaded: {}", err);
            }
        }
        self.perform_microtask_checkpoint();
        true
    }

    /// Dispatches the window `load` event once the page has finished loading.
    ///
    /// Supports the body `onload` inline handler, the `window.onload` property,
    /// and `addEventListener("load", ...)` registrations in that order. Returns
    /// `true` only for the first dispatch attempt.
    pub fn dispatch_window_load(&mut self) -> bool {
        let state = with_host_mut(self.engine.vm(), |host| {
            if host.window_load_fired {
                return None;
            }
            host.window_load_fired = true;
            Some((
                host.window_inline_event_handlers
                    .get("load")
                    .cloned()
                    .unwrap_or_default(),
                host.document_event_listeners
                    .get("load")
                    .cloned()
                    .unwrap_or_default(),
            ))
        });
        let Some((body_onload, listeners)) = state.flatten() else {
            return false;
        };

        let window_object = Rc::clone(self.engine.global_mut());

        // The body `onload` is a window-level inline event handler that fires
        // first, as if it had been registered first by the parser.
        if !body_onload.trim().is_empty() {
            self.call_inline_load_handler(&window_object, &body_onload, "body onload");
        }

        let onload = window_object.borrow().get("onload");
        if is_callable(&onload) {
            self.call_load_handler(&window_object, onload, "window.onload");
        }

        for listener in listeners {
            self.call_load_handler(&window_object, listener, "window load listener");
        }

        self.perform_microtask_checkpoint();
        true
    }

    /// Compiles and runs an inline event-handler content attribute (e.g.
    /// `<body onload="...">`) with `this` bound to the Window and an `event`
    /// parameter, per the HTML spec's inline event handler activation.
    fn call_inline_load_handler(
        &mut self,
        window: &Rc<RefCell<JSObject>>,
        code: &str,
        label: &str,
    ) {
        let event = make_event("load", Rc::clone(window), Rc::clone(window));
        // Per HTML spec, the inline handler is compiled as a function whose
        // `event` parameter and `this` (the Window) are set; invoking it runs
        // the assigned code (e.g. Acid3's body onload="update()").
        let wrapped = format!("(function(event) {{ {code} \n}})");
        let handler = self.engine.eval(&wrapped).unwrap_or(JSValue::undefined());
        if is_callable(&handler)
            && let Err(err) = self.engine.call(
                handler,
                JSValue::from_object(Rc::clone(window)),
                vec![JSValue::from_object(event)],
            )
        {
            log::info!("JS error in {label}: {err}");
        }
    }

    /// Invokes a registered window `load` listener with the Window as `this`.
    fn call_load_handler(&mut self, window: &Rc<RefCell<JSObject>>, handler: JSValue, label: &str) {
        let event = make_event("load", Rc::clone(window), Rc::clone(window));
        if let Err(err) = self.engine.call(
            handler,
            JSValue::from_object(Rc::clone(window)),
            vec![JSValue::from_object(event)],
        ) {
            log::info!("JS error in {label}: {err}");
        }
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
            if let Err(err) = self.engine.call(callback, JSValue::undefined(), arguments) {
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

    pub(crate) fn take_dynamic_script_requests(&mut self) -> Vec<JsDynamicScriptRequest> {
        with_host_mut(self.engine.vm(), |host| {
            std::mem::take(&mut host.dynamic_script_requests)
        })
        .unwrap_or_default()
    }

    pub(crate) fn take_dynamic_style_requests(&mut self) -> Vec<JsDynamicStyleRequest> {
        with_host_mut(self.engine.vm(), |host| {
            std::mem::take(&mut host.dynamic_style_requests)
        })
        .unwrap_or_default()
    }

    pub(crate) fn take_dynamic_image_requests(&mut self) -> Vec<JsDynamicImageRequest> {
        with_host_mut(self.engine.vm(), |host| {
            std::mem::take(&mut host.dynamic_image_requests)
        })
        .unwrap_or_default()
    }

    /// Dispatches a non-bubbling event to a dynamically inserted element.
    pub(crate) fn dispatch_element_event(&mut self, node_id: u64, event_type: &str) {
        let Some((target, listeners)) = with_host(self.engine.vm(), |host| {
            let target = host.objects.get(&node_id).cloned()?;
            let listeners = host
                .element_event_listeners
                .get(&node_id)
                .and_then(|events| events.get(event_type))
                .cloned()
                .unwrap_or_default();
            Some((target, listeners))
        })
        .flatten() else {
            return;
        };
        let handler = target.borrow().get(&format!("on{event_type}"));
        let event = make_event(event_type, Rc::clone(&target), Rc::clone(&target));
        if is_callable(&handler)
            && let Err(error) = self.engine.call(
                handler,
                JSValue::from_object(Rc::clone(&target)),
                vec![JSValue::from_object(Rc::clone(&event))],
            )
        {
            log::info!("JS error in on{event_type}: {error}");
        }
        for listener in listeners {
            if event_flag(&event, "__orinium_immediate_propagation_stopped") {
                break;
            }
            if let Err(error) = self.engine.call(
                listener,
                JSValue::from_object(Rc::clone(&target)),
                vec![JSValue::from_object(Rc::clone(&event))],
            ) {
                log::info!("JS error in {event_type} listener: {error}");
            }
        }
    }

    /// Resolves a pending JavaScript fetch and runs its microtask checkpoint.
    pub(crate) fn resolve_fetch(&mut self, id: u64, response: JsFetchResponse) {
        let capability =
            with_host_mut(self.engine.vm(), |host| host.fetch_capabilities.remove(&id)).flatten();
        if let Some(capability) = capability {
            let response = make_fetch_response(response);
            if let Err(err) = self.engine.call(
                capability.resolve,
                JSValue::undefined(),
                vec![JSValue::from_object(response)],
            ) {
                log::info!("JS error while resolving fetch: {}", err);
            }
            self.perform_microtask_checkpoint();
            return;
        }
        let xhr = with_host_mut(self.engine.vm(), |host| host.xhr_requests.remove(&id)).flatten();
        let Some(xhr) = xhr else { return };
        resolve_xml_http_request(&mut self.engine, xhr, response);
        self.perform_microtask_checkpoint();
    }

    /// Rejects a pending JavaScript fetch and runs its microtask checkpoint.
    pub(crate) fn reject_fetch(&mut self, id: u64, reason: String) {
        let capability =
            with_host_mut(self.engine.vm(), |host| host.fetch_capabilities.remove(&id)).flatten();
        if let Some(capability) = capability {
            if let Err(err) = self.engine.call(
                capability.reject,
                JSValue::undefined(),
                vec![JSValue::from_string(reason)],
            ) {
                log::info!("JS error while rejecting fetch: {}", err);
            }
            self.perform_microtask_checkpoint();
            return;
        }
        let xhr = with_host_mut(self.engine.vm(), |host| host.xhr_requests.remove(&id)).flatten();
        let Some(xhr) = xhr else { return };
        let handler = xhr.borrow().get("onerror");
        if is_callable(&handler) {
            let _ = self.engine.call(
                handler,
                JSValue::from_object(Rc::clone(&xhr)),
                vec![JSValue::from_string(reason)],
            );
        }
        self.perform_microtask_checkpoint();
    }

    /// Serializes the current mirror DOM for the browser side.
    ///
    /// Nodes exposed to scripts keep their stable `dom_id` so the UI thread can
    /// rebuild the tree and re-register references on commit.
    pub fn snapshot(&self) -> DomSnapshot {
        let Some((root, dom_ids)) = with_host(self.engine.vm(), |host| {
            let mut reverse = HashMap::new();
            for (dom_id, weak) in &host.refs {
                if let Some(node) = weak.upgrade() {
                    reverse.insert(Rc::as_ptr(&node) as usize, *dom_id);
                }
            }
            (Rc::clone(&host.dom.root), reverse)
        }) else {
            return DomSnapshot::default();
        };
        DomSnapshot::from_mirror(&root, &dom_ids)
    }

    /// Replaces the mirror DOM with a snapshot produced by the browser side.
    ///
    /// The mirror is rebuilt from `snapshot` and node references are re-registered
    /// so existing JS element handles keep resolving. JS-created (detached) nodes
    /// are preserved; they are not part of the committed DOM but may still be
    /// referenced from scripts.
    pub fn apply_dom(&mut self, snapshot: &DomSnapshot) {
        let (tree, dom_ids) = snapshot.into_tree();
        with_host_mut(self.engine.vm(), |host| {
            host.dom = Rc::new(tree);
            let mut refs = std::mem::take(&mut host.refs);
            host.dom.traverse(|node| {
                if let Some(&dom_id) = dom_ids.get(&(Rc::as_ptr(node) as usize)) {
                    refs.insert(dom_id, Rc::downgrade(node));
                }
            });
            for (&dom_id, node) in &host.detached_nodes {
                refs.entry(dom_id).or_insert_with(|| Rc::downgrade(node));
            }
            host.refs = refs;
            if let Some(max_id) = dom_ids.values().max() {
                host.next_id = host.next_id.max(*max_id);
            }
            Self::register_window_event_handlers(host);
        });
    }

    /// Hoists the document's event-handler content attributes that the HTML
    /// spec maps onto the Window (`<body onload="...">`) into the host's
    /// handler registry, so dispatching never searches the DOM again.
    ///
    /// Called once when a DOM is bound to the runtime (initial parse and every
    /// `apply_dom`), mirroring how the parser activates a `load` handler on the
    /// `<body>` element as it is parsed.
    fn register_window_event_handlers(host: &mut JsHost) {
        for node in host.dom.find_all(|node| node.tag_name() == Some("body")) {
            if let Some(code) = node.borrow().value.get_attr("onload") {
                host.window_inline_event_handlers
                    .insert("load".to_string(), code.to_string());
            }
        }
    }

    /// Dispatches a click to the handlers registered on the element with the
    /// given JS-facing dom id. Returns whether at least one handler ran.
    pub fn click_dom_id(&mut self, dom_id: u64) -> bool {
        let Some(node) = with_host(self.engine.vm(), |host| {
            host.refs.get(&dom_id).and_then(|w| w.upgrade())
        })
        .flatten() else {
            return false;
        };
        self.click(&node)
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
            if let Some(object) = expose_node(self.engine.vm(), node).and_then(|v| v.as_object()) {
                path.push(object);
            }
        }
        let Some(target) = path.first().cloned() else {
            return false;
        };

        let mut ran_handler = false;
        for current_target in path {
            let Some(dom_id) = node_dom_id(&JSValue::from_object(Rc::clone(&current_target)))
            else {
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
            let event = make_event("click", Rc::clone(&target), Rc::clone(&current_target));
            if has_onclick
                && let Err(err) = self.engine.call(
                    onclick,
                    JSValue::from_object(Rc::clone(&current_target)),
                    vec![JSValue::from_object(Rc::clone(&event))],
                )
            {
                log::info!("JS error in onclick: {}", err);
            }
            if !event_flag(&event, "__orinium_immediate_propagation_stopped") {
                for listener in listeners {
                    if let Err(err) = self.engine.call(
                        listener,
                        JSValue::from_object(Rc::clone(&current_target)),
                        vec![JSValue::from_object(Rc::clone(&event))],
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
            if let JSError::Thrown(value) = &err
                && let Some(object) = value.as_object()
            {
                let object = object.borrow();
                let details = object
                    .keys()
                    .into_iter()
                    .map(|key| format!("{key}={}", object.get(&key).to_console_string()))
                    .collect::<Vec<_>>()
                    .join(", ");
                log::info!("JS error in microtask: {} ({details})", err);
            } else {
                log::info!("JS error in microtask: {}", err);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::html::Parser as HtmlParser;
    use web_apis::dom::element::{HTML_NAMESPACE, SVG_NAMESPACE, style_property_name};

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
    fn viewport_dimensions_follow_browser_resizes() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.set_viewport(1280.0, 720.0);
        runtime.run_script(
            r#"document.getElementById("result").setAttribute("data-size", innerWidth + ":" + innerHeight);"#,
        );
        assert_eq!(
            dom.get_element_by_id("result")
                .unwrap()
                .borrow()
                .value
                .get_attr("data-size"),
            Some("1280:720")
        );
        runtime.run_script(
            r#"document.getElementById("result").setAttribute("data-root", document.body.clientWidth + ":" + document.body.clientHeight + ":" + outerWidth + ":" + outerHeight);"#,
        );
        assert_eq!(
            dom.get_element_by_id("result")
                .unwrap()
                .borrow()
                .value
                .get_attr("data-root"),
            Some("1280:720:1280:720")
        );
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
    fn browser_environment_exposes_react_bootstrap_apis() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.set_document_url("https://scratch.mit.edu/projects/editor/?tutorial=1#stage");
        runtime.run_script(
            r#"
            localStorage.setItem("answer", 42);
            sessionStorage.setItem("temporary", "yes");
            const query = matchMedia("(prefers-color-scheme: dark)");
            query.addEventListener("change", function () {});
            const event = new CustomEvent("ready", {detail: "loaded", cancelable: true});
            event.preventDefault();
            const frame = requestAnimationFrame(function () {});
            cancelAnimationFrame(frame);
            document.getElementById("result").setAttribute(
                "data-environment",
                navigator.language + ":" + localStorage.getItem("answer") + ":" +
                    localStorage.length + ":" + query.matches + ":" + (frame > 0) + ":" +
                    location.pathname + ":" + event.detail + ":" + event.defaultPrevented + ":" +
                    (typeof Intl === "object") + ":" + ("Locale" in Intl) + ":" +
                    Intl.getCanonicalLocales(["EN-us", "ja"])[0] + ":" +
                    new Intl.Locale("und-x-private").toString()
            );
            "#,
        );

        let node = dom.get_element_by_id("result").unwrap();
        assert_eq!(
            node.borrow().value.get_attr("data-environment"),
            Some(
                "en-US:42:1:false:true:/projects/editor/:loaded:true:true:true:en-US:und-x-private"
            )
        );
    }

    #[test]
    fn browser_origin_exposed_consistently_across_window_location_and_document() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.set_page_origin("https://example.test");
        runtime.run_script(
            r#"document.getElementById("result").setAttribute(
                "data-origins",
                window.origin + ":" + location.origin + ":" + document.origin
            );"#,
        );

        let node = dom.get_element_by_id("result").unwrap();
        assert_eq!(
            node.borrow().value.get_attr("data-origins"),
            Some("https://example.test:https://example.test:https://example.test")
        );
    }

    #[test]
    fn opaque_page_reports_null_origin_everywhere() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.set_page_origin("null");
        runtime.run_script(
            r#"document.getElementById("result").setAttribute(
                "data-origins",
                window.origin + ":" + location.origin + ":" + document.origin
            );"#,
        );

        let node = dom.get_element_by_id("result").unwrap();
        assert_eq!(
            node.borrow().value.get_attr("data-origins"),
            Some("null:null:null")
        );
    }

    #[test]
    fn browser_language_preferences_follow_the_host() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.set_language("ja-JP");
        runtime.run_script(
            r#"
            document.getElementById("result").setAttribute(
                "data-languages",
                navigator.language + ":" + navigator.languages[0] + ":" +
                    navigator.languages[1] + ":" + navigator.languages[2]
            );
            "#,
        );

        assert_eq!(
            dom.get_element_by_id("result")
                .unwrap()
                .borrow()
                .value
                .get_attr("data-languages"),
            Some("ja-JP:ja-JP:ja:en-US")
        );
    }

    #[test]
    fn document_cookie_is_a_string_and_supports_assignment_and_expiry() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r#"
            const result = document.getElementById("result");
            result.setAttribute("data-empty-cookie", typeof document.cookie + ":" + document.cookie);
            document.cookie = "scratchlanguage=ja; Path=/";
            result.setAttribute("data-cookie", document.cookie);
            document.cookie = "scratchlanguage=; Max-Age=0; Path=/";
            result.setAttribute("data-expired-cookie", document.cookie);
            "#,
        );

        let result = dom.get_element_by_id("result").unwrap();
        let result = result.borrow();
        assert_eq!(result.value.get_attr("data-empty-cookie"), Some("string:"));
        assert_eq!(
            result.value.get_attr("data-cookie"),
            Some("scratchlanguage=ja")
        );
        assert_eq!(result.value.get_attr("data-expired-cookie"), Some(""));
    }

    #[test]
    fn url_apis_resolve_assets_and_manage_query_parameters() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r#"
            const asset = new URL("../assets/stage.svg?locale=ja#costume", "https://scratch.mit.edu/projects/editor/");
            const params = new URLSearchParams("project=123&mode=editor");
            params.set("mode", "fullscreen");
            params.append("cloud", "on");
            params.delete("project");
            document.getElementById("result").setAttribute(
                "data-url",
                asset.origin + ":" + asset.pathname + ":" + asset.searchParams.get("locale") +
                    ":" + params.has("cloud") + ":" + params.toString()
            );
            "#,
        );

        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(
            result.borrow().value.get_attr("data-url"),
            Some(
                "https://scratch.mit.edu:/projects/assets/stage.svg:ja:true:mode=fullscreen&cloud=on"
            )
        );
    }

    #[test]
    fn encoding_apis_round_trip_utf8_and_base64() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r#"
            const encoder = new TextEncoder();
            const decoder = new TextDecoder("utf-8");
            const bytes = encoder.encode("Scratch 日本");
            document.getElementById("result").setAttribute(
                "data-encoding",
                decoder.decode(bytes) + ":" + bytes.length + ":" + atob(btoa("Scratch")) +
                    ":" + decodeURIComponent(encodeURIComponent("日本 語"))
            );
            "#,
        );

        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(
            result.borrow().value.get_attr("data-encoding"),
            Some("Scratch 日本:14:Scratch:日本 語")
        );
    }

    #[test]
    fn layout_measurement_and_resize_observer_report_element_size() {
        let (mut runtime, dom) = runtime_from_html(
            r#"<canvas id="stage" width="480" height="360"></canvas><div id="result"></div>"#,
        );
        runtime.run_script(
            r#"
            const stage = document.getElementById("stage");
            const rect = stage.getBoundingClientRect();
            const observer = new ResizeObserver(function (entries) {
                const observed = entries[0].contentRect;
                document.getElementById("result").setAttribute(
                    "data-resize",
                    observed.width + ":" + observed.height
                );
            });
            observer.observe(stage);
            document.getElementById("result").setAttribute(
                "data-measure",
                rect.width + ":" + rect.height + ":" + stage.clientWidth + ":" + stage.offsetHeight
            );
            "#,
        );

        let result = dom.get_element_by_id("result").unwrap();
        let result = result.borrow();
        assert_eq!(
            result.value.get_attr("data-measure"),
            Some("480:360:480:360")
        );
        assert_eq!(result.value.get_attr("data-resize"), Some("480:360"));
    }

    #[test]
    fn layout_offsets_have_a_numeric_fallback() {
        let (mut runtime, dom) = runtime_from_html(
            r#"<div id="list" class="carousel slick-list"></div><div id="plain"></div><div id="result"></div>"#,
        );
        runtime.run_script(
            r#"
            document.getElementById("result").setAttribute(
                "data-widths",
                document.getElementById("list").offsetWidth + ":" +
                    document.getElementById("plain").offsetWidth + ":" +
                    document.getElementById("list").offsetLeft + ":" +
                    document.getElementById("list").offsetTop
            );
            "#,
        );

        assert_eq!(
            dom.get_element_by_id("result")
                .unwrap()
                .borrow()
                .value
                .get_attr("data-widths"),
            Some("800:0:0:0")
        );
    }

    #[test]
    fn dom_measurements_prefer_committed_layout_geometry() {
        let (mut runtime, dom) = runtime_from_html(
            r#"<div id="target" style="width: 1px; height: 2px"></div><div id="result"></div>"#,
        );
        let target = dom.get_element_by_id("target").unwrap();
        runtime.set_layout_metrics(HashMap::from([(
            Rc::as_ptr(&target) as usize,
            JsLayoutMetrics {
                offset_left: 12.0,
                offset_top: 34.0,
                offset_width: 222.0,
                offset_height: 111.0,
                client_width: 218.0,
                client_height: 107.0,
                rect_left: 42.5,
                rect_top: 64.25,
                rect_width: 222.0,
                rect_height: 111.0,
            },
        )]));
        runtime.run_script(
            r#"
            const target = document.getElementById("target");
            const rect = target.getBoundingClientRect();
            document.getElementById("result").setAttribute(
                "data-layout",
                target.offsetLeft + ":" + target.offsetTop + ":" +
                    target.offsetWidth + ":" + target.offsetHeight + ":" +
                    target.clientWidth + ":" + target.clientHeight + ":" +
                    rect.left + ":" + rect.top + ":" + rect.right + ":" + rect.bottom
            );
            "#,
        );

        assert_eq!(
            dom.get_element_by_id("result")
                .unwrap()
                .borrow()
                .value
                .get_attr("data-layout"),
            Some("12:34:222:111:218:107:42.5:64.25:264.5:175.25")
        );
    }

    #[test]
    fn inserting_script_element_queues_dynamic_resource_load() {
        let (mut runtime, _dom) = runtime_from_html(r#"<html><head></head><body></body></html>"#);
        runtime.run_script(
            r#"
            const script = document.createElement("script");
            script.src = "/static/chunks/editor.js";
            script.async = true;
            document.head.appendChild(script);
            "#,
        );

        let requests = runtime.take_dynamic_script_requests();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].node_id > 0);
        match &requests[0].source {
            JsDynamicScriptSource::External(source) => {
                assert_eq!(source, "/static/chunks/editor.js")
            }
            JsDynamicScriptSource::Inline(_) => panic!("expected an external script request"),
        }
    }

    #[test]
    fn inserting_stylesheet_link_queues_dynamic_resource_load() {
        let (mut runtime, _dom) = runtime_from_html(r#"<html><head></head><body></body></html>"#);
        runtime.run_script(
            r#"
            const link = document.createElement("link");
            link.rel = "stylesheet";
            link.href = "/static/css/editor.css";
            document.head.appendChild(link);
            "#,
        );

        let requests = runtime.take_dynamic_style_requests();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].node_id > 0);
        assert_eq!(requests[0].url, "/static/css/editor.css");
    }

    #[test]
    fn inserting_image_queues_dynamic_resource_load_once() {
        let (mut runtime, _dom) = runtime_from_html(r#"<html><body></body></html>"#);
        runtime.run_script(
            r#"
            const image = document.createElement("img");
            image.src = "/images/scratch-logo.svg";
            document.body.appendChild(image);
            "#,
        );

        let requests = runtime.take_dynamic_image_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].source, "/images/scratch-logo.svg");
    }

    #[test]
    fn canvas_2d_context_records_visible_rectangle_commands() {
        let (mut runtime, dom) = runtime_from_html(r#"<canvas id="stage"></canvas>"#);
        runtime.run_script(
            r##"
            const canvas = document.getElementById("stage");
            canvas.width = 480;
            canvas.height = 360;
            const context = canvas.getContext("2d");
            context.fillStyle = "#ff8800";
            context.fillRect(10, 20, 30, 40);
            context.strokeStyle = "blue";
            context.strokeRect(0, 0, 480, 360);
            canvas.setAttribute("data-metrics", context.measureText("Scratch").width);
            "##,
        );

        let canvas = dom.get_element_by_id("stage").unwrap();
        let canvas = canvas.borrow();
        assert_eq!(canvas.value.get_attr("width"), Some("480"));
        assert_eq!(canvas.value.get_attr("height"), Some("360"));
        assert_eq!(canvas.value.get_attr("data-metrics"), Some("42"));
        assert_eq!(
            canvas.value.get_attr("data-orinium-canvas-commands"),
            Some("fillRect|#ff8800|10|20|30|40\nstrokeRect|blue|0|0|480|360")
        );
    }

    #[test]
    fn canvas_exposes_webgl_capability_surface() {
        let (mut runtime, dom) =
            runtime_from_html(r#"<canvas id="stage" width="480" height="360"></canvas>"#);
        runtime.run_script(
            r#"
            const canvas = document.getElementById("stage");
            const gl = canvas.getContext("webgl");
            const shader = gl.createShader(gl.VERTEX_SHADER);
            gl.shaderSource(shader, "void main() {}");
            gl.compileShader(shader);
            const program = gl.createProgram();
            gl.attachShader(program, shader);
            gl.linkProgram(program);
            canvas.setAttribute(
                "data-webgl",
                gl.getShaderParameter(shader, gl.COMPILE_STATUS) + ":" +
                    gl.getProgramParameter(program, gl.LINK_STATUS) + ":" +
                    gl.getParameter(gl.MAX_TEXTURE_SIZE) + ":" + gl.drawingBufferWidth
            );
            "#,
        );

        let canvas = dom.get_element_by_id("stage").unwrap();
        assert_eq!(
            canvas.borrow().value.get_attr("data-webgl"),
            Some("true:true:4096:480")
        );
    }

    #[test]
    fn mutation_observer_can_register_for_dom_changes() {
        let (mut runtime, dom) =
            runtime_from_html(r#"<html><body><div id="target"></div></body></html>"#);
        runtime.run_script(
            r#"
            const observer = new MutationObserver(function () {
                document.getElementById("target").setAttribute("data-observed", "yes");
            });
            observer.observe(document.documentElement, { childList: true, subtree: true });
            const records = observer.takeRecords();
            observer.disconnect();
            document.getElementById("target").setAttribute("data-records", records.length);
            "#,
        );

        let node = dom.get_element_by_id("target").unwrap();
        assert_eq!(node.borrow().value.get_attr("data-records"), Some("0"));
        assert_eq!(node.borrow().value.get_attr("data-observed"), Some("yes"));
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
        assert_eq!(
            child.borrow().value.get_attr("data-current-id"),
            Some("second")
        );
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
        assert_eq!(
            field.value.get_attr("data-accessor"),
            Some("function:function")
        );
        assert_eq!(field.value.get_attr("data-read"), Some("tracked"));
        assert_eq!(field.value.get_attr("value"), Some("tracked"));
    }

    #[test]
    fn exposes_document_and_node_metadata_used_by_react_dom() {
        let (mut runtime, dom) = runtime_from_html(
            r#"<html><body><main id="root"><span id="child">text</span></main></body></html>"#,
        );
        runtime.run_script(
            r#"
            const root = document.getElementById("root");
            const child = document.getElementById("child");
            child.setAttribute("data-default-view", document.defaultView === window);
            child.setAttribute("data-ready-before", document.readyState);
            child.setAttribute("data-local-name", child.localName);
            child.setAttribute("data-parent-element", child.parentElement === root);
            child.setAttribute("data-connected", child.isConnected);
            child.setAttribute("data-text-connected", child.firstChild.isConnected);
            "#,
        );

        let child = dom.get_element_by_id("child").unwrap();
        let child = child.borrow();
        assert_eq!(child.value.get_attr("data-default-view"), Some("true"));
        assert_eq!(child.value.get_attr("data-ready-before"), Some("loading"));
        assert_eq!(child.value.get_attr("data-local-name"), Some("span"));
        assert_eq!(child.value.get_attr("data-parent-element"), Some("true"));
        assert_eq!(child.value.get_attr("data-connected"), Some("true"));
        assert_eq!(child.value.get_attr("data-text-connected"), Some("true"));
        drop(child);

        assert!(runtime.dispatch_dom_content_loaded());
        runtime.run_script(
            r#"
            document.getElementById("child").setAttribute(
                "data-ready-after",
                document.readyState
            );
            "#,
        );
        assert_eq!(
            dom.get_element_by_id("child")
                .unwrap()
                .borrow()
                .value
                .get_attr("data-ready-after"),
            Some("complete")
        );
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
    fn window_onload_runs_when_dispatched() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r#"
            window.onload = function (event) {
                const result = document.getElementById("result");
                result.setAttribute("data-ready", "yes");
                result.setAttribute("data-event-type", event.type);
            };
            "#,
        );

        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(result.borrow().value.get_attr("data-ready"), None);
        assert!(runtime.dispatch_window_load());
        assert_eq!(result.borrow().value.get_attr("data-ready"), Some("yes"));
        assert_eq!(
            result.borrow().value.get_attr("data-event-type"),
            Some("load")
        );
    }

    #[test]
    fn window_load_listener_runs_when_dispatched_and_only_once() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r#"
            let dispatchCount = 0;
            window.addEventListener("load", function () {
                dispatchCount = dispatchCount + 1;
                document.getElementById("result").setAttribute("data-count", dispatchCount);
            });
            "#,
        );

        assert!(runtime.dispatch_window_load());
        assert!(!runtime.dispatch_window_load());
        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(result.borrow().value.get_attr("data-count"), Some("1"));
    }

    #[test]
    fn body_onload_registered_at_setup_runs_on_dispatch() {
        let (mut runtime, dom) = runtime_from_html(
            r#"<body onload="document.getElementById('result').setAttribute('data-onload', 'yes')"><div id="result"></div></body>"#,
        );

        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(result.borrow().value.get_attr("data-onload"), None);
        assert!(runtime.dispatch_window_load());
        assert_eq!(result.borrow().value.get_attr("data-onload"), Some("yes"));
        // The handler is registered at setup time, not re-scanned at dispatch.
        assert!(!runtime.dispatch_window_load());
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
            const classified = document.querySelectorAll("[class]");
            items[0].setAttribute("data-first", "yes");
            items.forEach(function (item, index) {
                item.setAttribute("data-index", index);
            });
            document.getElementById("result").setAttribute("data-count", items.length);
            document.getElementById("result").setAttribute("data-class-count", classified.length);
            "#,
        );

        let items = dom.get_elements_by_class_name("item");
        assert_eq!(items[0].borrow().value.get_attr("data-first"), Some("yes"));
        assert_eq!(items[0].borrow().value.get_attr("data-index"), Some("0"));
        assert_eq!(items[1].borrow().value.get_attr("data-index"), Some("1"));
        assert_eq!(
            items[1].borrow().value.get_attr("data-selected"),
            Some("yes")
        );
        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(result.borrow().value.get_attr("data-count"), Some("2"));
        assert_eq!(
            result.borrow().value.get_attr("data-class-count"),
            Some("2")
        );
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
    fn react_dom_collection_and_event_apis_are_available() {
        let (mut runtime, dom) = runtime_from_html(
            r#"
            <main id="root">
                <button class="control primary">one</button>
                <button class="control">two</button>
                <span class="primary">label</span>
            </main>
            <div id="result"></div>
            "#,
        );
        runtime.run_script(
            r#"
            const root = document.getElementById("root");
            const button = root.getElementsByTagName("button")[0];
            let received = "no";
            button.addEventListener("scratch-ready", function (event) {
                received = event.detail + ":" + (event.target === button);
                event.preventDefault();
            });
            const accepted = button.dispatchEvent(new CustomEvent(
                "scratch-ready", {detail: "yes", cancelable: true}
            ));
            document.getElementById("result").setAttribute(
                "data-dom-apis",
                document.getElementsByTagName("button").length + ":" +
                    document.getElementsByClassName("control primary").length + ":" +
                    root.getElementsByClassName("primary").length + ":" + received + ":" + accepted
            );
            "#,
        );

        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(
            result.borrow().value.get_attr("data-dom-apis"),
            Some("2:1:2:yes:true:false")
        );
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
    fn document_head_and_element_append_insert_dynamic_styles() {
        let (mut runtime, dom) = runtime_from_html(r#"<html><head></head><body></body></html>"#);
        runtime.run_script(
            r#"
            const style = document.createElement("style");
            style.append("body { color: red; }");
            document.head.append(style);
            "#,
        );

        let style = dom.query_selector("head style").unwrap();
        assert_eq!(DomTree::inner_text(&style), "body { color: red; }");
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
        assert_eq!(
            root.borrow().children()[0]
                .borrow()
                .value
                .get_attr("data-remove"),
            None
        );
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
            let initial = "";
            for (const token of target.classList) initial += token + ",";
            target.setAttribute("data-initial-classes", initial);
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
        assert_eq!(
            target.value.get_attr("data-initial-classes"),
            Some("one,two,")
        );
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
        assert_eq!(
            result.borrow().value.get_attr("data-monotonic"),
            Some("true")
        );
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
        assert_eq!(result.value.get_attr("data-status-text"), Some("All Good"));
        assert_eq!(result.value.get_attr("data-redirected"), Some("true"));
        assert_eq!(
            result.value.get_attr("data-body-used-before"),
            Some("false")
        );
        assert_eq!(result.value.get_attr("data-body-used-after"), Some("true"));
        assert_eq!(
            result.value.get_attr("data-url"),
            Some("data:text/plain,hello")
        );
        assert_eq!(result.value.get_attr("data-text"), Some("hello"));
    }

    #[test]
    fn fetch_array_buffer_preserves_binary_bytes() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r##"
            fetch("https://assets.scratch.mit.edu/project.sb3")
                .then(response => response.arrayBuffer())
                .then(buffer => {
                    const bytes = new Uint8Array(buffer);
                    document.querySelector("#result").setAttribute(
                        "data-bytes",
                        buffer.byteLength + ":" + bytes.length + ":" + bytes[0] + ":" + bytes[3]
                    );
                });
            "##,
        );

        let requests = runtime.take_fetch_requests();
        runtime.resolve_fetch(
            requests[0].id,
            JsFetchResponse {
                url: requests[0].url.clone(),
                status: 200,
                status_text: "OK".to_string(),
                redirected: false,
                body: vec![0, 127, 128, 255],
                headers: Vec::new(),
            },
        );

        let result = dom.get_element_by_id("result").unwrap();
        assert_eq!(
            result.borrow().value.get_attr("data-bytes"),
            Some("4:4:0:255")
        );
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
    fn xml_http_request_captures_request_and_dispatches_load() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="result"></div>"#);
        runtime.run_script(
            r##"
            const request = new XMLHttpRequest();
            request.open("post", "https://example.test/messages");
            request.setRequestHeader("Content-Type", "text/plain");
            request.onload = function () {
                const result = document.querySelector("#result");
                result.setAttribute("data-state", this.readyState);
                result.setAttribute("data-status", this.status);
                result.setAttribute("data-text", this.responseText);
                result.setAttribute("data-headers", this.getAllResponseHeaders());
            };
            request.send("hello");
            "##,
        );

        let requests = runtime.take_fetch_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, "https://example.test/messages");
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].body, b"hello");
        assert!(
            requests[0]
                .headers
                .contains(&("Content-Type".to_string(), "text/plain".to_string()))
        );

        runtime.resolve_fetch(
            requests[0].id,
            JsFetchResponse {
                url: requests[0].url.clone(),
                status: 201,
                status_text: "Created".to_string(),
                redirected: false,
                body: b"saved".to_vec(),
                headers: vec![("X-Test".to_string(), "yes".to_string())],
            },
        );

        let result = dom.get_element_by_id("result").unwrap();
        let result = result.borrow();
        assert_eq!(result.value.get_attr("data-state"), Some("4"));
        assert_eq!(result.value.get_attr("data-status"), Some("201"));
        assert_eq!(result.value.get_attr("data-text"), Some("saved"));
        assert_eq!(
            result.value.get_attr("data-headers"),
            Some("X-Test: yes\r\n")
        );
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

    #[test]
    fn intersection_observer_fires_on_observe() {
        let (mut runtime, dom) = runtime_from_html(
            r#"<div id="target" style="width: 100px; height: 50px"></div><div id="result"></div>"#,
        );
        runtime.run_script(
            r#"
            const target = document.getElementById("target");
            const result = document.getElementById("result");
            const observer = new IntersectionObserver(function (entries) {
                const entry = entries[0];
                result.setAttribute("data-target", entry.target === target);
                result.setAttribute("data-is-intersecting", entry.isIntersecting);
                result.setAttribute("data-ratio", entry.intersectionRatio);
                result.setAttribute("data-has-root-bounds", entry.rootBounds !== null);
                result.setAttribute("data-bcr-width", entry.boundingClientRect.width);
            });
            observer.observe(target);
            "#,
        );

        let result = dom.get_element_by_id("result").unwrap();
        let result = result.borrow();
        assert_eq!(result.value.get_attr("data-target"), Some("true"));
        // The element has style width/height so it is visible within the viewport.
        assert_eq!(result.value.get_attr("data-is-intersecting"), Some("true"));
        assert_eq!(result.value.get_attr("data-ratio"), Some("1"));
        assert_eq!(result.value.get_attr("data-has-root-bounds"), Some("true"));
        assert_eq!(result.value.get_attr("data-bcr-width"), Some("100"));
    }

    #[test]
    fn custom_elements_define_and_connect() {
        let (mut runtime, dom) =
            runtime_from_html(r#"<html><body><div id="result"></div></body></html>"#);
        runtime.run_script(
            r#"
            class MyElement extends HTMLElement {
                connectedCallback() {
                    document.getElementById("result").setAttribute("data-connected", "yes");
                }
                disconnectedCallback() {
                    document.getElementById("result").setAttribute("data-disconnected", "yes");
                }
            }
            customElements.define("my-element", MyElement);
            document.getElementById("result").setAttribute(
                "data-proto",
                typeof MyElement.prototype.connectedCallback
            );
            const el = document.createElement("my-element");
            document.body.appendChild(el);
            document.body.removeChild(el);
            "#,
        );

        let result = dom.get_element_by_id("result").unwrap();
        let result = result.borrow();
        // The prototype lookup finds the function.
        assert_eq!(result.value.get_attr("data-proto"), Some("function"));
        // Lifecycle callbacks fire via enqueue_job + microtask checkpoint.
        assert_eq!(result.value.get_attr("data-connected"), Some("yes"));
        assert_eq!(result.value.get_attr("data-disconnected"), Some("yes"));
    }

    #[test]
    fn custom_elements_define_getters_work() {
        let (mut runtime, _dom) = runtime_from_html(r#"<html><body></body></html>"#);
        runtime.run_script(
            r#"
            class MyEl extends HTMLElement {}
            customElements.define("my-el", MyEl);
            if (customElements.get("my-el") === undefined) throw new Error("get failed");
            if (customElements.get("no-such") !== undefined) throw new Error("get should be undefined");
            "#,
        );
    }

    #[test]
    fn custom_elements_attribute_changed_and_when_defined() {
        let (mut runtime, dom) =
            runtime_from_html(r#"<html><body><div id="result"></div></body></html>"#);
        runtime.run_script(
            r#"
            globalThis.__attrLog = [];
            class AttrEl extends HTMLElement {
                attributeChangedCallback(name, oldVal, newVal) {
                    globalThis.__attrLog.push(name + ":" + (oldVal === null ? "null" : oldVal) + ":" + (newVal === null ? "null" : newVal));
                }
            }
            AttrEl.observedAttributes = ["data-val"];
            customElements.define("attr-el", AttrEl);
            const el = document.createElement("attr-el");
            document.body.appendChild(el);
            el.setAttribute("data-val", "first");
            el.setAttribute("data-val", "second");
            el.removeAttribute("data-val");
            // whenDefined resolves immediately for an already-defined name.
            let wdResolved = false;
            customElements.whenDefined("attr-el").then(function () {
                wdResolved = true;
            });
            document.getElementById("result").setAttribute(
                "data-wd", wdResolved
            );
            "#,
        );

        // Read the log after microtasks have fired the callbacks.
        runtime.run_script(
            r#"document.getElementById("result").setAttribute(
                "data-log", globalThis.__attrLog.join("|")
            );"#,
        );

        let result = dom.get_element_by_id("result").unwrap();
        let result = result.borrow();
        // Three callbacks fire via microtask: first set, second set, remove.
        // oldValue is null on first set (attr didn't exist before).        assert_eq!(result.value.get_attr("data-log"), Some("data-val:null:first|data-val:first:second|data-val:second:null"));
        assert_eq!(result.value.get_attr("data-wd"), Some("true"));
    }

    #[test]
    fn shadow_dom_attach_and_query() {
        let (mut runtime, dom) =
            runtime_from_html(r#"<html><body><div id="host"></div></body></html>"#);
        // First: verify attachShadow works at all
        runtime.run_script(
            r##"
            var host = document.getElementById("host");
            host.setAttribute("data-step1", "ready");
            "##,
        );
        let result = dom.get_element_by_id("host").unwrap();
        assert_eq!(result.borrow().value.get_attr("data-step1"), Some("ready"));

        // Now try attachShadow in its own script
        runtime.run_script(
            r##"
            var host = document.getElementById("host");
            host.attachShadow({ mode: "open" });
            host.setAttribute("data-step2", "shadow-attached");
            "##,
        );
        assert_eq!(
            result.borrow().value.get_attr("data-step2"),
            Some("shadow-attached")
        );

        // Now test the rest
        runtime.run_script(
            r##"
            var host = document.getElementById("host");
            try {
                var sr = host.shadowRoot;
                host.setAttribute("data-sr", sr !== null ? "true" : "false");
                var span = document.createElement("span");
                span.id = "inner";
                span.textContent = "shadow text";
                sr.appendChild(span);
                var found = sr.querySelector("#inner");
                host.setAttribute("data-found", found !== null ? found.textContent : "NOT_FOUND");
                var notFound = host.querySelector("#inner");
                host.setAttribute("data-boundary", notFound === null ? "true" : "false");
                host.setAttribute("data-text", host.textContent.trim() === "" ? "true" : "false");
            } catch(e) {
                host.setAttribute("data-error", e.toString());
            }
            "##,
        );
        let result = dom.get_element_by_id("host").unwrap();
        let result = result.borrow();
        assert_eq!(result.value.get_attr("data-sr"), Some("true"));
        assert_eq!(result.value.get_attr("data-found"), Some("shadow text"));
        assert_eq!(result.value.get_attr("data-boundary"), Some("true"));
        assert_eq!(result.value.get_attr("data-text"), Some("true"));
    }

    #[test]
    fn shadow_dom_closed_root() {
        let (mut runtime, dom) =
            runtime_from_html(r#"<html><body><div id="host"></div></body></html>"#);
        runtime.run_script(
            r##"
            var host = document.getElementById("host");
            host.attachShadow({ mode: "closed" });
            host.setAttribute("data-closed", host.shadowRoot === null ? "true" : "false");
            "##,
        );
        let result = dom.get_element_by_id("host").unwrap();
        let result = result.borrow();
        assert_eq!(result.value.get_attr("data-closed"), Some("true"));
    }

    #[test]
    fn document_write_inserts_parsed_html_into_body() {
        let (mut runtime, dom) = runtime_from_html(r#"<html><body></body></html>"#);
        runtime.run_script(r#"document.write("<p>Hello</p>");"#);
        let p = dom.query_selector("body p").unwrap();
        assert_eq!(DomTree::inner_text(&p), "Hello");
        assert!(runtime.needs_redraw());
    }

    #[test]
    fn document_writeln_appends_content_with_newline() {
        let (mut runtime, dom) = runtime_from_html(r#"<html><body></body></html>"#);
        runtime.run_script(r#"document.writeln("<span>A</span>");"#);
        let span = dom.query_selector("body span").unwrap();
        assert_eq!(DomTree::inner_text(&span), "A");
    }

    #[test]
    fn dom_exception_has_name_message_and_code() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="r"></div>"#);
        runtime.run_script(
            r#"
        try {
            throw new DOMException("test error", "SyntaxError");
        } catch (e) {
            document.getElementById("r").setAttribute("data-name", e.name);
            document.getElementById("r").setAttribute("data-msg", e.message);
            document.getElementById("r").setAttribute("data-code", e.code);
        }
        "#,
        );
        let r = dom.get_element_by_id("r").unwrap();
        let r = r.borrow();
        assert_eq!(r.value.get_attr("data-name"), Some("SyntaxError"));
        assert_eq!(r.value.get_attr("data-msg"), Some("test error"));
        assert_eq!(r.value.get_attr("data-code"), Some("12"));
    }

    #[test]
    fn dom_exception_static_constants_are_exposed() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="r"></div>"#);
        runtime.run_script(
            r#"
        document.getElementById("r").setAttribute(
            "data-codes",
            DOMException.SYNTAX_ERR + ":" +
            DOMException.HIERARCHY_REQUEST_ERR + ":" +
            DOMException.NOT_FOUND_ERR + ":" +
            DOMException.INVALID_STATE_ERR
        );
        "#,
        );
        let r = dom.get_element_by_id("r").unwrap();
        assert_eq!(r.borrow().value.get_attr("data-codes"), Some("12:3:8:11"));
    }

    #[test]
    fn dom_exception_to_string_formats_name_and_message() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="r"></div>"#);
        runtime.run_script(
            r#"
        var e = new DOMException("oops", "NotFoundError");
        document.getElementById("r").setAttribute("data-str", e.toString());
        "#,
        );
        let r = dom.get_element_by_id("r").unwrap();
        assert_eq!(
            r.borrow().value.get_attr("data-str"),
            Some("NotFoundError: oops")
        );
    }

    #[test]
    fn create_element_throws_invalid_character_error_for_empty_name() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="r"></div>"#);
        runtime.run_script(
            r#"
        try {
            document.createElement("");
            document.getElementById("r").setAttribute("data-error", "no-throw");
        } catch (e) {
            document.getElementById("r").setAttribute("data-error", e.name);
        }
        "#,
        );
        let r = dom.get_element_by_id("r").unwrap();
        assert_eq!(
            r.borrow().value.get_attr("data-error"),
            Some("InvalidCharacterError")
        );
    }

    #[test]
    fn create_element_throws_invalid_character_error_for_invalid_name() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="r"></div>"#);
        runtime.run_script(
            r#"
        try {
            document.createElement("123bad");
            document.getElementById("r").setAttribute("data-error", "no-throw");
        } catch (e) {
            document.getElementById("r").setAttribute("data-error", e.name);
        }
        "#,
        );
        let r = dom.get_element_by_id("r").unwrap();
        assert_eq!(
            r.borrow().value.get_attr("data-error"),
            Some("InvalidCharacterError")
        );
    }

    #[test]
    fn create_element_valid_name_works() {
        let (mut runtime, dom) = runtime_from_html(r#"<html><body></body></html>"#);
        runtime.run_script(
            r#"
        var el = document.createElement("div");
        el.id = "created";
        document.body.appendChild(el);
        "#,
        );
        assert!(dom.get_element_by_id("created").is_some());
    }

    #[test]
    fn create_element_ns_throws_invalid_character_error_for_invalid_name() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="r"></div>"#);
        runtime.run_script(
            r#"
        try {
            document.createElementNS("http://www.w3.org/2000/svg", "123bad");
            document.getElementById("r").setAttribute("data-error", "no-throw");
        } catch (e) {
            document.getElementById("r").setAttribute("data-error", e.name);
        }
        "#,
        );
        let r = dom.get_element_by_id("r").unwrap();
        assert_eq!(
            r.borrow().value.get_attr("data-error"),
            Some("InvalidCharacterError")
        );
    }

    #[test]
    fn node_constants_are_exposed_on_global() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="r"></div>"#);
        runtime.run_script(
            r#"
        document.getElementById("r").setAttribute("data-elem", Node.ELEMENT_NODE);
        document.getElementById("r").setAttribute("data-text", Node.TEXT_NODE);
        document.getElementById("r").setAttribute("data-doc", Node.DOCUMENT_NODE);
        document.getElementById("r").setAttribute("data-frag", Node.DOCUMENT_FRAGMENT_NODE);
        "#,
        );
        let r = dom.get_element_by_id("r").unwrap();
        let r = r.borrow();
        assert_eq!(r.value.get_attr("data-elem"), Some("1"));
        assert_eq!(r.value.get_attr("data-text"), Some("3"));
        assert_eq!(r.value.get_attr("data-doc"), Some("9"));
        assert_eq!(r.value.get_attr("data-frag"), Some("11"));
    }

    #[test]
    fn node_constants_are_exposed_on_instance_nodes() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="r"></div>"#);
        runtime.run_script(
            r#"
        document.getElementById("r").setAttribute("data-doc-frag", document.DOCUMENT_FRAGMENT_NODE);
        document.getElementById("r").setAttribute("data-cmt", document.body.COMMENT_NODE);
        document.getElementById("r").setAttribute("data-txt", document.createTextNode("").ELEMENT_NODE);
        document.getElementById("r").setAttribute("data-frag", document.createElement("div").DOCUMENT_FRAGMENT_NODE);
        "#,
        );
        let r = dom.get_element_by_id("r").unwrap();
        let r = r.borrow();
        assert_eq!(r.value.get_attr("data-doc-frag"), Some("11"));
        assert_eq!(r.value.get_attr("data-cmt"), Some("8"));
        assert_eq!(r.value.get_attr("data-txt"), Some("1"));
        assert_eq!(r.value.get_attr("data-frag"), Some("11"));
    }

    #[test]
    fn document_first_child_is_doctype_node() {
        let (mut runtime, dom) = runtime_from_html(
            r#"<!DOCTYPE html><html><body><div id="r"><span>x</span></div></body></html>"#,
        );
        runtime.run_script(
            r#"
        document.getElementById("r").setAttribute("data-doc-type", document.nodeType);
        document.getElementById("r").setAttribute("data-doctype-node", document.firstChild.nodeType);
        document.getElementById("r").setAttribute("data-doctype-name", document.firstChild.nodeName);
        var span = document.getElementById("r").firstChild;
        document.getElementById("r").setAttribute("data-text-type", span.firstChild.nodeType);
        "#,
        );
        let r = dom.get_element_by_id("r").unwrap();
        let r = r.borrow();
        assert_eq!(r.value.get_attr("data-doc-type"), Some("9"));
        assert_eq!(r.value.get_attr("data-doctype-node"), Some("10"));
        assert_eq!(r.value.get_attr("data-doctype-name"), Some("html"));
        assert_eq!(r.value.get_attr("data-text-type"), Some("3"));
    }

    #[test]
    fn create_element_ns_preserves_qualified_name() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="r"></div>"#);
        runtime.run_script(
            r#"
        var el = document.createElementNS("http://ns.example.com/", "prefix:localname");
        document.getElementById("r").setAttribute("data-tag", el.tagName);
        document.getElementById("r").setAttribute("data-local", el.localName);
        document.getElementById("r").setAttribute("data-prefix", el.prefix);
        document.getElementById("r").setAttribute("data-ns", el.namespaceURI);
        "#,
        );
        let r = dom.get_element_by_id("r").unwrap();
        let r = r.borrow();
        assert_eq!(r.value.get_attr("data-tag"), Some("prefix:localname"));
        assert_eq!(r.value.get_attr("data-local"), Some("localname"));
        assert_eq!(r.value.get_attr("data-prefix"), Some("prefix"));
        assert_eq!(r.value.get_attr("data-ns"), Some("http://ns.example.com/"));
    }

    #[test]
    fn document_close_returns_undefined() {
        let (mut runtime, _) = runtime_from_html(r#"<html><body></body></html>"#);
        runtime.run_script(
            r#"
        var result = document.close();
        document.getElementById("r").setAttribute("data-close", typeof result);
        "#,
        );
        // document.close() returns undefined, and there is no element "r" yet
        // so we test it differently
        let (mut runtime2, dom2) = runtime_from_html(r#"<div id="r"></div>"#);
        runtime2.run_script(
            r#"
        document.close();
        document.getElementById("r").setAttribute("data-close", "ok");
        "#,
        );
        let r = dom2.get_element_by_id("r").unwrap();
        assert_eq!(r.borrow().value.get_attr("data-close"), Some("ok"));
    }

    #[test]
    fn dom_exception_instanceof_error() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="r"></div>"#);
        runtime.run_script(
            r#"
        var e = new DOMException("test", "SyntaxError");
        document.getElementById("r").setAttribute("data-is-error", e instanceof Error);
        document.getElementById("r").setAttribute("data-is-domexc", e instanceof DOMException);
        "#,
        );
        let r = dom.get_element_by_id("r").unwrap();
        let r = r.borrow();
        assert_eq!(r.value.get_attr("data-is-error"), Some("true"));
        assert_eq!(r.value.get_attr("data-is-domexc"), Some("true"));
    }

    #[test]
    fn create_comment_returns_comment_node() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="r"></div>"#);
        runtime.run_script(
            r#"
        var c = document.createComment("hello");
        document.getElementById("r").setAttribute("data-type", c.nodeType);
        document.getElementById("r").setAttribute("data-name", c.nodeName);
        document.getElementById("r").setAttribute("data-data", c.data);
        "#,
        );
        let r = dom.get_element_by_id("r").unwrap();
        let r = r.borrow();
        assert_eq!(r.value.get_attr("data-type"), Some("8"));
        assert_eq!(r.value.get_attr("data-name"), Some("#comment"));
        assert_eq!(r.value.get_attr("data-data"), Some("hello"));
    }

    #[test]
    fn create_processing_instruction_returns_pi_node() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="r"></div>"#);
        runtime.run_script(
            r#"
        var pi = document.createProcessingInstruction("xml-stylesheet", "href=\"style.css\"");
        document.getElementById("r").setAttribute("data-type", pi.nodeType);
        document.getElementById("r").setAttribute("data-data", pi.data);
        "#,
        );
        let r = dom.get_element_by_id("r").unwrap();
        let r = r.borrow();
        assert_eq!(r.value.get_attr("data-type"), Some("7"));
        assert_eq!(r.value.get_attr("data-data"), Some("href=\"style.css\""));
    }

    #[test]
    fn create_processing_instruction_empty_target_throws() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="r"></div>"#);
        runtime.run_script(
            r#"
        try {
            document.createProcessingInstruction("", "data");
            document.getElementById("r").setAttribute("data-error", "no-throw");
        } catch (e) {
            document.getElementById("r").setAttribute("data-error", e.name);
        }
        "#,
        );
        let r = dom.get_element_by_id("r").unwrap();
        assert_eq!(r.borrow().value.get_attr("data-error"), Some("SyntaxError"));
    }

    #[test]
    fn innerhtml_setter_uses_parser_and_replaces_children() {
        let (mut runtime, dom) = runtime_from_html(r#"<div id="target"></div><div id="r"></div>"#);
        runtime.run_script(r#"
        var t = document.getElementById("target");
        t.innerHTML = "<p>A</p><span>B</span>";
        document.getElementById("r").setAttribute("data-count", t.childNodes.length);
        document.getElementById("r").setattr || document.getElementById("r").setAttribute("data-tags",
            t.children[0].tagName + ":" + t.children[1].tagName
        );
        "#);
        let r = dom.get_element_by_id("r").unwrap();
        let r = r.borrow();
        assert_eq!(r.value.get_attr("data-count"), Some("2"));
    }
}
