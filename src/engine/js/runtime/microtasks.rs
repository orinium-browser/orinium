use crate::engine::js::common::is_callable;
use pixi_byte::vm::VM;
use pixi_byte::{JSError, JSResult, JSValue};

pub(crate) fn install_microtasks(engine: &mut pixi_byte::JSEngine) {
    engine.global_mut().borrow_mut().set(
        "queueMicrotask".to_string(),
        JSValue::NativeFunction(queue_microtask),
    );
}

fn queue_microtask(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(callback) = args.get(1).filter(|value| is_callable(value)).cloned() else {
        return Err(JSError::TypeError(
            "queueMicrotask callback must be callable".to_string(),
        ));
    };
    vm.enqueue_job(callback, JSValue::Undefined, Vec::new());
    Ok(JSValue::Undefined)
}
