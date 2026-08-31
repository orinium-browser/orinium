//! `customElements` Web API — registers and instantiates custom element classes.
//!
//! Supports `define`, `get`, `upgrade`, and `whenDefined`.  Lifecycle callbacks
//! (`connectedCallback`, `disconnectedCallback`, `attributeChangedCallback`) are
//! dispatched when elements are added/removed from the DOM or have observed
//! attributes changed.

use crate::engine::js::CustomElementDefinition;
use crate::engine::js::common::{is_callable, with_host, with_host_mut};
use pixi_byte::value::jsobject::JSObject;
use pixi_byte::vm::VM;
use pixi_byte::{JSError, JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;

// Cached JS helper for reading prototype callbacks from class constructors.
// pixi_byte class constructors are `Function` values (not `JSObject`s), so
// their `.prototype` is not accessible from native code.  The helper does
// `c.prototype[n]` — the VM resolves property access through its internal
// `user_function_object` map.
thread_local! {
    static CE_READ_CB: RefCell<Option<JSValue>> = const { RefCell::new(None) };
}

pub(crate) fn install_custom_elements(engine: &mut pixi_byte::JSEngine) {
    // Define the JS helper in the global scope.  Must run after
    // `install_global_aliases` so `globalThis` exists.
    let _ = engine.eval(
        "var __ce_read_cb = function(c, n) { \
            if (c && c[n] !== undefined) return c[n]; \
            var p = c && c.prototype; \
            return (p && p[n] !== undefined) ? p[n] : undefined; \
        }",
    );
    if let Ok(val) = engine.eval("__ce_read_cb") {
        CE_READ_CB.with(|cell| *cell.borrow_mut() = Some(val));
    }

    let mut custom_elements = JSObject::new();
    custom_elements.set(
        "define".to_string(),
        JSValue::from_native_function(custom_elements_define),
    );
    custom_elements.set(
        "get".to_string(),
        JSValue::from_native_function(custom_elements_get),
    );
    custom_elements.set(
        "upgrade".to_string(),
        JSValue::from_native_function(custom_elements_upgrade),
    );
    custom_elements.set(
        "whenDefined".to_string(),
        JSValue::from_native_function(custom_elements_when_defined),
    );
    engine.global_mut().borrow_mut().set(
        "customElements".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(custom_elements))),
    );
}

// ---------------------------------------------------------------------------
// `customElements.define(name, constructor, options?)`
// ---------------------------------------------------------------------------

fn custom_elements_define(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(name) = args.get(1).and_then(JSValue::as_string) else {
        return Err(JSError::TypeError(
            "customElements.define requires a name string".to_string(),
        ));
    };
    let name = name.trim().to_ascii_lowercase();
    if !name.contains('-') {
        return Err(JSError::TypeError(
            "Custom element name must contain a hyphen".to_string(),
        ));
    }
    let Some(constructor) = args.get(2).cloned() else {
        return Err(JSError::TypeError(
            "customElements.define requires a constructor".to_string(),
        ));
    };
    if !is_callable(&constructor) {
        return Err(JSError::TypeError(
            "customElements.define constructor must be callable".to_string(),
        ));
    }

    let connected_callback = get_prototype_callback(vm, &constructor, "connectedCallback");
    let disconnected_callback = get_prototype_callback(vm, &constructor, "disconnectedCallback");
    let attribute_changed_callback =
        get_prototype_callback(vm, &constructor, "attributeChangedCallback");
    let observed_attributes = get_observed_attributes(vm, &constructor);

    let definition = CustomElementDefinition {
        constructor: constructor.clone(),
        connected_callback,
        disconnected_callback,
        attribute_changed_callback,
        observed_attributes,
        when_defined_resolvers: Vec::new(),
    };

    with_host_mut(vm, |host| {
        host.custom_elements.insert(name.clone(), definition);
    });

    // Upgrade existing elements with this tag name.
    let dom_ids: Vec<u64> = with_host(vm, |host| {
        host.refs
            .iter()
            .filter_map(|(&dom_id, weak)| {
                let node = weak.upgrade()?;
                let tag = node.borrow().value.tag_name()?.to_ascii_lowercase();
                if tag == name { Some(dom_id) } else { None }
            })
            .collect()
    })
    .unwrap_or_default();

    for dom_id in &dom_ids {
        fire_lifecycle_callback(vm, *dom_id, |d| d.connected_callback.clone());
    }

    // Resolve pending `whenDefined()` promises.
    let resolvers: Vec<JSValue> = with_host_mut(vm, |host| {
        if let Some(def) = host.custom_elements.get_mut(&name) {
            std::mem::take(&mut def.when_defined_resolvers)
        } else {
            Vec::new()
        }
    })
    .unwrap_or_default();

    for resolver in resolvers {
        let _ = vm.call(resolver, JSValue::undefined(), vec![]);
    }

    Ok(JSValue::undefined())
}

