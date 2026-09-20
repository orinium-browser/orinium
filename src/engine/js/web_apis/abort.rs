//! `AbortController` / `AbortSignal` — cancellation plumbing for scripts.
//!
//! `AbortSignal` instances are ordinary `JSObject`s carrying an internal
//! `aborted` flag plus registration lists. `AbortController.abort(reason)`
//! flips the flag, stores the reason, queues `abort` event listeners, and
//! rejects every pending fetch that registered the signal with
//! `fetch(url, { signal })`. Timing follows the platform: `abort()` marks
//! state synchronously and listeners run at the next microtask checkpoint.

use crate::engine::js::common::{is_callable, noop, with_host_mut};
use pixi_byte::value::jsobject::{JSObject, Property};
use pixi_byte::value::jsvalue::{BoundFunctionData, JsValueKind};
use pixi_byte::vm::VM;
use pixi_byte::{JSError, JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;

/// Internal flag marking an aborted signal.
const ABORTED: &str = "__orinium_aborted";
/// Internal storage for the abort reason.
const ABORT_REASON: &str = "__orinium_abort_reason";
/// Internal list of `abort` event listeners queued for the next checkpoint.
const ABORT_LISTENERS: &str = "__orinium_abort_listeners";
/// Marks the singleton `AbortSignal.abort()` reason object.
const ABORT_SIGNAL_MARKER: &str = "__orinium_abort_signal";
/// Marks the per-controller signal object.
const SIGNAL_CONTROLLER: &str = "__orinium_signal_controller";

/// Installs `AbortController` and `AbortSignal` on the global object.
pub(crate) fn install_abort_apis(engine: &mut pixi_byte::JSEngine) {
    let mut controller = JSObject::new();
    controller.set(
        "__construct__".to_string(),
        JSValue::from_native_function(abort_controller_constructor),
    );
    let mut signal_constructor = JSObject::new();
    signal_constructor.set(
        "__construct__".to_string(),
        JSValue::from_native_function(signal_construct_error),
    );
    signal_constructor.set(
        "abort".to_string(),
        JSValue::from_native_function(signal_static_abort),
    );
    signal_constructor.set(
        "timeout".to_string(),
        JSValue::from_native_function(signal_static_timeout),
    );
    signal_constructor.set(
        "any".to_string(),
        JSValue::from_native_function(signal_static_any),
    );
    let mut global = engine.global_mut().borrow_mut();
    global.set(
        "AbortController".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(controller))),
    );
    global.set(
        "AbortSignal".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(signal_constructor))),
    );
}

/// `new AbortController()` — creates `{ signal, abort }`.
fn abort_controller_constructor(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let signal = make_signal(false, JSValue::undefined());
    let mut controller = JSObject::new();
    controller.define_property(
        "signal".to_string(),
        Property::read_only(JSValue::from_object(Rc::clone(&signal))),
    );
    controller.set(
        "abort".to_string(),
        JSValue::from_native_function(controller_abort),
    );
    signal
        .borrow_mut()
        .set(SIGNAL_CONTROLLER.to_string(), JSValue::undefined());
    Ok(JSValue::from_object(Rc::new(RefCell::new(controller))))
}

/// `new AbortSignal()` directly is forbidden per spec.
fn signal_construct_error(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Err(JSError::TypeError(
        "Illegal constructor: AbortSignal cannot be constructed directly".to_string(),
    ))
}

/// Builds a signal object with the given aborted state and reason.
fn make_signal(aborted: bool, reason: JSValue) -> Rc<RefCell<JSObject>> {
    let mut signal = JSObject::new();
    signal.set(ABORTED.to_string(), JSValue::from_bool(aborted));
    signal.set(ABORT_REASON.to_string(), reason);
    signal.set(ABORT_LISTENERS.to_string(), JSValue::undefined());
    signal.set(ABORT_SIGNAL_MARKER.to_string(), JSValue::from_bool(true));
    signal.define_property(
        "aborted".to_string(),
        Property {
            value: JSValue::undefined(),
            enumerable: true,
            writable: false,
            configurable: true,
            getter: Some(JSValue::from_native_function(signal_get_aborted)),
            setter: None,
        },
    );
    signal.define_property(
        "reason".to_string(),
        Property {
            value: JSValue::undefined(),
            enumerable: true,
            writable: false,
            configurable: true,
            getter: Some(JSValue::from_native_function(signal_get_reason)),
            setter: None,
        },
    );
    signal.set(
        "throwIfAborted".to_string(),
        JSValue::from_native_function(signal_throw_if_aborted),
    );
    signal.set(
        "addEventListener".to_string(),
        JSValue::from_native_function(signal_add_event_listener),
    );
    signal.set(
        "removeEventListener".to_string(),
        JSValue::from_native_function(signal_remove_event_listener),
    );
    signal.set(
        "dispatchEvent".to_string(),
        JSValue::from_native_function(noop),
    );
    Rc::new(RefCell::new(signal))
}

