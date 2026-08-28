use crate::js::common::{is_callable, noop};
use crate::js::web_apis::dom::element::{element_layout_size, make_dom_rect};
use pixi_byte::value::jsobject::{JSObject, Property};
use pixi_byte::vm::VM;
use pixi_byte::{JSError, JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;

// --- MutationObserver ---

const MUTATION_OBSERVER_CALLBACK: &str = "__orinium_mutation_observer_callback";
const MUTATION_OBSERVER_SCHEDULED: &str = "__orinium_mutation_observer_scheduled";

pub(crate) fn install_mutation_observer(engine: &mut pixi_byte::JSEngine) {
    let mut constructor = JSObject::new();
    constructor.set(
        "__construct__".to_string(),
        JSValue::NativeFunction(mutation_observer_constructor),
    );
    engine.global_mut().borrow_mut().set(
        "MutationObserver".to_string(),
        JSValue::Object(Rc::new(RefCell::new(constructor))),
    );
}

fn mutation_observer_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let callback = args.get(1).cloned().unwrap_or(JSValue::Undefined);
    if !is_callable(&callback) {
        return Err(JSError::TypeError(
            "MutationObserver callback must be callable".to_string(),
        ));
    }
    let mut observer = JSObject::new();
    observer.set(MUTATION_OBSERVER_CALLBACK.to_string(), callback);
    observer.set(
        "observe".to_string(),
        JSValue::NativeFunction(mutation_observer_observe),
    );
    observer.set(
        "disconnect".to_string(),
        JSValue::NativeFunction(mutation_observer_disconnect),
    );
    observer.set(
        "takeRecords".to_string(),
        JSValue::NativeFunction(mutation_observer_take_records),
    );
    Ok(JSValue::Object(Rc::new(RefCell::new(observer))))
}

fn mutation_observer_observe(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    if !matches!(args.get(1), Some(JSValue::Object(_))) {
        return Err(JSError::TypeError(
            "MutationObserver.observe target must be a Node".to_string(),
        ));
    }
    let Some(JSValue::Object(observer)) = args.first() else {
        return Err(JSError::TypeError(
            "MutationObserver.observe called on an invalid receiver".to_string(),
        ));
    };
    if !observer
        .borrow()
        .get(MUTATION_OBSERVER_SCHEDULED)
        .to_boolean()
    {
        let callback = observer.borrow().get(MUTATION_OBSERVER_CALLBACK);
        observer.borrow_mut().set(
            MUTATION_OBSERVER_SCHEDULED.to_string(),
            JSValue::Boolean(true),
        );
        vm.enqueue_job(
            callback,
            JSValue::Undefined,
            vec![
                vm.array_from_values(Vec::new()),
                JSValue::Object(Rc::clone(observer)),
            ],
        );
    }
    Ok(JSValue::Undefined)
}

fn mutation_observer_disconnect(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::Undefined)
}

fn mutation_observer_take_records(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(vm.array_from_values(Vec::new()))
}

// --- ResizeObserver ---

const RESIZE_OBSERVER_CALLBACK: &str = "__orinium_resize_observer_callback";

pub(crate) fn install_resize_observer(engine: &mut pixi_byte::JSEngine) {
    let mut constructor = JSObject::new();
    constructor.set(
        "__construct__".to_string(),
        JSValue::NativeFunction(resize_observer_constructor),
    );
    engine.global_mut().borrow_mut().set(
        "ResizeObserver".to_string(),
        JSValue::Object(Rc::new(RefCell::new(constructor))),
    );
}

fn resize_observer_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let callback = args.get(1).cloned().unwrap_or(JSValue::Undefined);
    if !is_callable(&callback) {
        return Err(JSError::TypeError(
            "ResizeObserver callback must be callable".to_string(),
        ));
    }
    let mut observer = JSObject::new();
    observer.set(RESIZE_OBSERVER_CALLBACK.to_string(), callback);
    observer.set(
        "observe".to_string(),
        JSValue::NativeFunction(resize_observer_observe),
    );
    // TODO: Track ResizeObserver subscriptions across layout commits and implement teardown.
    observer.set("unobserve".to_string(), JSValue::NativeFunction(noop));
    observer.set("disconnect".to_string(), JSValue::NativeFunction(noop));
    Ok(JSValue::Object(Rc::new(RefCell::new(observer))))
}

fn resize_observer_observe(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(JSValue::Object(observer)) = args.first() else {
        return Err(JSError::TypeError(
            "ResizeObserver.observe called on an invalid receiver".to_string(),
        ));
    };
    let Some(target) = args.get(1).cloned() else {
        return Err(JSError::TypeError(
            "ResizeObserver.observe target must be an Element".to_string(),
        ));
    };
    let Some((width, height)) = element_layout_size(vm, &target) else {
        return Err(JSError::TypeError(
            "ResizeObserver.observe target must be an Element".to_string(),
        ));
    };
    let mut entry = JSObject::new();
    entry.define_property("target".to_string(), Property::read_only(target));
    entry.define_property(
        "contentRect".to_string(),
        Property::read_only(make_dom_rect(0.0, 0.0, width, height)),
    );
    let callback = observer.borrow().get(RESIZE_OBSERVER_CALLBACK);
    vm.enqueue_job(
        callback,
        JSValue::Undefined,
        vec![
            vm.array_from_values(vec![JSValue::Object(Rc::new(RefCell::new(entry)))]),
            JSValue::Object(Rc::clone(observer)),
        ],
    );
    Ok(JSValue::Undefined)
}
