//! `Blob` and `FileReader` — binary data containers and async readers.
//!
//! `Blob` stores its payload as a string (the engine's byte model: bytes are
//! represented as Latin-1 code units) with the MIME type recorded separately.
//! `FileReader` exposes the `readAsText`/`readAsDataURL`/`readAsArrayBuffer`
//! surface whose `load` / `error` events and `onload`/`onerror` handlers fire
//! synchronously with the read completing (a simplification of the platform's
//! task-based timing that scripts setting handlers synchronously after calling
//! `readAs*` do not notice).

use crate::engine::js::common::{is_callable, resolved_promise};
use pixi_byte::value::jsobject::{JSObject, Property};
use pixi_byte::vm::VM;
use pixi_byte::{JSError, JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;

/// Internal storage for a blob's payload string.
const BLOB_DATA: &str = "__orinium_blob_data";
/// Internal storage for a blob's MIME type.
const BLOB_TYPE: &str = "__orinium_blob_type";
/// Marker distinguishing blob receivers.
const BLOB_MARKER: &str = "__orinium_blob";
/// Marker distinguishing FileReader receivers.
const FILE_READER_MARKER: &str = "__orinium_file_reader";

/// Installs `Blob` and `FileReader` on the global object.
pub(crate) fn install_blob_apis(engine: &mut pixi_byte::JSEngine) {
    let mut blob = JSObject::new();
    blob.set(
        "__construct__".to_string(),
        JSValue::from_native_function(blob_constructor),
    );
    let mut file_reader = JSObject::new();
    file_reader.set(
        "__construct__".to_string(),
        JSValue::from_native_function(file_reader_constructor),
    );
    let mut global = engine.global_mut().borrow_mut();
    global.set(
        "Blob".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(blob))),
    );
    global.set(
        "FileReader".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(file_reader))),
    );
}

/// Concatenates blob parts into the payload string. String parts append
/// directly; nested blobs append their payload.
fn blob_parts_to_data(parts: Option<&JSValue>) -> String {
    let Some(parts) = parts.and_then(JSValue::as_object) else {
        return String::new();
    };
    let length = parts.borrow().get("length").to_number().max(0.0) as usize;
    let mut data = String::new();
    for index in 0..length {
        let value = parts.borrow().get(&index.to_string());
        if let Some(part) = value.as_object()
            && part.borrow().get(BLOB_MARKER).as_boolean() == Some(true)
        {
            data.push_str(&part.borrow().get(BLOB_DATA).to_string());
        } else if value.is_string() {
            data.push_str(value.as_string().unwrap_or_default());
        }
    }
    data
}

/// `new Blob(parts?, options?)` — builds a blob from string/blob parts with an
/// optional `type` option for the MIME type.
fn blob_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let data = blob_parts_to_data(args.get(1));
    let mime = args
        .get(2)
        .and_then(JSValue::as_object)
        .map(|options| options.borrow().get("type").to_string())
        .unwrap_or_default();
    let blob = make_blob(data, mime);
    attach_blob_methods(&blob);
    Ok(JSValue::from_object(blob))
}

/// Builds a blob object carrying `data` and `mime` with methods attached.
pub(crate) fn make_blob(data: String, mime: String) -> Rc<RefCell<JSObject>> {
    let mut blob = JSObject::new();
    blob.set(BLOB_MARKER.to_string(), JSValue::from_bool(true));
    blob.set(BLOB_DATA.to_string(), JSValue::from_string(data));
    blob.set(BLOB_TYPE.to_string(), JSValue::from_string(mime));
    let blob = Rc::new(RefCell::new(blob));
    attach_blob_methods(&blob);
    blob
}

/// Installs the instance methods shared by all blobs (the VM uses the
/// constructor's returned object verbatim, so methods live on the instance).
fn attach_blob_methods(blob: &Rc<RefCell<JSObject>>) {
    blob.borrow_mut().set(
        "text".to_string(),
        JSValue::from_native_function(blob_text),
    );
    blob.borrow_mut().set(
        "arrayBuffer".to_string(),
        JSValue::from_native_function(blob_array_buffer),
    );
    blob.borrow_mut().set(
        "slice".to_string(),
        JSValue::from_native_function(blob_slice),
    );
    blob.borrow_mut().set(
        "stream".to_string(),
        JSValue::from_native_function(blob_unsupported),
    );
    let size = Property {
        value: JSValue::undefined(),
        enumerable: true,
        writable: false,
        configurable: true,
        getter: Some(JSValue::from_native_function(blob_get_size)),
        setter: None,
    };
    let type_ = Property {
        value: JSValue::undefined(),
        enumerable: true,
        writable: false,
        configurable: true,
        getter: Some(JSValue::from_native_function(blob_get_type)),
        setter: None,
    };
    blob.borrow_mut().define_property("size".to_string(), size);
    blob.borrow_mut().define_property("type".to_string(), type_);
}