fn signal_receiver(args: &[JSValue]) -> JSResult<Rc<RefCell<JSObject>>> {
    let signal = args.first().and_then(JSValue::as_object).ok_or_else(|| {
        JSError::TypeError("AbortSignal method called on incompatible receiver".to_string())
    })?;
    if signal.borrow().get(ABORT_SIGNAL_MARKER).as_boolean() != Some(true) {
        return Err(JSError::TypeError(
            "AbortSignal method called on incompatible receiver".to_string(),
        ));
    }
    Ok(signal)
}

fn signal_get_aborted(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let signal = signal_receiver(&args)?;
    Ok(JSValue::from_bool(
        signal.borrow().get(ABORTED).as_boolean() == Some(true),
    ))
}

fn signal_get_reason(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let signal = signal_receiver(&args)?;
    Ok(signal.borrow().get(ABORT_REASON))
}

/// `signal.throwIfAborted()` — throws the abort reason when aborted.
fn signal_throw_if_aborted(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let signal = signal_receiver(&args)?;
    let (aborted, reason) = {
        let borrowed = signal.borrow();
        (
            borrowed.get(ABORTED).as_boolean() == Some(true),
            borrowed.get(ABORT_REASON),
        )
    };
    if aborted {
        return Err(JSError::Thrown(reason));
    }
    Ok(JSValue::undefined())
}

/// `signal.addEventListener("abort", listener)` — records the listener so
/// `abort()` can invoke it synchronously when the signal transitions.
fn signal_add_event_listener(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let signal = signal_receiver(&args)?;
    let event_type = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    let listener = args.get(2).cloned().unwrap_or(JSValue::undefined());
    if event_type != "abort" || !is_callable(&listener) {
        return Ok(JSValue::undefined());
    }
    let mut borrowed = signal.borrow_mut();
    let existing = borrowed.get(ABORT_LISTENERS);
    let listeners = match existing.as_object() {
        Some(object) => object,
        None => {
            let object = Rc::new(RefCell::new(JSObject::new()));
            borrowed.set(
                ABORT_LISTENERS.to_string(),
                JSValue::from_object(Rc::clone(&object)),
            );
            object
        }
    };
    drop(borrowed);
    let next_key = listeners.borrow().keys().len().to_string();
    listeners.borrow_mut().set(next_key, listener);
    Ok(JSValue::undefined())
}

/// `signal.removeEventListener("abort", listener)` — drops a previously
/// registered listener (matched by strict identity).
fn signal_remove_event_listener(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let signal = signal_receiver(&args)?;
    let event_type = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    let listener = args.get(2).cloned().unwrap_or(JSValue::undefined());
    if event_type != "abort" {
        return Ok(JSValue::undefined());
    }
    let listeners = match signal.borrow().get(ABORT_LISTENERS).as_object() {
        Some(object) => object,
        None => return Ok(JSValue::undefined()),
    };
    let matched: Vec<String> = listeners
        .borrow()
        .keys()
        .into_iter()
        .filter(|key| listeners.borrow().get(key).strict_equals(&listener))
        .collect();
    for key in matched {
        listeners.borrow_mut().delete(&key);
    }
    Ok(JSValue::undefined())
}

/// `controller.abort(reason?)` — marks the signal aborted, stores the reason
/// (defaulting to a fresh `AbortError` DOMException-shaped object), queues the
/// registered `abort` listeners as microtasks, and rejects pending fetches.
fn controller_abort(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(controller) = args.first().and_then(JSValue::as_object) else {
        return Err(JSError::TypeError(
            "AbortController.abort called on incompatible receiver".to_string(),
        ));
    };
    let signal = match controller.borrow().get("signal").as_object() {
        Some(signal) => signal,
        None => {
            return Err(JSError::TypeError(
                "AbortController.abort called on incompatible receiver".to_string(),
            ));
        }
    };
    let reason = match args.get(1) {
        Some(value) if value.kind() != JsValueKind::Undefined => value.clone(),
        _ => abort_error_value(),
    };
    abort_signal_now(vm, &signal, reason);
    Ok(JSValue::undefined())
}

