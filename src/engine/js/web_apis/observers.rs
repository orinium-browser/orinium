use crate::engine::js::common::{is_callable, noop, with_host};
use crate::engine::js::web_apis::dom::element::{
    element_layout_metrics, element_layout_size, make_dom_rect,
};
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
        JSValue::from_native_function(mutation_observer_constructor),
    );
    engine.global_mut().borrow_mut().set(
        "MutationObserver".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(constructor))),
    );
}

fn mutation_observer_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let callback = args.get(1).cloned().unwrap_or(JSValue::undefined());
    if !is_callable(&callback) {
        return Err(JSError::TypeError(
            "MutationObserver callback must be callable".to_string(),
        ));
    }
    let mut observer = JSObject::new();
    observer.set(MUTATION_OBSERVER_CALLBACK.to_string(), callback);
    observer.set(
        "observe".to_string(),
        JSValue::from_native_function(mutation_observer_observe),
    );
    observer.set(
        "disconnect".to_string(),
        JSValue::from_native_function(mutation_observer_disconnect),
    );
    observer.set(
        "takeRecords".to_string(),
        JSValue::from_native_function(mutation_observer_take_records),
    );
    Ok(JSValue::from_object(Rc::new(RefCell::new(observer))))
}

fn mutation_observer_observe(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    if !args.get(1).is_some_and(JSValue::is_object) {
        return Err(JSError::TypeError(
            "MutationObserver.observe target must be a Node".to_string(),
        ));
    }
    let Some(observer) = args.first().and_then(JSValue::as_object) else {
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
            JSValue::from_bool(true),
        );
        vm.enqueue_job(
            callback,
            JSValue::undefined(),
            vec![
                vm.array_from_values(Vec::new()),
                JSValue::from_object(Rc::clone(&observer)),
            ],
        );
    }
    Ok(JSValue::undefined())
}

fn mutation_observer_disconnect(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::undefined())
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
        JSValue::from_native_function(resize_observer_constructor),
    );
    engine.global_mut().borrow_mut().set(
        "ResizeObserver".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(constructor))),
    );
}

fn resize_observer_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let callback = args.get(1).cloned().unwrap_or(JSValue::undefined());
    if !is_callable(&callback) {
        return Err(JSError::TypeError(
            "ResizeObserver callback must be callable".to_string(),
        ));
    }
    let mut observer = JSObject::new();
    observer.set(RESIZE_OBSERVER_CALLBACK.to_string(), callback);
    observer.set(
        "observe".to_string(),
        JSValue::from_native_function(resize_observer_observe),
    );
    // TODO: Track ResizeObserver subscriptions across layout commits and implement teardown.
    observer.set("unobserve".to_string(), JSValue::from_native_function(noop));
    observer.set(
        "disconnect".to_string(),
        JSValue::from_native_function(noop),
    );
    Ok(JSValue::from_object(Rc::new(RefCell::new(observer))))
}

fn resize_observer_observe(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(observer) = args.first().and_then(JSValue::as_object) else {
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
        JSValue::undefined(),
        vec![
            vm.array_from_values(vec![JSValue::from_object(Rc::new(RefCell::new(entry)))]),
            JSValue::from_object(Rc::clone(&observer)),
        ],
    );
    Ok(JSValue::undefined())
}

// --- IntersectionObserver ---

const INTERSECTION_OBSERVER_CALLBACK: &str = "__orinium_intersection_observer_callback";
pub(crate) fn install_intersection_observer(engine: &mut pixi_byte::JSEngine) {
    let mut constructor = JSObject::new();
    constructor.set(
        "__construct__".to_string(),
        JSValue::from_native_function(intersection_observer_constructor),
    );
    engine.global_mut().borrow_mut().set(
        "IntersectionObserver".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(constructor))),
    );
}

