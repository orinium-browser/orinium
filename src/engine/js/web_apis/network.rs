use crate::engine::js::common::{is_callable, noop, with_host_mut};
use crate::engine::js::web_apis::abort::{signal_abort_reason, signal_is_aborted};
use crate::engine::js::web_apis::encoding::make_array_buffer_from_value;
use crate::engine::js::{JsFetchCapability, JsFetchRequest, JsFetchResponse};
use pixi_byte::value::JSArray;
use pixi_byte::value::jsobject::{JSObject, Property};
use pixi_byte::vm::VM;
use pixi_byte::{JSError, JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;

// --- fetch ---

const HEADERS_DATA: &str = "__orinium_headers_data";
const HEADERS_IMMUTABLE: &str = "__orinium_headers_immutable";
const REQUEST_MARKER: &str = "__orinium_request";
const REQUEST_BODY: &str = "__orinium_request_body";
const RESPONSE_BODY_USED: &str = "__orinium_response_body_used";
const RESPONSE_BODY_BYTES: &str = "__orinium_response_body_bytes";

pub(crate) fn install_headers(engine: &mut pixi_byte::JSEngine) {
    let mut constructor = JSObject::new();
    constructor.set(
        "__construct__".to_string(),
        JSValue::from_native_function(headers_constructor),
    );
    engine.global_mut().borrow_mut().set(
        "Headers".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(constructor))),
    );
}

fn headers_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let entries = args.get(1).map(extract_header_entries).unwrap_or_default();
    Ok(JSValue::from_object(make_headers(entries, false)))
}

pub(crate) fn make_headers(
    entries: Vec<(String, String)>,
    immutable: bool,
) -> Rc<RefCell<JSObject>> {
    let data = Rc::new(RefCell::new(JSObject::new()));
    for (name, value) in entries {
        append_header_value(&mut data.borrow_mut(), &name, &value);
    }

    let mut headers = JSObject::new();
    headers.set(HEADERS_DATA.to_string(), JSValue::from_object(data));
    headers.set(HEADERS_IMMUTABLE.to_string(), JSValue::from_bool(immutable));
    headers.set(
        "get".to_string(),
        JSValue::from_native_function(headers_get),
    );
    headers.set(
        "has".to_string(),
        JSValue::from_native_function(headers_has),
    );
    headers.set(
        "set".to_string(),
        JSValue::from_native_function(headers_set),
    );
    headers.set(
        "append".to_string(),
        JSValue::from_native_function(headers_append),
    );
    headers.set(
        "delete".to_string(),
        JSValue::from_native_function(headers_delete),
    );
    Rc::new(RefCell::new(headers))
}

pub(crate) fn extract_header_entries(value: &JSValue) -> Vec<(String, String)> {
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    let object = object.borrow();
    if let Some(data) = object.get(HEADERS_DATA).as_object() {
        let data = data.borrow();
        return data
            .keys()
            .into_iter()
            .map(|name| {
                let value = data.get(&name).to_string();
                (name, value)
            })
            .collect();
    }
    object
        .keys()
        .into_iter()
        .map(|name| {
            let value = object.get(&name).to_string();
            (name, value)
        })
        .collect()
}

fn headers_data(args: &[JSValue]) -> JSResult<Rc<RefCell<JSObject>>> {
    let Some(headers) = args.first().and_then(JSValue::as_object) else {
        return Err(JSError::TypeError(
            "Headers method called on incompatible receiver".to_string(),
        ));
    };
    let data = headers.borrow().get(HEADERS_DATA);
    data.as_object().ok_or_else(|| {
        JSError::TypeError("Headers method called on incompatible receiver".to_string())
    })
}

fn ensure_headers_mutable(args: &[JSValue]) -> JSResult<()> {
    let Some(headers) = args.first().and_then(JSValue::as_object) else {
        return Err(JSError::TypeError(
            "Headers method called on incompatible receiver".to_string(),
        ));
    };
    if headers.borrow().get(HEADERS_IMMUTABLE).as_boolean() == Some(true) {
        return Err(JSError::TypeError(
            "Response headers are immutable".to_string(),
        ));
    }
    Ok(())
}

fn header_argument(args: &[JSValue], index: usize, label: &str) -> JSResult<String> {
    let Some(value) = args.get(index) else {
        return Err(JSError::TypeError(format!("Missing header {label}")));
    };
    Ok(value.to_string())
}