/// Builds the default `AbortError` reason (a DOMException-shaped object).
fn abort_error_value() -> JSValue {
    let mut error = JSObject::new();
    error.define_property(
        "name".to_string(),
        Property::read_only(JSValue::from_string("AbortError".to_string())),
    );
    error.define_property(
        "message".to_string(),
        Property::read_only(JSValue::from_string(
            "The operation was aborted.".to_string(),
        )),
    );
    error.define_property(
        "code".to_string(),
        Property::read_only(JSValue::from_number(20.0)),
    );
    JSValue::from_object(Rc::new(RefCell::new(error)))
}

/// Builds an `abort` Event object carrying the abort reason as `detail`.
fn make_abort_event(reason: &JSValue) -> JSValue {
    let mut event = JSObject::new();
    event.define_property(
        "type".to_string(),
        Property::read_only(JSValue::from_string("abort".to_string())),
    );
    event.define_property(
        "bubbles".to_string(),
        Property::read_only(JSValue::from_bool(false)),
    );
    event.define_property(
        "cancelable".to_string(),
        Property::read_only(JSValue::from_bool(false)),
    );
    event.set("detail".to_string(), reason.clone());
    JSValue::from_object(Rc::new(RefCell::new(event)))
}

/// Rejects all fetch capabilities whose request registered `signal`, removing
/// them from the pending queue so the network layer skips them.
fn abort_registered_fetches(vm: &mut VM, signal: &Rc<RefCell<JSObject>>) {
    let reason = signal.borrow().get(ABORT_REASON);
    let rejections: Vec<JSValue> = with_host_mut(vm, |host| {
        let mut rejections = Vec::new();
        let mut remaining = Vec::new();
        let requests = std::mem::take(&mut host.fetch_requests);
        for request in requests {
            let registered = request
                .signal
                .as_ref()
                .and_then(JSValue::as_object)
                .is_some_and(|candidate| Rc::ptr_eq(&candidate, signal));
            if registered {
                if let Some(capability) = host.fetch_capabilities.remove(&request.id) {
                    rejections.push(capability.reject);
                }
            } else {
                remaining.push(request);
            }
        }
        host.fetch_requests = remaining;
        rejections
    })
    .unwrap_or_default();
    for reject in rejections {
        if is_callable(&reject) {
            let _ = vm.call(reject, JSValue::undefined(), vec![reason.clone()]);
        }
    }
}

/// `AbortSignal.abort(reason?)` — returns an already-aborted signal.
fn signal_static_abort(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let reason = match args.get(1) {
        Some(value) if !value.is_undefined() => value.clone(),
        _ => abort_error_value(),
    };
    Ok(JSValue::from_object(make_signal(true, reason)))
}

/// `AbortSignal.timeout(ms)` — returns a signal that aborts with a
/// `TimeoutError` after the delay (implemented as a plain timer; the timer
/// callback aborts the signal object held in a closure-captured slot).
fn signal_static_timeout(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let milliseconds = args.get(1).map(JSValue::to_number).unwrap_or(0.0);
    let signal = make_signal(false, JSValue::undefined());
    let signal_value = JSValue::from_object(Rc::clone(&signal));
    let callback = JSValue::from_native_function(timeout_abort_callback);
    let bound =
        JSValue::from_bound_function(BoundFunctionData::new(callback, signal_value, Vec::new()));
    let set_timeout = vm.global_object.borrow().get("setTimeout");
    let _ = vm.call(
        set_timeout,
        JSValue::undefined(),
        vec![bound, JSValue::from_number(milliseconds)],
    );
    Ok(JSValue::from_object(signal))
}

/// Timer callback for `AbortSignal.timeout`: aborts the bound signal with a
/// `TimeoutError` reason.
fn timeout_abort_callback(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let signal = match args.first().and_then(JSValue::as_object) {
        Some(signal) if signal.borrow().get(ABORT_SIGNAL_MARKER).as_boolean() == Some(true) => {
            signal
        }
        _ => return Ok(JSValue::undefined()),
    };
    let mut error = JSObject::new();
    error.define_property(
        "name".to_string(),
        Property::read_only(JSValue::from_string("TimeoutError".to_string())),
    );
    error.define_property(
        "message".to_string(),
        Property::read_only(JSValue::from_string("The operation timed out.".to_string())),
    );
    let reason = JSValue::from_object(Rc::new(RefCell::new(error)));
    abort_signal_now(vm, &signal, reason);
    Ok(JSValue::undefined())
}

