use pixi_byte::value::jsobject::{JSObject, Property};
use pixi_byte::vm::VM;
use pixi_byte::{JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) fn make_event_constructor(custom: bool) -> Rc<RefCell<JSObject>> {
    let mut constructor = JSObject::new();
    constructor.set(
        "__construct__".to_string(),
        JSValue::NativeFunction(if custom {
            custom_event_constructor
        } else {
            event_constructor
        }),
    );
    Rc::new(RefCell::new(constructor))
}

fn event_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    make_constructed_event(args, false)
}

fn custom_event_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    make_constructed_event(args, true)
}

fn make_constructed_event(args: Vec<JSValue>, custom: bool) -> JSResult<JSValue> {
    let event_type = args.get(1).unwrap_or(&JSValue::Undefined).to_string();
    let options = args.get(2).and_then(|value| match value {
        JSValue::Object(object) => Some(Rc::clone(object)),
        _ => None,
    });
    let option = |name: &str| {
        options
            .as_ref()
            .map(|object| object.borrow().get(name))
            .unwrap_or(JSValue::Undefined)
    };
    let mut event = JSObject::new();
    event.define_property(
        "type".to_string(),
        Property::read_only(JSValue::String(event_type)),
    );
    event.define_property(
        "bubbles".to_string(),
        Property::read_only(JSValue::Boolean(option("bubbles").to_boolean())),
    );
    event.define_property(
        "cancelable".to_string(),
        Property::read_only(JSValue::Boolean(option("cancelable").to_boolean())),
    );
    event.set("defaultPrevented".to_string(), JSValue::Boolean(false));
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
    if custom {
        event.define_property("detail".to_string(), Property::read_only(option("detail")));
    }
    Ok(JSValue::Object(Rc::new(RefCell::new(event))))
}

pub(crate) fn make_event(
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

pub(crate) fn event_flag(event: &Rc<RefCell<JSObject>>, name: &str) -> bool {
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