fn normalize_header_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn normalize_header_value(value: &str) -> String {
    value.trim().to_string()
}

fn append_header_value(data: &mut JSObject, name: &str, value: &str) {
    let name = normalize_header_name(name);
    if name.is_empty() {
        return;
    }
    let value = normalize_header_value(value);
    let combined = if data.get(&name).is_undefined() {
        value
    } else {
        format!("{}, {}", data.get(&name), value)
    };
    data.set(name, JSValue::from_string(combined));
}

fn headers_get(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let data = headers_data(&args)?;
    let name = normalize_header_name(&header_argument(&args, 1, "name")?);
    let value = data.borrow().get(&name);
    Ok(if value.is_undefined() {
        JSValue::null()
    } else {
        value
    })
}

fn headers_has(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let data = headers_data(&args)?;
    let name = normalize_header_name(&header_argument(&args, 1, "name")?);
    let has = data.borrow().has_own_property(&name);
    Ok(JSValue::from_bool(has))
}

fn headers_set(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    ensure_headers_mutable(&args)?;
    let data = headers_data(&args)?;
    let name = normalize_header_name(&header_argument(&args, 1, "name")?);
    let value = normalize_header_value(&header_argument(&args, 2, "value")?);
    if !name.is_empty() {
        data.borrow_mut().set(name, JSValue::from_string(value));
    }
    Ok(JSValue::undefined())
}

fn headers_append(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    ensure_headers_mutable(&args)?;
    let data = headers_data(&args)?;
    let name = header_argument(&args, 1, "name")?;
    let value = header_argument(&args, 2, "value")?;
    append_header_value(&mut data.borrow_mut(), &name, &value);
    Ok(JSValue::undefined())
}

fn headers_delete(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    ensure_headers_mutable(&args)?;
    let data = headers_data(&args)?;
    let name = normalize_header_name(&header_argument(&args, 1, "name")?);
    data.borrow_mut().delete(&name);
    Ok(JSValue::undefined())
}

pub(crate) struct RequestParts {
    url: String,
    method: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    signal: Option<JSValue>,
}

pub(crate) fn install_request(engine: &mut pixi_byte::JSEngine) {
    let mut constructor = JSObject::new();
    constructor.set(
        "__construct__".to_string(),
        JSValue::from_native_function(request_constructor),
    );
    engine.global_mut().borrow_mut().set(
        "Request".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(constructor))),
    );
}

fn request_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(input) = args.get(1) else {
        return Err(JSError::TypeError("Request input is required".to_string()));
    };
    let parts = request_parts(input, args.get(2));
    Ok(JSValue::from_object(make_request(parts)))
}

fn make_request(parts: RequestParts) -> Rc<RefCell<JSObject>> {
    let mut request = JSObject::new();
    request.define_property(
        "url".to_string(),
        Property::read_only(JSValue::from_string(parts.url)),
    );
    request.define_property(
        "method".to_string(),
        Property::read_only(JSValue::from_string(parts.method)),
    );
    request.define_property(
        "headers".to_string(),
        Property::read_only(JSValue::from_object(make_headers(parts.headers, false))),
    );
    if let Some(signal) = parts.signal {
        request.define_property("signal".to_string(), Property::read_only(signal));
    }
    request.set(
        REQUEST_BODY.to_string(),
        JSValue::from_string(String::from_utf8_lossy(&parts.body).into_owned()),
    );
    request.set(REQUEST_MARKER.to_string(), JSValue::from_bool(true));
    Rc::new(RefCell::new(request))
}

pub(crate) fn request_parts(input: &JSValue, init: Option<&JSValue>) -> RequestParts {
    let mut parts = if let Some(request) = input.as_object()
        && request.borrow().get(REQUEST_MARKER).as_boolean() == Some(true)
    {
        let request = request.borrow();
        RequestParts {
            url: request.get("url").to_string(),
            method: request.get("method").to_string(),
            headers: extract_header_entries(&request.get("headers")),
            body: request
                .get(REQUEST_BODY)
                .as_string_owned()
                .map(|body| body.into_bytes())
                .unwrap_or_default(),
            signal: request
                .get("signal")
                .as_object()
                .map(|_| request.get("signal")),
        }
    } else {
        RequestParts {
            url: input.to_string(),
            method: "GET".to_string(),
            headers: Vec::new(),
            body: Vec::new(),
            signal: None,
        }
    };
    apply_request_init(&mut parts, init);
    parts
}

