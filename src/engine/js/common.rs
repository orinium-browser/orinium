use crate::engine::html::HtmlNodeType;
use crate::engine::js::JsHost;
use crate::engine::tree::NodeRef;
use pixi_byte::value::jsobject::Property;
use pixi_byte::vm::VM;
use pixi_byte::{JSResult, JSValue};
use std::any::Any;

/// `&JSValue` sentinel for `undefined`. A `const fn` call can't be a promoted
/// rvalue, so `&JSValue::undefined()` wouldn't outlive the statement.
pub(crate) static UNDEFINED: JSValue = JSValue::undefined();

/// Runs `f` with an immutable borrow of the host data, if set and downcastable.
pub(crate) fn with_host<R>(vm: &VM, f: impl FnOnce(&JsHost) -> R) -> Option<R> {
    let host = vm.host.as_ref()?;
    let host_ref = host.borrow();
    let js_host = (&*host_ref as &dyn Any).downcast_ref::<JsHost>()?;
    Some(f(js_host))
}

/// Runs `f` with a mutable borrow of the host data, if set and downcastable.
pub(crate) fn with_host_mut<R>(vm: &VM, f: impl FnOnce(&mut JsHost) -> R) -> Option<R> {
    let host = vm.host.as_ref()?;
    let mut host_ref = host.borrow_mut();
    let js_host = (&mut *host_ref as &mut dyn Any).downcast_mut::<JsHost>()?;
    Some(f(js_host))
}

/// Records a DOM mutation: bumps the tree version and flags a relayout.
pub(crate) fn mark_dom_dirty(vm: &VM) {
    if let Some(host) = vm.host.as_ref() {
        let host_ref = host.borrow();
        if let Some(js_host) = (&*host_ref as &dyn Any).downcast_ref::<JsHost>() {
            js_host.dom.mark_dirty();
            js_host.needs_redraw.set(true);
        }
    }
}

/// Extracts the hidden DOM id counter from an element `this` object.
pub(crate) fn node_dom_id(this: &JSValue) -> Option<u64> {
    let obj = this.as_object()?;
    let n = obj.borrow().get("__orinium_dom_id").as_number()?;
    Some(n as u64)
}

/// Resolves the `this` element back to a live DOM node (dead node -> None).
pub(crate) fn dom_node(vm: &VM, this: &JSValue) -> Option<NodeRef<HtmlNodeType>> {
    let dom_id = node_dom_id(this)?;
    with_host(vm, |host| host.refs.get(&dom_id).and_then(|w| w.upgrade())).flatten()
}

pub(crate) fn host_read_only_property(value: JSValue) -> Property {
    let mut property = Property::read_only(value);
    // The host may refresh this state, while page assignments remain blocked.
    property.configurable = true;
    property
}

pub(crate) fn is_callable(value: &JSValue) -> bool {
    value.is_callable()
}

pub(crate) fn noop(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::undefined())
}

pub(crate) fn read_only_accessor_property(getter: pixi_byte::NativeFunctionType) -> Property {
    Property {
        value: JSValue::undefined(),
        enumerable: true,
        writable: false,
        configurable: false,
        getter: Some(JSValue::from_native_function(getter)),
        setter: None,
    }
}