// ---------------------------------------------------------------------------
// `customElements.get(name)`
// ---------------------------------------------------------------------------

fn custom_elements_get(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(name) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(JSValue::undefined());
    };
    let name = name.trim().to_ascii_lowercase();
    with_host(vm, |host| {
        host.custom_elements
            .get(&name)
            .map(|def| def.constructor.clone())
    })
    .flatten()
    .ok_or_else(|| JSError::TypeError("customElements.get: definition not found".to_string()))
    .or(Ok(JSValue::undefined()))
}

// ---------------------------------------------------------------------------
// `customElements.upgrade(root)`
// ---------------------------------------------------------------------------

fn custom_elements_upgrade(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let root_value = args.get(1).cloned().unwrap_or(JSValue::undefined());

    // Collect all DOM ids of elements that have a registered custom element
    // tag.  If a root argument was given, restrict to descendants of that root.
    let dom_ids: Vec<u64> = with_host(vm, |host| {
        let root_dom_id = crate::engine::js::common::node_dom_id(&root_value);

        host.refs
            .iter()
            .filter_map(|(&dom_id, weak)| {
                let node = weak.upgrade()?;
                let tag = node.borrow().value.tag_name()?.to_ascii_lowercase();
                if !host.custom_elements.contains_key(&tag) {
                    return None;
                }
                // If a root was specified, check that the element is a
                // descendant of the root (or is the root itself).
                if let Some(root_id) = root_dom_id {
                    if dom_id == root_id {
                        return Some(dom_id);
                    }
                    // Walk up from this node to see if we reach the root.
                    let mut current = Some(Rc::clone(&node));
                    let mut found = false;
                    while let Some(n) = current {
                        if let Some(n_id) = host.dom_id_for_node(&n)
                            && n_id == root_id
                        {
                            found = true;
                            break;
                        }
                        current = n.borrow().parent();
                    }
                    if found { Some(dom_id) } else { None }
                } else {
                    Some(dom_id)
                }
            })
            .collect()
    })
    .unwrap_or_default();

    for dom_id in &dom_ids {
        fire_lifecycle_callback(vm, *dom_id, |d| d.connected_callback.clone());
    }

    Ok(JSValue::undefined())
}

// ---------------------------------------------------------------------------
// `customElements.whenDefined(name)`
// ---------------------------------------------------------------------------

fn custom_elements_when_defined(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(name) = args.get(1).and_then(JSValue::as_string) else {
        return Err(JSError::TypeError(
            "customElements.whenDefined requires a name string".to_string(),
        ));
    };
    let name = name.trim().to_ascii_lowercase();
    let is_defined =
        with_host(vm, |host| host.custom_elements.contains_key(&name)).unwrap_or(false);

    if is_defined {
        return Ok(make_resolved_promise_stub());
    }

    // Create a thenable promise.  The `then` method stores the
    // onFulfilled callback in the definition's pending queue so that
    // `define()` can invoke it when the name is registered.
    //
    // We store the element name as a property on the promise object so
    // the native `then` function can read it from `this`.
    let mut promise = JSObject::new();
    promise.set("__ce_name__".to_string(), JSValue::from_string(name));
    promise.set(
        "then".to_string(),
        JSValue::from_native_function(when_defined_then),
    );

    Ok(JSValue::from_object(Rc::new(RefCell::new(promise))))
}