fn apply_request_init(parts: &mut RequestParts, init: Option<&JSValue>) {
    let Some(init) = init.and_then(JSValue::as_object) else {
        return;
    };
    let init = init.borrow();
    if init.has_own_property("method") {
        let method = init.get("method");
        if !method.is_undefined() && !method.is_null() {
            parts.method = method.to_string().to_ascii_uppercase();
        }
    }
    if init.has_own_property("headers") {
        parts.headers = extract_header_entries(&init.get("headers"));
    }
    if init.has_own_property("body") {
        let body = init.get("body");
        if body.is_undefined() || body.is_null() {
            parts.body = Vec::new();
        } else {
            parts.body = body.to_string().into_bytes();
        }
    }
    if init.has_own_property("signal") {
        let signal = init.get("signal");
        parts.signal = if signal.is_undefined() || signal.is_null() {
            None
        } else {
            Some(signal)
        };
    }
}

pub(crate) fn install_fetch(engine: &mut pixi_byte::JSEngine) {
    engine
        .global_mut()
        .borrow_mut()
        .set("fetch".to_string(), JSValue::from_native_function(fetch));
}

const XHR_METHOD: &str = "__orinium_xhr_method";
const XHR_URL: &str = "__orinium_xhr_url";
const XHR_HEADERS: &str = "__orinium_xhr_headers";

pub(crate) fn install_xml_http_request(engine: &mut pixi_byte::JSEngine) {
    let mut prototype = JSObject::new();
    prototype.set(
        "open".to_string(),
        JSValue::from_native_function(xml_http_request_open),
    );
    prototype.set(
        "send".to_string(),
        JSValue::from_native_function(xml_http_request_send),
    );
    prototype.set(
        "setRequestHeader".to_string(),
        JSValue::from_native_function(xml_http_request_set_request_header),
    );
    prototype.set(
        "getAllResponseHeaders".to_string(),
        JSValue::from_native_function(xml_http_request_get_all_response_headers),
    );
    prototype.set("abort".to_string(), JSValue::from_native_function(noop));
    prototype.set(
        "addEventListener".to_string(),
        JSValue::from_native_function(noop),
    );
    prototype.set(
        "removeEventListener".to_string(),
        JSValue::from_native_function(noop),
    );
    prototype.set(
        "dispatchEvent".to_string(),
        JSValue::from_native_function(noop),
    );
    let prototype = Rc::new(RefCell::new(prototype));
    let mut constructor = JSObject::new();
    constructor.set(
        "__construct__".to_string(),
        JSValue::from_native_function(xml_http_request_constructor),
    );
    constructor.define_property(
        "prototype".to_string(),
        pixi_byte::value::jsobject::Property::read_only(JSValue::from_object(prototype)),
    );
    engine.global_mut().borrow_mut().set(
        "XMLHttpRequest".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(constructor))),
    );
}

fn xml_http_request_constructor(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let mut xhr = JSObject::new();
    xhr.set("readyState".to_string(), JSValue::from_number(0.0));
    xhr.set("status".to_string(), JSValue::from_number(0.0));
    xhr.set(
        "statusText".to_string(),
        JSValue::from_string(String::new()),
    );
    xhr.set(
        "responseText".to_string(),
        JSValue::from_string(String::new()),
    );
    xhr.set("response".to_string(), JSValue::from_string(String::new()));
    xhr.set(
        "responseType".to_string(),
        JSValue::from_string(String::new()),
    );
    xhr.set("withCredentials".to_string(), JSValue::from_bool(false));
    xhr.set(
        XHR_HEADERS.to_string(),
        JSValue::from_object(Rc::new(RefCell::new(JSObject::new()))),
    );
    xhr.set(
        "open".to_string(),
        JSValue::from_native_function(xml_http_request_open),
    );
    xhr.set(
        "send".to_string(),
        JSValue::from_native_function(xml_http_request_send),
    );
    xhr.set(
        "setRequestHeader".to_string(),
        JSValue::from_native_function(xml_http_request_set_request_header),
    );
    xhr.set(
        "getAllResponseHeaders".to_string(),
        JSValue::from_native_function(xml_http_request_get_all_response_headers),
    );
    // TODO: Cancel the in-flight network request and dispatch XMLHttpRequest abort events.
    xhr.set("abort".to_string(), JSValue::from_native_function(noop));
    Ok(JSValue::from_object(Rc::new(RefCell::new(xhr))))
}