fn intersection_observer_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let callback = args.get(1).cloned().unwrap_or(JSValue::undefined());
    if !is_callable(&callback) {
        return Err(JSError::TypeError(
            "IntersectionObserver callback must be callable".to_string(),
        ));
    }
    let mut observer = JSObject::new();
    observer.set(INTERSECTION_OBSERVER_CALLBACK.to_string(), callback);
    observer.set(
        "observe".to_string(),
        JSValue::from_native_function(intersection_observer_observe),
    );
    observer.set(
        "unobserve".to_string(),
        JSValue::from_native_function(intersection_observer_unobserve),
    );
    observer.set(
        "disconnect".to_string(),
        JSValue::from_native_function(intersection_observer_disconnect),
    );
    observer.set(
        "takeRecords".to_string(),
        JSValue::from_native_function(intersection_observer_take_records),
    );
    Ok(JSValue::from_object(Rc::new(RefCell::new(observer))))
}

fn intersection_observer_observe(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(observer) = args.first().and_then(JSValue::as_object) else {
        return Err(JSError::TypeError(
            "IntersectionObserver.observe called on an invalid receiver".to_string(),
        ));
    };
    let Some(target) = args.get(1).cloned() else {
        return Err(JSError::TypeError(
            "IntersectionObserver.observe target must be an Element".to_string(),
        ));
    };

    // Compute intersection using layout metrics.
    let target_metrics = element_layout_metrics(vm, &target);
    let viewport = with_host(vm, |host| host.viewport).unwrap_or((800.0, 600.0));

    let (target_rect, root_bounds) = match target_metrics {
        Some(metrics) => {
            let target_rect = (
                metrics.rect_left,
                metrics.rect_top,
                metrics.rect_width,
                metrics.rect_height,
            );
            let root_bounds = (0.0, 0.0, viewport.0, viewport.1);
            (target_rect, root_bounds)
        }
        None => {
            // Fallback: use layout size, position at origin.
            let (w, h) = element_layout_size(vm, &target).unwrap_or((0.0, 0.0));
            ((0.0, 0.0, w, h), (0.0, 0.0, viewport.0, viewport.1))
        }
    };

    // Compute intersection rectangle (clipping).
    let ix = target_rect.0.max(root_bounds.0);
    let iy = target_rect.1.max(root_bounds.1);
    let ir = (target_rect.0 + target_rect.2).min(root_bounds.0 + root_bounds.2);
    let ib = (target_rect.1 + target_rect.3).min(root_bounds.1 + root_bounds.3);
    let intersection_width = (ir - ix).max(0.0);
    let intersection_height = (ib - iy).max(0.0);
    let intersection_area = intersection_width * intersection_height;
    let target_area = target_rect.2 * target_rect.3;
    let intersection_ratio = if target_area > 0.0 {
        (intersection_area / target_area).min(1.0).max(0.0)
    } else {
        0.0
    };
    let is_intersecting = intersection_area > 0.0;

    let mut entry = JSObject::new();
    entry.define_property("target".to_string(), Property::read_only(target));
    entry.define_property(
        "boundingClientRect".to_string(),
        Property::read_only(make_dom_rect(
            target_rect.0,
            target_rect.1,
            target_rect.2,
            target_rect.3,
        )),
    );
    entry.define_property(
        "rootBounds".to_string(),
        Property::read_only(make_dom_rect(
            root_bounds.0,
            root_bounds.1,
            root_bounds.2,
            root_bounds.3,
        )),
    );
    entry.define_property(
        "intersectionRect".to_string(),
        Property::read_only(make_dom_rect(
            ix,
            iy,
            intersection_width,
            intersection_height,
        )),
    );
    entry.define_property(
        "intersectionRatio".to_string(),
        Property::read_only(JSValue::from_number(intersection_ratio)),
    );
    entry.define_property(
        "isIntersecting".to_string(),
        Property::read_only(JSValue::from_bool(is_intersecting)),
    );
    entry.define_property(
        "time".to_string(),
        Property::read_only(JSValue::from_number(0.0)),
    );

    let callback = observer.borrow().get(INTERSECTION_OBSERVER_CALLBACK);
    vm.enqueue_job(
        callback,
        JSValue::undefined(),
        vec![
            vm.array_from_values(vec![JSValue::from_object(Rc::new(RefCell::new(entry)))]),
            JSValue::from_object(Rc::clone(&observer)),
        ],
    );
    Ok(JSValue::undefined())
}

fn intersection_observer_unobserve(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::undefined())
}

fn intersection_observer_disconnect(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::undefined())
}

fn intersection_observer_take_records(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::undefined())
}
