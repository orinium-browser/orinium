use pixi_byte::value::jsobject::{JSObject, Property};
use pixi_byte::vm::VM;
use pixi_byte::{JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) fn make_event_constructor(custom: bool) -> Rc<RefCell<JSObject>> {
    let mut constructor = JSObject::new();
    constructor.set(
        "__construct__".to_string(),
        JSValue::from_native_function(if custom {
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
    let event_type = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    let options = args.get(2).and_then(JSValue::as_object);
    let option = |name: &str| {
        options
            .as_ref()
            .map(|object| object.borrow().get(name))
            .unwrap_or(JSValue::undefined())
    };
    let mut event = JSObject::new();
    event.define_property(
        "type".to_string(),
        Property::read_only(JSValue::from_string(event_type)),
    );
    event.define_property(
        "bubbles".to_string(),
        Property::read_only(JSValue::from_bool(option("bubbles").to_boolean())),
    );
    event.define_property(
        "cancelable".to_string(),
        Property::read_only(JSValue::from_bool(option("cancelable").to_boolean())),
    );
    event.set("defaultPrevented".to_string(), JSValue::from_bool(false));
    event.set(
        "preventDefault".to_string(),
        JSValue::from_native_function(event_prevent_default),
    );
    event.set(
        "stopPropagation".to_string(),
        JSValue::from_native_function(event_stop_propagation),
    );
    event.set(
        "stopImmediatePropagation".to_string(),
        JSValue::from_native_function(event_stop_immediate_propagation),
    );
    if custom {
        event.define_property("detail".to_string(), Property::read_only(option("detail")));
    }
    Ok(JSValue::from_object(Rc::new(RefCell::new(event))))
}

pub(crate) fn make_event(
    event_type: &str,
    target: Rc<RefCell<JSObject>>,
    current_target: Rc<RefCell<JSObject>>,
) -> Rc<RefCell<JSObject>> {
    let mut event = JSObject::new();
    event.define_property(
        "type".to_string(),
        Property::read_only(JSValue::from_string(event_type.to_string())),
    );
    event.define_property(
        "target".to_string(),
        Property::read_only(JSValue::from_object(Rc::clone(&target))),
    );
    event.define_property(
        "currentTarget".to_string(),
        Property::read_only(JSValue::from_object(current_target)),
    );
    event.define_property(
        "bubbles".to_string(),
        Property::read_only(JSValue::from_bool(true)),
    );
    event.define_property(
        "cancelable".to_string(),
        Property::read_only(JSValue::from_bool(true)),
    );
    event.set("defaultPrevented".to_string(), JSValue::from_bool(false));
    event.set("cancelBubble".to_string(), JSValue::from_bool(false));
    event.set(
        "preventDefault".to_string(),
        JSValue::from_native_function(event_prevent_default),
    );
    event.set(
        "stopPropagation".to_string(),
        JSValue::from_native_function(event_stop_propagation),
    );
    event.set(
        "stopImmediatePropagation".to_string(),
        JSValue::from_native_function(event_stop_immediate_propagation),
    );
    Rc::new(RefCell::new(event))
}

pub(crate) fn event_flag(event: &Rc<RefCell<JSObject>>, name: &str) -> bool {
    event.borrow().get(name).as_boolean() == Some(true)
}

pub(crate) fn event_prevent_default(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    if let Some(event) = args.first().and_then(JSValue::as_object) {
        event
            .borrow_mut()
            .set("defaultPrevented".to_string(), JSValue::from_bool(true));
    }
    Ok(JSValue::undefined())
}

pub(crate) fn event_stop_propagation(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    if let Some(event) = args.first().and_then(JSValue::as_object) {
        event
            .borrow_mut()
            .set("cancelBubble".to_string(), JSValue::from_bool(true));
    }
    Ok(JSValue::undefined())
}

pub(crate) fn event_stop_immediate_propagation(
    vm: &mut VM,
    args: Vec<JSValue>,
) -> JSResult<JSValue> {
    event_stop_propagation(vm, args.clone())?;
    if let Some(event) = args.first().and_then(JSValue::as_object) {
        event.borrow_mut().set(
            "__orinium_immediate_propagation_stopped".to_string(),
            JSValue::from_bool(true),
        );
    }
    Ok(JSValue::undefined())
}

/// Legacy `document.createEvent(type)` event object. Returns an `Event`-shaped
/// object with mutable `type`/`bubbles`/`cancelable`/`detail`/`eventPhase`
/// populated by `initEvent`/`initUIEvent` (and initMouseEvent stubs).
pub(crate) fn make_create_event(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let _type = args.first().unwrap_or(&JSValue::undefined()).to_string();
    let mut event = JSObject::new();
    event.set("type".to_string(), JSValue::from_string(String::new()));
    event.set("bubbles".to_string(), JSValue::from_bool(false));
    event.set("cancelable".to_string(), JSValue::from_bool(false));
    event.set("detail".to_string(), JSValue::from_number(0.0));
    event.set("eventPhase".to_string(), JSValue::from_number(0.0));
    event.set("target".to_string(), JSValue::null());
    event.set("currentTarget".to_string(), JSValue::null());
    event.set("defaultPrevented".to_string(), JSValue::from_bool(false));
    event.set(
        "preventDefault".to_string(),
        JSValue::from_native_function(event_prevent_default),
    );
    event.set(
        "stopPropagation".to_string(),
        JSValue::from_native_function(event_stop_propagation),
    );
    event.set(
        "stopImmediatePropagation".to_string(),
        JSValue::from_native_function(event_stop_immediate_propagation),
    );
    event.set(
        "initEvent".to_string(),
        JSValue::from_native_function(init_event),
    );
    event.set(
        "initUIEvent".to_string(),
        JSValue::from_native_function(init_ui_event),
    );
    Ok(JSValue::from_object(Rc::new(RefCell::new(event))))
}

fn init_event(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    init_event_common(vm, args, false)
}

fn init_ui_event(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    init_event_common(vm, args, true)
}

fn init_event_common(_vm: &mut VM, args: Vec<JSValue>, ui: bool) -> JSResult<JSValue> {
    let Some(event) = args.first().and_then(JSValue::as_object) else {
        return Ok(JSValue::undefined());
    };
    let event_type = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    let bubbles = args.get(2).unwrap_or(&JSValue::undefined()).to_boolean();
    let cancelable = args.get(3).unwrap_or(&JSValue::undefined()).to_boolean();
    let mut e = event.borrow_mut();
    e.set("type".to_string(), JSValue::from_string(event_type));
    e.set("bubbles".to_string(), JSValue::from_bool(bubbles));
    e.set("cancelable".to_string(), JSValue::from_bool(cancelable));
    if ui {
        let detail = args.get(5).unwrap_or(&JSValue::undefined()).clone();
        e.set("detail".to_string(), detail);
    }
    Ok(JSValue::undefined())
}
