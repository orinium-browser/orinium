//! Minimal JS runtime backed by `pixi_byte`.
//!
//! Hosts the JS engine on the UI thread and installs a small set of DOM
//! bindings (`console`, `document.getElementById`, element properties).
//! The engine never imports `platform`; DOM access goes through the shared
//! host slot that `JsRuntime` registers on the VM.

use crate::engine::html::{DomTree, HtmlNodeType};
use crate::engine::tree::NodeRef;
use pixi_byte::value::jsobject::{JSObject, Property};
use pixi_byte::vm::VM;
use pixi_byte::{JSResult, JSValue};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

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
            next_id: 0,
            needs_redraw: Rc::clone(&needs_redraw),
        }));

        let mut engine = pixi_byte::JSEngine::new();
        engine.set_host(host);

        install_console(&mut engine);
        install_document(&mut engine);

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
    }

    /// Returns whether a script mutated the DOM and a relayout is needed.
    pub fn needs_redraw(&self) -> bool {
        self.needs_redraw.get()
    }

    /// Clears and returns the redraw flag.
    pub fn take_needs_redraw(&self) -> bool {
        self.needs_redraw.replace(false)
    }

    /// Dispatches a click to the `onclick` handler of `node`, if one was
    /// registered on its JS element object.
    ///
    /// Returns whether the handler ran (regardless of DOM mutation).
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
        if !matches!(
            onclick,
            JSValue::Function(..) | JSValue::NativeFunction(_) | JSValue::BoundFunction(..)
        ) {
            return false;
        }
        if let Err(err) = self.engine.call(onclick, JSValue::Object(obj), Vec::new()) {
            log::info!("JS error on click: {}", err);
        }
        true
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
fn element_dom_id(this: &JSValue) -> Option<u64> {
    let JSValue::Object(obj) = this else {
        return None;
    };
    let JSValue::Number(n) = obj.borrow().get("__orinium_dom_id") else {
        return None;
    };
    Some(n as u64)
}

/// Resolves the `this` element back to a live DOM node (dead node -> None).
fn element_node(vm: &VM, this: &JSValue) -> Option<NodeRef<HtmlNodeType>> {
    let dom_id = element_dom_id(this)?;
    with_host(vm, |host| host.refs.get(&dom_id).and_then(|w| w.upgrade())).flatten()
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

// --- document ---

fn install_document(engine: &mut pixi_byte::JSEngine) {
    let document_obj = Rc::new(RefCell::new(JSObject::new()));
    {
        let mut document = document_obj.borrow_mut();
        document.set(
            "getElementById".to_string(),
            JSValue::NativeFunction(get_element_by_id),
        );
    }
    engine
        .global_mut()
        .borrow_mut()
        .set("document".to_string(), JSValue::Object(document_obj));
}

fn get_element_by_id(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(JSValue::String(id)) = args.get(1) else {
        return Ok(JSValue::Null);
    };

    // Look up the node and capture its static attributes first.
    let Some((node, tag_name, attr_id)) = with_host(vm, |host| {
        host.dom.get_element_by_id(id).map(|node| {
            let (tag_name, attr_id) = {
                let n = node.borrow();
                (
                    n.value.tag_name().unwrap_or("").to_string(),
                    n.value.get_attr("id").unwrap_or("").to_string(),
                )
            };
            (node, tag_name, attr_id)
        })
    })
    .flatten() else {
        return Ok(JSValue::Null);
    };

    // Register the live node so later property access can resolve it. Reuse
    // the existing id and element object when this node was already exposed.
    let Some(dom_id) = with_host_mut(vm, |host| {
        if let Some(dom_id) = host.dom_id_for_node(&node) {
            return Some(dom_id);
        }
        host.next_id += 1;
        let dom_id = host.next_id;
        host.refs.insert(dom_id, Rc::downgrade(&node));
        Some(dom_id)
    })
    .flatten() else {
        return Ok(JSValue::Null);
    };

    let obj = with_host_mut(vm, |host| {
        if let Some(existing) = host.objects.get(&dom_id) {
            return Rc::clone(existing);
        }
        let obj = make_element(tag_name, attr_id, dom_id);
        host.objects.insert(dom_id, Rc::clone(&obj));
        obj
    })
    .expect("host must be present");

    Ok(JSValue::Object(obj))
}

// --- Element ---

fn make_element(tag_name: String, attr_id: String, dom_id: u64) -> Rc<RefCell<JSObject>> {
    let mut obj = JSObject::new();
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
    obj.set(
        "getAttribute".to_string(),
        JSValue::NativeFunction(get_attribute),
    );
    obj.set(
        "setAttribute".to_string(),
        JSValue::NativeFunction(set_attribute),
    );
    Rc::new(RefCell::new(obj))
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

fn get_text_content(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = element_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
        return Ok(JSValue::Null);
    };
    Ok(JSValue::String(DomTree::inner_text(&node)))
}

fn set_text_content(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = element_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
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
    let Some(node) = element_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
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
    let Some(node) = element_node(vm, args.first().unwrap_or(&JSValue::Undefined)) else {
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
}
