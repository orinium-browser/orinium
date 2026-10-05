use crate::engine::js::common::{read_only_accessor_property, with_host, with_host_mut};
use pixi_byte::value::jsobject::JSObject;
use pixi_byte::vm::VM;
use pixi_byte::{JSError, JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;

const STORAGE_KIND: &str = "__orinium_storage_kind";

pub(crate) fn make_storage(kind: &str) -> Rc<RefCell<JSObject>> {
    let mut storage = JSObject::new();
    storage.set(
        STORAGE_KIND.to_string(),
        JSValue::from_string(kind.to_string()),
    );
    storage.define_property(
        "length".to_string(),
        read_only_accessor_property(storage_length),
    );
    storage.set(
        "getItem".to_string(),
        JSValue::from_native_function(storage_get_item),
    );
    storage.set(
        "setItem".to_string(),
        JSValue::from_native_function(storage_set_item),
    );
    storage.set(
        "removeItem".to_string(),
        JSValue::from_native_function(storage_remove_item),
    );
    storage.set(
        "clear".to_string(),
        JSValue::from_native_function(storage_clear),
    );
    storage.set(
        "key".to_string(),
        JSValue::from_native_function(storage_key),
    );
    Rc::new(RefCell::new(storage))
}

fn storage_kind(args: &[JSValue]) -> JSResult<String> {
    let Some(storage) = args.first().and_then(JSValue::as_object) else {
        return Err(JSError::TypeError(
            "Storage method called on incompatible receiver".to_string(),
        ));
    };
    match storage.borrow().get(STORAGE_KIND).as_string() {
        Some(kind) if kind == "local" || kind == "session" => Ok(kind.to_string()),
        _ => Err(JSError::TypeError(
            "Storage method called on incompatible receiver".to_string(),
        )),
    }
}

fn storage_length(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let kind = storage_kind(&args)?;
    let length = with_host(vm, |host| {
        if kind == "local" {
            host.local_storage.len()
        } else {
            host.session_storage.len()
        }
    })
    .unwrap_or(0);
    Ok(JSValue::from_number(length as f64))
}

fn storage_get_item(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let kind = storage_kind(&args)?;
    let key = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    let value = with_host(vm, |host| {
        let storage = if kind == "local" {
            &host.local_storage
        } else {
            &host.session_storage
        };
        storage.get(&key).cloned()
    })
    .flatten();
    Ok(value.map(JSValue::from_string).unwrap_or(JSValue::null()))
}

fn storage_set_item(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let kind = storage_kind(&args)?;
    let key = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    let value = args.get(2).unwrap_or(&JSValue::undefined()).to_string();
    let _ = with_host_mut(vm, |host| {
        let storage = if kind == "local" {
            &mut host.local_storage
        } else {
            &mut host.session_storage
        };
        storage.insert(key, value);
    });
    Ok(JSValue::undefined())
}

fn storage_remove_item(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let kind = storage_kind(&args)?;
    let key = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    let _ = with_host_mut(vm, |host| {
        let storage = if kind == "local" {
            &mut host.local_storage
        } else {
            &mut host.session_storage
        };
        storage.remove(&key);
    });
    Ok(JSValue::undefined())
}

fn storage_clear(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let kind = storage_kind(&args)?;
    let _ = with_host_mut(vm, |host| {
        if kind == "local" {
            host.local_storage.clear();
        } else {
            host.session_storage.clear();
        }
    });
    Ok(JSValue::undefined())
}

fn storage_key(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let kind = storage_kind(&args)?;
    let index = args.get(1).map(JSValue::to_number).unwrap_or(f64::NAN);
    if !index.is_finite() || index < 0.0 {
        return Ok(JSValue::null());
    }
    let value = with_host(vm, |host| {
        let storage = if kind == "local" {
            &host.local_storage
        } else {
            &host.session_storage
        };
        let mut keys: Vec<_> = storage.keys().cloned().collect();
        keys.sort();
        keys.get(index as usize).cloned()
    })
    .flatten();
    Ok(value.map(JSValue::from_string).unwrap_or(JSValue::null()))
}