fn xml_http_request_open(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(xhr) = args.first().and_then(JSValue::as_object) else {
        return Err(JSError::TypeError(
            "invalid XMLHttpRequest receiver".to_string(),
        ));
    };
    let method = args
        .get(1)
        .cloned()
        .unwrap_or(JSValue::from_string("GET".to_string()))
        .to_string();
    let url = args
        .get(2)
        .cloned()
        .unwrap_or(JSValue::undefined())
        .to_string();
    let mut xhr = xhr.borrow_mut();
    xhr.set(
        XHR_METHOD.to_string(),
        JSValue::from_string(method.to_ascii_uppercase()),
    );
    xhr.set(XHR_URL.to_string(), JSValue::from_string(url));
    xhr.set("readyState".to_string(), JSValue::from_number(1.0));
    Ok(JSValue::undefined())
}

fn xml_http_request_set_request_header(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(xhr) = args.first().and_then(JSValue::as_object) else {
        return Err(JSError::TypeError(
            "invalid XMLHttpRequest receiver".to_string(),
        ));
    };
    let name = args
        .get(1)
        .cloned()
        .unwrap_or(JSValue::undefined())
        .to_string();
    let value = args
        .get(2)
        .cloned()
        .unwrap_or(JSValue::undefined())
        .to_string();
    if let Some(headers) = xhr.borrow().get(XHR_HEADERS).as_object() {
        headers.borrow_mut().set(name, JSValue::from_string(value));
    }
    Ok(JSValue::undefined())
}

fn xml_http_request_send(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(xhr) = args.first().and_then(JSValue::as_object) else {
        return Err(JSError::TypeError(
            "invalid XMLHttpRequest receiver".to_string(),
        ));
    };
    let (url, method, headers) = {
        let xhr_ref = xhr.borrow();
        let headers = xhr_ref
            .get(XHR_HEADERS)
            .as_object()
            .map(|headers| {
                headers
                    .borrow()
                    .keys()
                    .into_iter()
                    .map(|name| {
                        let value = headers.borrow().get(&name).to_string();
                        (name, value)
                    })
                    .collect()
            })
            .unwrap_or_default();
        (
            xhr_ref.get(XHR_URL).to_string(),
            xhr_ref.get(XHR_METHOD).to_string(),
            headers,
        )
    };
    let body = match args.get(1) {
        Some(value) if value.is_undefined() || value.is_null() => Vec::new(),
        Some(value) => value.to_string().into_bytes(),
        None => Vec::new(),
    };
    with_host_mut(vm, |host| {
        host.next_fetch_id += 1;
        let id = host.next_fetch_id;
        host.xhr_requests.insert(id, Rc::clone(&xhr));
        host.fetch_requests.push(JsFetchRequest {
            id,
            url,
            method,
            headers,
            body,
            signal: None,
        });
    })
    .ok_or_else(|| JSError::InternalError("XMLHttpRequest host is unavailable".to_string()))?;
    Ok(JSValue::undefined())
}

fn xml_http_request_get_all_response_headers(
    _vm: &mut VM,
    args: Vec<JSValue>,
) -> JSResult<JSValue> {
    let value = args
        .first()
        .and_then(JSValue::as_object)
        .map(|xhr| xhr.borrow().get("__orinium_xhr_response_headers"))
        .unwrap_or_else(|| JSValue::from_string(String::new()));
    Ok(value)
}

pub(crate) fn resolve_xml_http_request(
    engine: &mut pixi_byte::JSEngine,
    xhr: Rc<RefCell<JSObject>>,
    response: JsFetchResponse,
) {
    let body = String::from_utf8_lossy(&response.body).into_owned();
    let headers = response
        .headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect::<String>();
    {
        let mut xhr = xhr.borrow_mut();
        xhr.set("readyState".to_string(), JSValue::from_number(4.0));
        xhr.set(
            "status".to_string(),
            JSValue::from_number(response.status as f64),
        );
        xhr.set(
            "statusText".to_string(),
            JSValue::from_string(response.status_text),
        );
        xhr.set(
            "responseURL".to_string(),
            JSValue::from_string(response.url),
        );
        xhr.set(
            "responseText".to_string(),
            JSValue::from_string(body.clone()),
        );
        xhr.set("response".to_string(), JSValue::from_string(body));
        xhr.set(
            "__orinium_xhr_response_headers".to_string(),
            JSValue::from_string(headers),
        );
    }
    for name in ["onreadystatechange", "onload"] {
        let handler = xhr.borrow().get(name);
        if is_callable(&handler) {
            let _ = engine.call(handler, JSValue::from_object(Rc::clone(&xhr)), Vec::new());
        }
    }
}