/// `blob.size` — the payload's byte length (UTF-8, matching string storage).
fn blob_get_size(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let blob = blob_receiver(&args)?;
    let bytes = blob.borrow().get(BLOB_DATA).to_string().len() as f64;
    Ok(JSValue::from_number(bytes))
}

/// `blob.type` — the MIME type recorded at construction.
fn blob_get_type(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let blob = blob_receiver(&args)?;
    Ok(JSValue::from_string(
        blob.borrow().get(BLOB_TYPE).to_string(),
    ))
}

fn blob_receiver(args: &[JSValue]) -> JSResult<Rc<RefCell<JSObject>>> {
    let object = args
        .first()
        .and_then(JSValue::as_object)
        .ok_or_else(|| {
            JSError::TypeError("Blob method called on incompatible receiver".to_string())
        })?;
    if object.borrow().get(BLOB_MARKER).as_boolean() != Some(true) {
        return Err(JSError::TypeError(
            "Blob method called on incompatible receiver".to_string(),
        ));
    }
    Ok(object)
}

/// `blob.text()` — a promise resolving to the payload string.
fn blob_text(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let blob = blob_receiver(&args)?;
    let data = blob.borrow().get(BLOB_DATA).to_string();
    resolved_promise(vm, JSValue::from_string(data))
}

/// `blob.arrayBuffer()` — a promise resolving to an ArrayBuffer of the bytes.
fn blob_array_buffer(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let blob = blob_receiver(&args)?;
    let data = blob.borrow().get(BLOB_DATA).to_string();
    let bytes: Vec<JSValue> = data
        .into_bytes()
        .into_iter()
        .map(|byte| JSValue::from_number(byte as f64))
        .collect();
    let array = vm.array_from_values(bytes);
    let buffer = crate::engine::js::web_apis::encoding::make_array_buffer_from_value(&array);
    resolved_promise(vm, buffer)
}

/// `blob.slice(start?, end?, contentType?)` — a new blob over a sub-range.
fn blob_slice(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let blob = blob_receiver(&args)?;
    let data = blob.borrow().get(BLOB_DATA).to_string();
    let mime = blob.borrow().get(BLOB_TYPE).to_string();
    let total = data.chars().count() as f64;
    let clamp = |value: f64, default: f64| -> usize {
        let reference = if value.is_finite() { value } else { default };
        if reference < 0.0 {
            ((total + reference).max(0.0)) as usize
        } else {
            reference.min(total) as usize
        }
    };
    let start = args.get(1).map(|v| clamp(v.to_number(), 0.0)).unwrap_or(0);
    let end = args
        .get(2)
        .map(|v| clamp(v.to_number(), total))
        .unwrap_or(total as usize);
    let sliced: String = if start < end {
        data.chars().skip(start).take(end - start).collect()
    } else {
        String::new()
    };
    let slice_mime = args.get(3).map(JSValue::to_console_string).unwrap_or(mime);
    Ok(JSValue::from_object(make_blob(sliced, slice_mime)))
}

fn blob_unsupported(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Err(JSError::TypeError(
        "Blob.stream() is not supported".to_string(),
    ))
}

// ---------------------------------------------------------------------------
// FileReader
// ---------------------------------------------------------------------------

/// `new FileReader()` — reader with `readAs*` methods and handler properties.
fn file_reader_constructor(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let mut reader = JSObject::new();
    reader.set(FILE_READER_MARKER.to_string(), JSValue::from_bool(true));
    reader.set("readyState".to_string(), JSValue::from_number(0.0));
    reader.set("result".to_string(), JSValue::null());
    reader.set("error".to_string(), JSValue::null());
    // Handlers are plain writable properties so scripts can assign them.
    for name in [
        "onloadstart",
        "onprogress",
        "onload",
        "onerror",
        "onabort",
        "onloadend",
    ] {
        reader.set(name.to_string(), JSValue::null());
    }
    type ReaderMethod = fn(&mut VM, Vec<JSValue>) -> JSResult<JSValue>;
    for (name, function) in [
        ("readAsText", file_reader_read_as_text as ReaderMethod),
        ("readAsDataURL", file_reader_read_as_data_url as ReaderMethod),
        ("readAsArrayBuffer", file_reader_read_as_array_buffer as ReaderMethod),
        ("abort", file_reader_abort as ReaderMethod),
    ] {
        reader.set(name.to_string(), JSValue::from_native_function(function));
    }
    Ok(JSValue::from_object(Rc::new(RefCell::new(reader))))
}

