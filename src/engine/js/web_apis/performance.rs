use crate::engine::js::common::with_host;
use pixi_byte::value::jsobject::JSObject;
use pixi_byte::vm::VM;
use pixi_byte::{JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) fn install_performance(engine: &mut pixi_byte::JSEngine) {
    let mut performance = JSObject::new();
    performance.set(
        "now".to_string(),
        JSValue::from_native_function(performance_now),
    );
    engine.global_mut().borrow_mut().set(
        "performance".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(performance))),
    );
}

fn performance_now(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let milliseconds = with_host(vm, |host| {
        host.time_origin.elapsed().as_secs_f64() * 1_000.0
    })
    .unwrap_or(0.0);
    Ok(JSValue::from_number(milliseconds))
}