fn fetch(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let input = args.get(1).cloned().unwrap_or(JSValue::undefined());
    let RequestParts {
        url,
        method,
        headers,
        body,
        signal,
    } = request_parts(&input, args.get(2));
    let promise_constructor = vm.global_object.borrow().get("Promise");
    let Some(constructor) = promise_constructor.as_object() else {
        return Err(JSError::InternalError(
            "Promise constructor is unavailable".to_string(),
        ));
    };
    let construct = constructor.borrow().get("__construct__");
    let _ = with_host_mut(vm, |host| host.constructing_fetch_capability = None);
    let promise = vm.call(
        construct,
        promise_constructor,
        vec![JSValue::from_native_function(capture_fetch_capability)],
    )?;
    let capability = with_host_mut(vm, |host| host.constructing_fetch_capability.take())
        .flatten()
        .ok_or_else(|| JSError::InternalError("Failed to create fetch Promise".to_string()))?;

    // A pre-aborted signal rejects the fetch without queueing a request.
    if let Some(signal) = &signal
        && signal_is_aborted(signal)
    {
        let reason = signal_abort_reason(signal);
        if is_callable(&capability.reject) {
            vm.call(capability.reject, JSValue::undefined(), vec![reason])?;
        }
        return Ok(promise);
    }

    let _ = with_host_mut(vm, |host| {
        host.next_fetch_id += 1;
        let id = host.next_fetch_id;
        host.fetch_capabilities.insert(id, capability);
        host.fetch_requests.push(JsFetchRequest {
            id,
            url,
            method,
            headers,
            body,
            signal: signal.clone(),
        });
    });
    Ok(promise)
}

fn capture_fetch_capability(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let resolve = args.get(1).cloned().unwrap_or(JSValue::undefined());
    let reject = args.get(2).cloned().unwrap_or(JSValue::undefined());
    let Some(()) = with_host_mut(vm, |host| {
        host.constructing_fetch_capability = Some(JsFetchCapability { resolve, reject });
    }) else {
        return Err(JSError::InternalError(
            "Fetch host state is unavailable".to_string(),
        ));
    };
    Ok(JSValue::undefined())
}

pub(crate) fn make_fetch_response(response: JsFetchResponse) -> Rc<RefCell<JSObject>> {
    let body_bytes = response.body.clone();
    let mut object = JSObject::new();
    object.set(RESPONSE_MARKER.to_string(), JSValue::from_bool(true));
    object.define_property(
        "headers".to_string(),
        Property::read_only(JSValue::from_object(make_headers(response.headers, true))),
    );
    object.define_property(
        "ok".to_string(),
        Property::read_only(JSValue::from_bool((200..=299).contains(&response.status))),
    );
    object.define_property(
        "status".to_string(),
        Property::read_only(JSValue::from_number(response.status as f64)),
    );
    object.define_property(
        "statusText".to_string(),
        Property::read_only(JSValue::from_string(response.status_text)),
    );
    object.define_property(
        "redirected".to_string(),
        Property::read_only(JSValue::from_bool(response.redirected)),
    );
    object.define_property(
        "bodyUsed".to_string(),
        Property {
            value: JSValue::undefined(),
            enumerable: true,
            writable: false,
            configurable: false,
            getter: Some(JSValue::from_native_function(fetch_response_body_used)),
            setter: None,
        },
    );
    object.define_property(
        "url".to_string(),
        Property::read_only(JSValue::from_string(response.url)),
    );
    object.define_property(
        "text".to_string(),
        Property::read_only(JSValue::from_native_function(fetch_response_text)),
    );
    object.define_property(
        "json".to_string(),
        Property::read_only(JSValue::from_native_function(fetch_response_json)),
    );
    object.define_property(
        "arrayBuffer".to_string(),
        Property::read_only(JSValue::from_native_function(fetch_response_array_buffer)),
    );
    object.set(
        "__orinium_response_body".to_string(),
        JSValue::from_string(String::from_utf8_lossy(&response.body).into_owned()),
    );
    object.set(RESPONSE_BODY_USED.to_string(), JSValue::from_bool(false));
    object.set(
        RESPONSE_BODY_BYTES.to_string(),
        JSArray::from_vec(
            body_bytes
                .into_iter()
                .map(|byte| JSValue::from_number(byte as f64))
                .collect(),
        )
        .to_object(),
    );
    Rc::new(RefCell::new(object))
}

