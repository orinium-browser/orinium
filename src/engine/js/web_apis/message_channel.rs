use crate::engine::js::common::is_callable;
use pixi_byte::value::jsobject::JSObject;
use pixi_byte::vm::VM;
use pixi_byte::{JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;

const PEER: &str = "__message_port_peer";

/// Installs the `MessageChannel` / `MessagePort` pair on the global scope.
///
/// This is a minimal single-threaded implementation: `postMessage` delivers
/// the event to the peer port's `onmessage` handler asynchronously via the
/// microtask queue, carrying a `{data, ports}` event object.
pub(crate) fn install_message_channel(engine: &mut pixi_byte::JSEngine) {
    let mut constructor = JSObject::new();
    constructor.set(
        "__construct__".to_string(),
        JSValue::from_native_function(message_channel_constructor),
    );
    constructor.set(
        "__call__".to_string(),
        JSValue::from_native_function(message_channel_constructor),
    );
    engine.global_mut().borrow_mut().set(
        "MessageChannel".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(constructor))),
    );
}

fn make_port() -> JSValue {
    let mut port = JSObject::new();
    port.set(
        "postMessage".to_string(),
        JSValue::from_native_function(port_post_message),
    );
    port.set(
        "start".to_string(),
        JSValue::from_native_function(port_noop),
    );
    port.set(
        "close".to_string(),
        JSValue::from_native_function(port_noop),
    );
    JSValue::from_object(Rc::new(RefCell::new(port)))
}

fn message_channel_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let port1 = make_port();
    let port2 = make_port();
    let p1 = port1.as_object().unwrap();
    let p2 = port2.as_object().unwrap();
    p1.borrow_mut()
        .set(PEER.to_string(), JSValue::from_object(Rc::clone(&p2)));
    p2.borrow_mut()
        .set(PEER.to_string(), JSValue::from_object(Rc::clone(&p1)));
    let mut channel = JSObject::new();
    channel.set("port1".to_string(), port1);
    channel.set("port2".to_string(), port2);
    let _ = args; // receiver is unused
    Ok(JSValue::from_object(Rc::new(RefCell::new(channel))))
}

fn port_noop(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::undefined())
}

fn port_post_message(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(port) = args.first().and_then(JSValue::as_object) else {
        return Err(pixi_byte::JSError::TypeError(
            "MessagePort.postMessage: invalid receiver".to_string(),
        ));
    };
    let data = args.get(1).cloned().unwrap_or(JSValue::undefined());
    let peer = port.borrow().get(PEER);
    let Some(peer_obj) = peer.as_object() else {
        return Ok(JSValue::undefined());
    };
    let handler = peer_obj.borrow().get("onmessage");
    if is_callable(&handler) {
        let mut event = JSObject::new();
        event.set("data".to_string(), data);
        event.set(
            "ports".to_string(),
            vm.array_from_values(vec![JSValue::from_object(Rc::clone(&peer_obj))]),
        );
        event.set(
            "target".to_string(),
            JSValue::from_object(Rc::clone(&peer_obj)),
        );
        vm.enqueue_job(
            handler,
            JSValue::undefined(),
            vec![JSValue::from_object(Rc::new(RefCell::new(event)))],
        );
    }
    Ok(JSValue::undefined())
}