fn file_reader_receiver(args: &[JSValue]) -> JSResult<Rc<RefCell<JSObject>>> {
    let object = args
        .first()
        .and_then(JSValue::as_object)
        .ok_or_else(|| {
            JSError::TypeError("FileReader method called on incompatible receiver".to_string())
        })?;
    if object.borrow().get(FILE_READER_MARKER).as_boolean() != Some(true) {
        return Err(JSError::TypeError(
            "FileReader method called on incompatible receiver".to_string(),
        ));
    }
    Ok(object)
}

/// Builds a bare `ProgressEvent`-shaped object with `type` and `target`.
fn make_reader_event(event_type: &str, reader: &Rc<RefCell<JSObject>>) -> JSValue {
    let mut event = JSObject::new();
    event.define_property(
        "type".to_string(),
        Property::read_only(JSValue::from_string(event_type.to_string())),
    );
    event.set("target".to_string(), JSValue::from_object(Rc::clone(reader)));
    JSValue::from_object(Rc::new(RefCell::new(event)))
}

/// Updates the reader state and dispatches `onload` + `onloadend`
/// synchronously with the result attached to the reader (`reader.result`
/// reads it immediately).
fn file_reader_complete(vm: &mut VM, reader: &Rc<RefCell<JSObject>>, result: JSValue) {
    reader
        .borrow_mut()
        .set("readyState".to_string(), JSValue::from_number(2.0));
    reader.borrow_mut().set("result".to_string(), result);
    for event_type in ["load", "loadend"] {
        let handler_name = format!("on{event_type}");
        let handler = reader.borrow().get(&handler_name);
        if is_callable(&handler) {
            let event = make_reader_event(event_type, reader);
            let _ = vm.call(
                handler,
                JSValue::from_object(Rc::clone(reader)),
                vec![event],
            );
        }
    }
}

/// Dispatches `onerror` + `onloadend` synchronously for a failed read.
fn file_reader_fail(vm: &mut VM, reader: &Rc<RefCell<JSObject>>) {
    for event_type in ["error", "loadend"] {
        let handler_name = format!("on{event_type}");
        let handler = reader.borrow().get(&handler_name);
        if is_callable(&handler) {
            let event = make_reader_event(event_type, reader);
            let _ = vm.call(
                handler,
                JSValue::from_object(Rc::clone(reader)),
                vec![event],
            );
        }
    }
}

/// Resolves the payload string of a blob-ish argument.
fn blob_data_of(value: &JSValue) -> String {
    let Some(object) = value.as_object() else {
        return String::new();
    };
    if object.borrow().get(BLOB_MARKER).as_boolean() == Some(true) {
        object.borrow().get(BLOB_DATA).to_string()
    } else if value.is_string() {
        value.as_string().unwrap_or_default().to_string()
    } else {
        String::new()
    }
}

fn require_blob_argument(args: &[JSValue]) -> JSResult<String> {
    let Some(value) = args.get(1) else {
        return Err(JSError::TypeError(
            "FileReader.readAs* requires a Blob argument".to_string(),
        ));
    };
    let is_blob = value
        .as_object()
        .map(|object| object.borrow().get(BLOB_MARKER).as_boolean() == Some(true))
        .unwrap_or(false);
    if !is_blob {
        return Err(JSError::TypeError(
            "FileReader.readAs* requires a Blob argument".to_string(),
        ));
    }
    Ok(blob_data_of(value))
}

fn file_reader_read_as_text(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let reader = file_reader_receiver(&args)?;
    let data = require_blob_argument(&args)?;
    file_reader_complete(vm, &reader, JSValue::from_string(data));
    Ok(JSValue::undefined())
}

fn file_reader_read_as_data_url(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let reader = file_reader_receiver(&args)?;
    let blob = match args.get(1).and_then(JSValue::as_object) {
        Some(blob) => blob,
        None => {
            return Err(JSError::TypeError(
                "FileReader.readAsDataURL requires a Blob argument".to_string(),
            ))
        }
    };
    let data = blob.borrow().get(BLOB_DATA).to_string();
    let mime = blob.borrow().get(BLOB_TYPE).to_string();
    if mime.is_empty() {
        file_reader_fail(vm, &reader);
        return Ok(JSValue::undefined());
    }
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(data.as_bytes());
    file_reader_complete(
        vm,
        &reader,
        JSValue::from_string(format!("data:{mime};base64,{encoded}")),
    );
    Ok(JSValue::undefined())
}

fn file_reader_read_as_array_buffer(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let reader = file_reader_receiver(&args)?;
    let data = require_blob_argument(&args)?;
    let bytes: Vec<JSValue> = data
        .into_bytes()
        .into_iter()
        .map(|byte| JSValue::from_number(byte as f64))
        .collect();
    let array = vm.array_from_values(bytes);
    let buffer = crate::engine::js::web_apis::encoding::make_array_buffer_from_value(&array);
    file_reader_complete(vm, &reader, buffer);
    Ok(JSValue::undefined())
}

fn file_reader_abort(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::undefined())
}