// --- Response constructor ---

const RESPONSE_MARKER: &str = "__orinium_response";

/// `new Response(body?, init?)` — builds a Response from a string or Blob body
/// with `status` / `statusText` / `headers` init fields. The instance methods
/// (`text`, `json`, `arrayBuffer`) are the same natives the fetch-produced
/// response objects use, attached to the instance because the VM links the
/// returned object verbatim.
fn response_constructor(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let body = args.get(1).cloned().unwrap_or(JSValue::undefined());
    let init = args.get(2).and_then(JSValue::as_object);
    let init_field = |name: &str| -> JSValue {
        init.as_ref()
            .map(|object| object.borrow().get(name))
            .unwrap_or(JSValue::undefined())
    };
    // Per spec, a missing (or non-numeric) `status` defaults to 200; only an
    // explicitly out-of-range number is an error.
    let status_value = init_field("status").to_number();
    let status = if status_value.is_nan() {
        200
    } else if status_value.is_finite() && (200.0..=599.0).contains(&status_value) {
        status_value as u16
    } else {
        return Err(JSError::RangeError(
            "Failed to construct 'Response': The status provided (…) is outside the range [200, 599]"
                .to_string(),
        ));
    };
    let status_text = init_field("statusText").to_string();
    let headers = init
        .map(|object| extract_header_entries(&JSValue::from_object(Rc::clone(&object))))
        .unwrap_or_default();
    let body_string = if let Some(blob) = body.as_object()
        && blob.borrow().get("__orinium_blob").as_boolean() == Some(true)
    {
        blob.borrow().get("__orinium_blob_data").to_string()
    } else if body.is_undefined() || body.is_null() {
        String::new()
    } else {
        body.to_string()
    };
    let response = JsFetchResponse {
        url: String::new(),
        status,
        status_text,
        redirected: false,
        body: body_string.into_bytes(),
        headers,
    };
    let response_object = make_fetch_response(response);
    let _ = vm;
    Ok(JSValue::from_object(response_object))
}

/// Installs the `Response` global constructor.
pub(crate) fn install_response(engine: &mut pixi_byte::JSEngine) {
    let mut constructor = JSObject::new();
    constructor.set(
        "__construct__".to_string(),
        JSValue::from_native_function(response_constructor),
    );
    constructor.set(
        "error".to_string(),
        JSValue::from_native_function(response_static_error),
    );
    constructor.set(
        "redirect".to_string(),
        JSValue::from_native_function(response_static_redirect),
    );
    engine.global_mut().borrow_mut().set(
        "Response".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(constructor))),
    );
}

/// `Response.error()` — a response whose `type` is "error".
fn response_static_error(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let response = make_fetch_response(JsFetchResponse {
        url: String::new(),
        status: 0,
        status_text: String::new(),
        redirected: false,
        body: Vec::new(),
        headers: Vec::new(),
    });
    response
        .borrow_mut()
        .set("type".to_string(), JSValue::from_string("error".to_string()));
    Ok(JSValue::from_object(response))
}

/// `Response.redirect(url, status?)` — a redirect response for `url`.
fn response_static_redirect(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let url = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    let status = match args.get(2).map(JSValue::to_number).unwrap_or(302.0) as u16 {
        301 | 302 | 303 | 307 | 308 => status_argument(args.get(2)),
        _ => {
            return Err(JSError::RangeError(
                "Failed to construct 'Response': Invalid redirect status code".to_string(),
            ))
        }
    };
    let response = make_fetch_response(JsFetchResponse {
        url,
        status,
        status_text: String::new(),
        redirected: true,
        body: Vec::new(),
        headers: Vec::new(),
    });
    response
        .borrow_mut()
        .set("type".to_string(), JSValue::from_string("default".to_string()));
    Ok(JSValue::from_object(response))
}