fn make_resolved_promise_stub() -> JSValue {
    let mut promise = JSObject::new();
    promise.set(
        "then".to_string(),
        JSValue::from_native_function(then_invoke),
    );
    JSValue::from_object(Rc::new(RefCell::new(promise)))
}

/// Resolved-promise `then`: immediately invokes the onFulfilled callback.
fn then_invoke(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let callback = args.get(1).cloned().unwrap_or(JSValue::undefined());
    if is_callable(&callback) {
        let _ = vm.call(callback, JSValue::undefined(), vec![]);
    }
    Ok(JSValue::undefined())
}

/// Native function for `whenDefined().then(callback)`.  Reads the element
/// name from `this.__ce_name__` and stores the callback in the definition's
/// pending queue.
fn when_defined_then(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let this = args.first().cloned().unwrap_or(JSValue::undefined());
    let callback = args.get(1).cloned().unwrap_or(JSValue::undefined());
    if !is_callable(&callback) {
        return Ok(JSValue::undefined());
    }
    let name = if let Some(obj) = this.as_object() {
        obj.borrow()
            .get("__ce_name__")
            .as_string()
            .unwrap_or("")
            .to_string()
    } else {
        return Ok(JSValue::undefined());
    };
    if name.is_empty() {
        return Ok(JSValue::undefined());
    }
    // If the name is already defined, invoke the callback immediately
    // (mimicking a resolved promise).
    let already_defined =
        with_host(vm, |host| host.custom_elements.contains_key(&name)).unwrap_or(false);
    if already_defined {
        let _ = vm.call(callback, JSValue::undefined(), vec![]);
        return Ok(JSValue::undefined());
    }
    // Otherwise, store for later resolution by define().
    with_host_mut(vm, |host| {
        if let Some(def) = host.custom_elements.get_mut(&name) {
            def.when_defined_resolvers.push(callback);
        }
    });
    Ok(JSValue::undefined())
}

// ---------------------------------------------------------------------------
// Prototype callback extraction
// ---------------------------------------------------------------------------

/// Reads a lifecycle callback from the constructor's `prototype` property.
///
/// For JSObject constructors (e.g. `HTMLElement`), reads directly.
/// For `class` constructors (Function values), uses the cached JS helper
/// `__ce_read_cb(constructor, propName)` which the VM resolves through
/// its internal `user_function_object` mechanism.
fn get_prototype_callback(vm: &mut VM, constructor: &JSValue, prop_name: &str) -> Option<JSValue> {
    // Fast path: JSObject constructors (e.g. HTMLElement).
    if let Some(obj) = constructor.as_object() {
        let prototype_value = obj.borrow().get("prototype");
        if let Some(prototype) = prototype_value.as_object() {
            let value = prototype.borrow().get(prop_name);
            if is_callable(&value) {
                return Some(value);
            }
        }
    }
    // Slow path: class constructors (Function values).
    let helper_fn = CE_READ_CB.with(|cell| cell.borrow().clone())?;
    let result = vm.call(
        helper_fn,
        JSValue::undefined(),
        vec![
            constructor.clone(),
            JSValue::from_string(prop_name.to_string()),
        ],
    );
    match result {
        Ok(value) if is_callable(&value) => Some(value),
        _ => None,
    }
}

/// Reads the `observedAttributes` static property from a constructor.
/// For class constructors, this is a `static` property on the constructor
/// itself (not on the prototype).
fn get_observed_attributes(vm: &mut VM, constructor: &JSValue) -> Vec<String> {
    // Fast path: JSObject constructors — read directly.
    if let Some(obj) = constructor.as_object() {
        let attrs_value = obj.borrow().get("observedAttributes");
        let result = extract_string_array(&attrs_value);
        if !result.is_empty() {
            return result;
        }
    }
    // Slow path: class constructors (Function values).
    // The JS helper reads `c[n]` first (static properties), then
    // `c.prototype[n]` (instance methods).
    let helper_fn = match CE_READ_CB.with(|cell| cell.borrow().clone()) {
        Some(f) => f,
        None => return Vec::new(),
    };
    let result = vm.call(
        helper_fn,
        JSValue::undefined(),
        vec![
            constructor.clone(),
            JSValue::from_string("observedAttributes".to_string()),
        ],
    );
    match result {
        Ok(value) => extract_string_array(&value),
        _ => Vec::new(),
    }
}

