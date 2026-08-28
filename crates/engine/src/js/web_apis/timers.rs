use crate::js::JsTimer;
use crate::js::common::{is_callable, with_host_mut};
use pixi_byte::vm::VM;
use pixi_byte::{JSResult, JSValue};
use std::time::{Duration, Instant};

pub(crate) fn install_timers(engine: &mut pixi_byte::JSEngine) {
    let mut global = engine.global_mut().borrow_mut();
    global.set(
        "setTimeout".to_string(),
        JSValue::NativeFunction(set_timeout),
    );
    global.set(
        "clearTimeout".to_string(),
        JSValue::NativeFunction(clear_timer),
    );
    global.set(
        "setInterval".to_string(),
        JSValue::NativeFunction(set_interval),
    );
    global.set(
        "clearInterval".to_string(),
        JSValue::NativeFunction(clear_timer),
    );
}

fn set_timeout(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    schedule_timer(vm, args, false)
}

fn set_interval(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    schedule_timer(vm, args, true)
}

fn schedule_timer(vm: &mut VM, args: Vec<JSValue>, repeating: bool) -> JSResult<JSValue> {
    let Some(callback) = args.get(1).filter(|value| is_callable(value)).cloned() else {
        return Ok(JSValue::Number(0.0));
    };
    let delay = timer_delay(args.get(2));
    let arguments = args.into_iter().skip(3).collect();
    let Some(id) = with_host_mut(vm, |host| {
        host.next_timer_id += 1;
        let id = host.next_timer_id;
        host.timers.push(JsTimer {
            id,
            callback,
            arguments,
            deadline: Instant::now() + delay,
            interval: repeating.then_some(delay),
        });
        id
    }) else {
        return Ok(JSValue::Number(0.0));
    };
    Ok(JSValue::Number(id as f64))
}

fn timer_delay(value: Option<&JSValue>) -> Duration {
    let milliseconds = value.map(JSValue::to_number).unwrap_or(0.0);
    if !milliseconds.is_finite() || milliseconds <= 0.0 {
        Duration::ZERO
    } else {
        let milliseconds = milliseconds.min(i32::MAX as f64);
        Duration::from_secs_f64(milliseconds / 1000.0)
    }
}

pub(crate) fn clear_timer(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let id = args.get(1).map(JSValue::to_number).unwrap_or(0.0) as u64;
    let _ = with_host_mut(vm, |host| {
        host.timers.retain(|timer| timer.id != id);
    });
    Ok(JSValue::Undefined)
}