fn status_argument(value: Option<&JSValue>) -> u16 {
    value.map(JSValue::to_number).unwrap_or(302.0) as u16
}

fn fetch_response_text(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let body = match consume_response_body(vm, &args, "text")? {
        Ok(body) => body,
        Err(rejection) => return Ok(rejection),
    };
    settle_promise(vm, "resolve", JSValue::from_string(body))
}

fn fetch_response_json(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let body = match consume_response_body(vm, &args, "json")? {
        Ok(body) => body,
        Err(rejection) => return Ok(rejection),
    };
    match serde_json::from_str::<serde_json::Value>(&body) {
        Ok(value) => settle_promise(vm, "resolve", json_to_js_value(value)),
        Err(error) => settle_promise(
            vm,
            "reject",
            JSValue::from_string(format!("Failed to parse JSON: {error}")),
        ),
    }
}

fn fetch_response_array_buffer(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let response = match consume_response_object(vm, &args, "arrayBuffer") {
        Ok(response) => response,
        Err(JSError::Thrown(rejection)) => return Ok(rejection),
        Err(error) => return Err(error),
    };
    let bytes = response.borrow().get(RESPONSE_BODY_BYTES);
    settle_promise(vm, "resolve", make_array_buffer_from_value(&bytes))
}

fn fetch_response_body_used(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(response) = args.first().and_then(JSValue::as_object) else {
        return Err(JSError::TypeError(
            "Response.bodyUsed called on incompatible receiver".to_string(),
        ));
    };
    Ok(response.borrow().get(RESPONSE_BODY_USED))
}

fn consume_response_body(
    vm: &mut VM,
    args: &[JSValue],
    method: &str,
) -> JSResult<Result<String, JSValue>> {
    let response = match consume_response_object(vm, args, method) {
        Ok(response) => response,
        Err(JSError::Thrown(value)) => return Ok(Err(value)),
        Err(error) => return Err(error),
    };
    let response = response.borrow();
    response
        .get("__orinium_response_body")
        .as_string_owned()
        .map(Ok)
        .ok_or_else(|| JSError::InternalError("Response body is unavailable".to_string()))
}

fn consume_response_object(
    vm: &mut VM,
    args: &[JSValue],
    method: &str,
) -> JSResult<Rc<RefCell<JSObject>>> {
    let Some(response) = args.first().and_then(JSValue::as_object) else {
        return Err(JSError::TypeError(format!(
            "Response.{method} called on incompatible receiver"
        )));
    };
    if response.borrow().get(RESPONSE_BODY_USED).as_boolean() == Some(true) {
        let rejection = settle_promise(
            vm,
            "reject",
            JSValue::from_string("Response body has already been consumed".to_string()),
        )?;
        return Err(JSError::Thrown(rejection));
    }
    response
        .borrow_mut()
        .set(RESPONSE_BODY_USED.to_string(), JSValue::from_bool(true));
    Ok(Rc::clone(&response))
}

fn settle_promise(vm: &mut VM, method: &str, value: JSValue) -> JSResult<JSValue> {
    let promise = vm.global_object.borrow().get("Promise");
    let Some(constructor) = promise.as_object() else {
        return Err(JSError::InternalError(
            "Promise constructor is unavailable".to_string(),
        ));
    };
    let settle = constructor.borrow().get(method);
    vm.call(settle, promise, vec![value])
}

fn json_to_js_value(value: serde_json::Value) -> JSValue {
    match value {
        serde_json::Value::Null => JSValue::null(),
        serde_json::Value::Bool(value) => JSValue::from_bool(value),
        serde_json::Value::Number(value) => {
            JSValue::from_number(value.as_f64().unwrap_or(f64::NAN))
        }
        serde_json::Value::String(value) => JSValue::from_string(value),
        serde_json::Value::Array(values) => {
            JSArray::from_vec(values.into_iter().map(json_to_js_value).collect()).to_object()
        }
        serde_json::Value::Object(properties) => {
            let mut object = JSObject::new();
            for (key, value) in properties {
                object.set(key, json_to_js_value(value));
            }
            JSValue::from_object(Rc::new(RefCell::new(object)))
        }
    }
}