fn extract_string_array(value: &JSValue) -> Vec<String> {
    let Some(obj) = value.as_object() else {
        return Vec::new();
    };
    let obj_ref = obj.borrow();
    let len = obj_ref.get("length").as_number().unwrap_or(0.0) as usize;
    (0..len)
        .filter_map(|i| {
            let key = i.to_string();
            let val = obj_ref.get(&key);
            val.as_string().map(str::to_string)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Lifecycle callback dispatch (called from element.rs)
// ---------------------------------------------------------------------------

/// Dispatches `connectedCallback` on a custom element after it is connected.
pub(crate) fn fire_connected_callback(vm: &mut VM, dom_id: u64) {
    fire_lifecycle_callback(vm, dom_id, |d| d.connected_callback.clone());
}

/// Dispatches `disconnectedCallback` on a custom element after removal.
pub(crate) fn fire_disconnected_callback(vm: &mut VM, dom_id: u64) {
    fire_lifecycle_callback(vm, dom_id, |d| d.disconnected_callback.clone());
}

/// Generic lifecycle callback dispatcher using stored callbacks from the
/// custom element definition.
fn fire_lifecycle_callback(
    vm: &mut VM,
    dom_id: u64,
    extract: impl FnOnce(&CustomElementDefinition) -> Option<JSValue>,
) {
    let tag_name = with_host(vm, |host| {
        host.refs
            .get(&dom_id)
            .and_then(|w| w.upgrade())
            .and_then(|node| node.borrow().value.tag_name().map(str::to_ascii_lowercase))
    })
    .flatten();

    let Some(tag_name) = tag_name else { return };

    let callback_and_obj = with_host(vm, |host| {
        let definition = host.custom_elements.get(&tag_name)?;
        let callback = extract(definition)?;
        let element_obj = host.objects.get(&dom_id).cloned()?;
        Some((callback, element_obj))
    })
    .flatten();

    let Some((callback, element_obj)) = callback_and_obj else {
        return;
    };

    if let Err(err) = vm.call(
        callback,
        JSValue::from_object(Rc::clone(&element_obj)),
        vec![],
    ) {
        log::info!("JS error in lifecycle callback: {}", err);
    }
}

/// Dispatches `attributeChangedCallback` on a custom element when an observed
/// attribute is set or removed.  Per spec, `oldValue` is `null` when the
/// attribute is set for the first time.
pub(crate) fn fire_attribute_changed_callback(
    vm: &mut VM,
    dom_id: u64,
    attribute_name: &str,
    old_value: Option<&str>,
    new_value: Option<&str>,
) {
    let tag_name = with_host(vm, |host| {
        host.refs
            .get(&dom_id)
            .and_then(|w| w.upgrade())
            .and_then(|node| node.borrow().value.tag_name().map(str::to_ascii_lowercase))
    })
    .flatten();

    let Some(tag_name) = tag_name else { return };
    let definition = with_host(vm, |host| host.custom_elements.get(&tag_name).cloned()).flatten();
    let Some(definition) = definition else {
        return;
    };

    // Only fire if the attribute is in observedAttributes.
    if !definition
        .observed_attributes
        .iter()
        .any(|a| a == attribute_name)
    {
        return;
    }

    let Some(callback) = definition.attribute_changed_callback else {
        return;
    };

    let element_obj = with_host(vm, |host| host.objects.get(&dom_id).cloned()).flatten();
    let Some(element_obj) = element_obj else {
        return;
    };

    // Per spec: oldValue is null (not empty string) when the attribute didn't
    // exist before the call.
    let old_js = match old_value {
        Some(v) => JSValue::from_string(v.to_string()),
        None => JSValue::null(),
    };
    let new_js = match new_value {
        Some(v) => JSValue::from_string(v.to_string()),
        None => JSValue::null(),
    };

    vm.enqueue_job(
        callback,
        JSValue::from_object(Rc::clone(&element_obj)),
        vec![
            JSValue::from_string(attribute_name.to_string()),
            old_js,
            new_js,
        ],
    );
}