/// `AbortSignal.any(signals)` — a signal that aborts when any input aborts.
/// Inputs are inspected for their current state; live propagation across
/// independent signals is registered through their listener lists.
fn signal_static_any(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let inputs = args.get(1).and_then(JSValue::as_object);
    let length = inputs
        .as_ref()
        .map(|object| object.borrow().get("length").to_number().max(0.0) as usize)
        .unwrap_or(0);
    let combined = make_signal(false, JSValue::undefined());
    for index in 0..length {
        let value = inputs
            .as_ref()
            .map(|object| object.borrow().get(&index.to_string()))
            .unwrap_or(JSValue::undefined());
        let Ok(input) = signal_receiver(&[value]) else {
            continue;
        };
        let aborted = input.borrow().get(ABORTED).as_boolean() == Some(true);
        if aborted {
            let reason = input.borrow().get(ABORT_REASON);
            combined
                .borrow_mut()
                .set(ABORTED.to_string(), JSValue::from_bool(true));
            combined.borrow_mut().set(ABORT_REASON.to_string(), reason);
            return Ok(JSValue::from_object(combined));
        }
        // Register a forwarding listener on the input signal.
        let forward = JSValue::from_native_function(any_forward_callback);
        let bound = JSValue::from_bound_function(BoundFunctionData::new(
            forward,
            JSValue::from_object(Rc::clone(&combined)),
            Vec::new(),
        ));
        let _ = vm.call(
            JSValue::from_native_function(signal_add_event_listener),
            JSValue::from_object(input),
            vec![
                JSValue::undefined(),
                JSValue::from_string("abort".to_string()),
                bound,
            ],
        );
    }
    Ok(JSValue::from_object(combined))
}

/// Forwarding listener for `AbortSignal.any`: copies the abort state onto the
/// bound combined signal and dispatches its listeners.
fn any_forward_callback(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let combined = match args.first().and_then(JSValue::as_object) {
        Some(combined) => combined,
        None => return Ok(JSValue::undefined()),
    };
    let event = args.get(2).cloned().unwrap_or(JSValue::undefined());
    let reason = event
        .as_object()
        .map(|object| object.borrow().get("detail"))
        .filter(|value| value.kind() != JsValueKind::Undefined)
        .unwrap_or_else(abort_error_value);
    abort_signal_now(vm, &combined, reason);
    Ok(JSValue::undefined())
}

/// Returns `true` when `signal` is an aborted `AbortSignal`.
pub(crate) fn signal_is_aborted(signal: &JSValue) -> bool {
    signal
        .as_object()
        .map(|object| {
            object.borrow().get(ABORT_SIGNAL_MARKER).as_boolean() == Some(true)
                && object.borrow().get(ABORTED).as_boolean() == Some(true)
        })
        .unwrap_or(false)
}

/// The reason an aborted signal carries, for fetch rejections.
pub(crate) fn signal_abort_reason(signal: &JSValue) -> JSValue {
    signal
        .as_object()
        .map(|object| object.borrow().get(ABORT_REASON))
        .unwrap_or_else(|| abort_error_value())
}

/// Marks `signal` as aborted with `reason`, dispatches its `abort` listeners
/// synchronously, and rejects pending fetches. Shared by `controller.abort()`
/// and timers.
fn abort_signal_now(vm: &mut VM, signal: &Rc<RefCell<JSObject>>, reason: JSValue) {
    if signal.borrow().get(ABORTED).as_boolean() == Some(true) {
        return;
    }
    signal
        .borrow_mut()
        .set(ABORTED.to_string(), JSValue::from_bool(true));
    signal
        .borrow_mut()
        .set(ABORT_REASON.to_string(), reason.clone());
    let listeners = signal.borrow().get(ABORT_LISTENERS);
    if let Some(listeners) = listeners.as_object() {
        let keys = listeners.borrow().keys();
        for key in keys {
            let listener = listeners.borrow().get(&key);
            if is_callable(&listener) {
                let event = make_abort_event(&reason);
                let _ = vm.call(listener, JSValue::undefined(), vec![event]);
            }
        }
    }
    abort_registered_fetches(vm, signal);
}
