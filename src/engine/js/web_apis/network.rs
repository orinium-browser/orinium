use crate::engine::js::web_apis::encoding::make_array_buffer_from_value;
use crate::engine::js::common::{is_callable, noop, with_host_mut};
use crate::engine::js::{JsFetchRequest, JsFetchCapability, JsFetchResponse};
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
        JSValue::NativeFunction(headers_constructor),
    );
    engine.global_mut().borrow_mut().set(
        "Headers".to_string(),
        JSValue::Object(Rc::new(RefCell::new(constructor))),
    );
}

fn headers_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let entries = args.get(1).map(extract_header_entries).unwrap_or_default();
    Ok(JSValue::Object(make_headers(entries, false)))
}

pub(crate) fn make_headers(entries: Vec<(String, String)>, immutable: bool) -> Rc<RefCell<JSObject>> {
    let data = Rc::new(RefCell::new(JSObject::new()));
    for (name, value) in entries {
        append_header_value(&mut data.borrow_mut(), &name, &value);
    }

    let mut headers = JSObject::new();
    headers.set(HEADERS_DATA.to_string(), JSValue::Object(data));
    headers.set(HEADERS_IMMUTABLE.to_string(), JSValue::Boolean(immutable));
    headers.set("get".to_string(), JSValue::NativeFunction(headers_get));
    headers.set("has".to_string(), JSValue::NativeFunction(headers_has));
    headers.set("set".to_string(), JSValue::NativeFunction(headers_set));
    headers.set(
        "append".to_string(),
        JSValue::NativeFunction(headers_append),
    );
    headers.set(
        "delete".to_string(),
        JSValue::NativeFunction(headers_delete),
    );
    Rc::new(RefCell::new(headers))
}

pub(crate) fn extract_header_entries(value: &JSValue) -> Vec<(String, String)> {
    let JSValue::Object(object) = value else {
        return Vec::new();
    };
    let object = object.borrow();
    if let JSValue::Object(data) = object.get(HEADERS_DATA) {
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
    let Some(JSValue::Object(headers)) = args.first() else {
        return Err(JSError::TypeError(
            "Headers method called on incompatible receiver".to_string(),
        ));
    };
    let data = headers.borrow().get(HEADERS_DATA);
    match data {
        JSValue::Object(data) => Ok(data),
        _ => Err(JSError::TypeError(
            "Headers method called on incompatible receiver".to_string(),
        )),
    }
}

fn ensure_headers_mutable(args: &[JSValue]) -> JSResult<()> {
    let Some(JSValue::Object(headers)) = args.first() else {
        return Err(JSError::TypeError(
            "Headers method called on incompatible receiver".to_string(),
        ));
    };
    if matches!(
        headers.borrow().get(HEADERS_IMMUTABLE),
        JSValue::Boolean(true)
    ) {
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
    let combined = match data.get(&name) {
        JSValue::Undefined => value,
        current => format!("{}, {}", current, value),
    };
    data.set(name, JSValue::String(combined));
}

fn headers_get(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let data = headers_data(&args)?;
    let name = normalize_header_name(&header_argument(&args, 1, "name")?);
    Ok(match data.borrow().get(&name) {
        JSValue::Undefined => JSValue::Null,
        value => value,
    })
}

fn headers_has(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let data = headers_data(&args)?;
    let name = normalize_header_name(&header_argument(&args, 1, "name")?);
    let has = data.borrow().has_own_property(&name);
    Ok(JSValue::Boolean(has))
}

fn headers_set(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    ensure_headers_mutable(&args)?;
    let data = headers_data(&args)?;
    let name = normalize_header_name(&header_argument(&args, 1, "name")?);
    let value = normalize_header_value(&header_argument(&args, 2, "value")?);
    if !name.is_empty() {
        data.borrow_mut().set(name, JSValue::String(value));
    }
    Ok(JSValue::Undefined)
}

fn headers_append(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    ensure_headers_mutable(&args)?;
    let data = headers_data(&args)?;
    let name = header_argument(&args, 1, "name")?;
    let value = header_argument(&args, 2, "value")?;
    append_header_value(&mut data.borrow_mut(), &name, &value);
    Ok(JSValue::Undefined)
}

fn headers_delete(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    ensure_headers_mutable(&args)?;
    let data = headers_data(&args)?;
    let name = normalize_header_name(&header_argument(&args, 1, "name")?);
    data.borrow_mut().delete(&name);
    Ok(JSValue::Undefined)
}

pub(crate) struct RequestParts {
    url: String,
    method: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

pub(crate) fn install_request(engine: &mut pixi_byte::JSEngine) {
    let mut constructor = JSObject::new();
    constructor.set(
        "__construct__".to_string(),
        JSValue::NativeFunction(request_constructor),
    );
    engine.global_mut().borrow_mut().set(
        "Request".to_string(),
        JSValue::Object(Rc::new(RefCell::new(constructor))),
    );
}

fn request_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(input) = args.get(1) else {
        return Err(JSError::TypeError("Request input is required".to_string()));
    };
    let parts = request_parts(input, args.get(2));
    Ok(JSValue::Object(make_request(parts)))
}

fn make_request(parts: RequestParts) -> Rc<RefCell<JSObject>> {
    let mut request = JSObject::new();
    request.define_property(
        "url".to_string(),
        Property::read_only(JSValue::String(parts.url)),
    );
    request.define_property(
        "method".to_string(),
        Property::read_only(JSValue::String(parts.method)),
    );
    request.define_property(
        "headers".to_string(),
        Property::read_only(JSValue::Object(make_headers(parts.headers, false))),
    );
    request.set(
        REQUEST_BODY.to_string(),
        JSValue::String(String::from_utf8_lossy(&parts.body).into_owned()),
    );
    request.set(REQUEST_MARKER.to_string(), JSValue::Boolean(true));
    Rc::new(RefCell::new(request))
}

pub(crate) fn request_parts(input: &JSValue, init: Option<&JSValue>) -> RequestParts {
    let mut parts = match input {
        JSValue::Object(request)
            if matches!(request.borrow().get(REQUEST_MARKER), JSValue::Boolean(true)) =>
        {
            let request = request.borrow();
            RequestParts {
                url: request.get("url").to_string(),
                method: request.get("method").to_string(),
                headers: extract_header_entries(&request.get("headers")),
                body: match request.get(REQUEST_BODY) {
                    JSValue::String(body) => body.into_bytes(),
                    _ => Vec::new(),
                },
            }
        }
        value => RequestParts {
            url: value.to_string(),
            method: "GET".to_string(),
            headers: Vec::new(),
            body: Vec::new(),
        },
    };
    apply_request_init(&mut parts, init);
    parts
}

fn apply_request_init(parts: &mut RequestParts, init: Option<&JSValue>) {
    let Some(JSValue::Object(init)) = init else {
        return;
    };
    let init = init.borrow();
    if init.has_own_property("method") {
        match init.get("method") {
            JSValue::Undefined | JSValue::Null => {}
            value => parts.method = value.to_string().to_ascii_uppercase(),
        }
    }
    if init.has_own_property("headers") {
        parts.headers = extract_header_entries(&init.get("headers"));
    }
    if init.has_own_property("body") {
        parts.body = match init.get("body") {
            JSValue::Undefined | JSValue::Null => Vec::new(),
            value => value.to_string().into_bytes(),
        };
    }
}

pub(crate) fn install_fetch(engine: &mut pixi_byte::JSEngine) {
    engine
        .global_mut()
        .borrow_mut()
        .set("fetch".to_string(), JSValue::NativeFunction(fetch));
}

const XHR_METHOD: &str = "__orinium_xhr_method";
const XHR_URL: &str = "__orinium_xhr_url";
const XHR_HEADERS: &str = "__orinium_xhr_headers";

pub(crate) fn install_xml_http_request(engine: &mut pixi_byte::JSEngine) {
    let mut constructor = JSObject::new();
    constructor.set(
        "__construct__".to_string(),
        JSValue::NativeFunction(xml_http_request_constructor),
    );
    engine.global_mut().borrow_mut().set(
        "XMLHttpRequest".to_string(),
        JSValue::Object(Rc::new(RefCell::new(constructor))),
    );
}

fn xml_http_request_constructor(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let mut xhr = JSObject::new();
    xhr.set("readyState".to_string(), JSValue::Number(0.0));
    xhr.set("status".to_string(), JSValue::Number(0.0));
    xhr.set("statusText".to_string(), JSValue::String(String::new()));
    xhr.set("responseText".to_string(), JSValue::String(String::new()));
    xhr.set("response".to_string(), JSValue::String(String::new()));
    xhr.set("responseType".to_string(), JSValue::String(String::new()));
    xhr.set("withCredentials".to_string(), JSValue::Boolean(false));
    xhr.set(
        XHR_HEADERS.to_string(),
        JSValue::Object(Rc::new(RefCell::new(JSObject::new()))),
    );
    xhr.set(
        "open".to_string(),
        JSValue::NativeFunction(xml_http_request_open),
    );
    xhr.set(
        "send".to_string(),
        JSValue::NativeFunction(xml_http_request_send),
    );
    xhr.set(
        "setRequestHeader".to_string(),
        JSValue::NativeFunction(xml_http_request_set_request_header),
    );
    xhr.set(
        "getAllResponseHeaders".to_string(),
        JSValue::NativeFunction(xml_http_request_get_all_response_headers),
    );
    // TODO: Cancel the in-flight network request and dispatch XMLHttpRequest abort events.
    xhr.set("abort".to_string(), JSValue::NativeFunction(noop));
    Ok(JSValue::Object(Rc::new(RefCell::new(xhr))))
}

fn xml_http_request_open(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(JSValue::Object(xhr)) = args.first() else {
        return Err(JSError::TypeError(
            "invalid XMLHttpRequest receiver".to_string(),
        ));
    };
    let method = args
        .get(1)
        .cloned()
        .unwrap_or(JSValue::String("GET".to_string()))
        .to_string();
    let url = args
        .get(2)
        .cloned()
        .unwrap_or(JSValue::Undefined)
        .to_string();
    let mut xhr = xhr.borrow_mut();
    xhr.set(
        XHR_METHOD.to_string(),
        JSValue::String(method.to_ascii_uppercase()),
    );
    xhr.set(XHR_URL.to_string(), JSValue::String(url));
    xhr.set("readyState".to_string(), JSValue::Number(1.0));
    Ok(JSValue::Undefined)
}

fn xml_http_request_set_request_header(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(JSValue::Object(xhr)) = args.first() else {
        return Err(JSError::TypeError(
            "invalid XMLHttpRequest receiver".to_string(),
        ));
    };
    let name = args
        .get(1)
        .cloned()
        .unwrap_or(JSValue::Undefined)
        .to_string();
    let value = args
        .get(2)
        .cloned()
        .unwrap_or(JSValue::Undefined)
        .to_string();
    if let JSValue::Object(headers) = xhr.borrow().get(XHR_HEADERS) {
        headers.borrow_mut().set(name, JSValue::String(value));
    }
    Ok(JSValue::Undefined)
}

fn xml_http_request_send(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(JSValue::Object(xhr)) = args.first() else {
        return Err(JSError::TypeError(
            "invalid XMLHttpRequest receiver".to_string(),
        ));
    };
    let (url, method, headers) = {
        let xhr_ref = xhr.borrow();
        let headers = match xhr_ref.get(XHR_HEADERS) {
            JSValue::Object(headers) => headers
                .borrow()
                .keys()
                .into_iter()
                .map(|name| {
                    let value = headers.borrow().get(&name).to_string();
                    (name, value)
                })
                .collect(),
            _ => Vec::new(),
        };
        (
            xhr_ref.get(XHR_URL).to_string(),
            xhr_ref.get(XHR_METHOD).to_string(),
            headers,
        )
    };
    let body = match args.get(1) {
        Some(JSValue::Undefined | JSValue::Null) | None => Vec::new(),
        Some(value) => value.to_string().into_bytes(),
    };
    with_host_mut(vm, |host| {
        host.next_fetch_id += 1;
        let id = host.next_fetch_id;
        host.xhr_requests.insert(id, Rc::clone(xhr));
        host.fetch_requests.push(JsFetchRequest {
            id,
            url,
            method,
            headers,
            body,
        });
    })
    .ok_or_else(|| JSError::InternalError("XMLHttpRequest host is unavailable".to_string()))?;
    Ok(JSValue::Undefined)
}

fn xml_http_request_get_all_response_headers(
    _vm: &mut VM,
    args: Vec<JSValue>,
) -> JSResult<JSValue> {
    let value = match args.first() {
        Some(JSValue::Object(xhr)) => xhr.borrow().get("__orinium_xhr_response_headers"),
        _ => JSValue::String(String::new()),
    };
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
        xhr.set("readyState".to_string(), JSValue::Number(4.0));
        xhr.set(
            "status".to_string(),
            JSValue::Number(response.status as f64),
        );
        xhr.set(
            "statusText".to_string(),
            JSValue::String(response.status_text),
        );
        xhr.set("responseURL".to_string(), JSValue::String(response.url));
        xhr.set("responseText".to_string(), JSValue::String(body.clone()));
        xhr.set("response".to_string(), JSValue::String(body));
        xhr.set(
            "__orinium_xhr_response_headers".to_string(),
            JSValue::String(headers),
        );
    }
    for name in ["onreadystatechange", "onload"] {
        let handler = xhr.borrow().get(name);
        if is_callable(&handler) {
            let _ = engine.call(handler, JSValue::Object(Rc::clone(&xhr)), Vec::new());
        }
    }
}

fn fetch(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let input = args.get(1).cloned().unwrap_or(JSValue::Undefined);
    let RequestParts {
        url,
        method,
        headers,
        body,
    } = request_parts(&input, args.get(2));
    let promise_constructor = vm.global_object.borrow().get("Promise");
    let JSValue::Object(constructor) = &promise_constructor else {
        return Err(JSError::InternalError(
            "Promise constructor is unavailable".to_string(),
        ));
    };
    let construct = constructor.borrow().get("__construct__");
    let _ = with_host_mut(vm, |host| host.constructing_fetch_capability = None);
    let promise = vm.call(
        construct,
        promise_constructor,
        vec![JSValue::NativeFunction(capture_fetch_capability)],
    )?;
    let capability = with_host_mut(vm, |host| host.constructing_fetch_capability.take())
        .flatten()
        .ok_or_else(|| JSError::InternalError("Failed to create fetch Promise".to_string()))?;

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
        });
    });
    Ok(promise)
}

fn capture_fetch_capability(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let resolve = args.get(1).cloned().unwrap_or(JSValue::Undefined);
    let reject = args.get(2).cloned().unwrap_or(JSValue::Undefined);
    let Some(()) = with_host_mut(vm, |host| {
        host.constructing_fetch_capability = Some(JsFetchCapability { resolve, reject });
    }) else {
        return Err(JSError::InternalError(
            "Fetch host state is unavailable".to_string(),
        ));
    };
    Ok(JSValue::Undefined)
}

pub(crate) fn make_fetch_response(response: JsFetchResponse) -> Rc<RefCell<JSObject>> {
    let body_bytes = response.body.clone();
    let mut object = JSObject::new();
    object.define_property(
        "headers".to_string(),
        Property::read_only(JSValue::Object(make_headers(response.headers, true))),
    );
    object.define_property(
        "ok".to_string(),
        Property::read_only(JSValue::Boolean((200..=299).contains(&response.status))),
    );
    object.define_property(
        "status".to_string(),
        Property::read_only(JSValue::Number(response.status as f64)),
    );
    object.define_property(
        "statusText".to_string(),
        Property::read_only(JSValue::String(response.status_text)),
    );
    object.define_property(
        "redirected".to_string(),
        Property::read_only(JSValue::Boolean(response.redirected)),
    );
    object.define_property(
        "bodyUsed".to_string(),
        Property {
            value: JSValue::Undefined,
            enumerable: true,
            writable: false,
            configurable: false,
            getter: Some(JSValue::NativeFunction(fetch_response_body_used)),
            setter: None,
        },
    );
    object.define_property(
        "url".to_string(),
        Property::read_only(JSValue::String(response.url)),
    );
    object.define_property(
        "text".to_string(),
        Property::read_only(JSValue::NativeFunction(fetch_response_text)),
    );
    object.define_property(
        "json".to_string(),
        Property::read_only(JSValue::NativeFunction(fetch_response_json)),
    );
    object.define_property(
        "arrayBuffer".to_string(),
        Property::read_only(JSValue::NativeFunction(fetch_response_array_buffer)),
    );
    object.set(
        "__orinium_response_body".to_string(),
        JSValue::String(String::from_utf8_lossy(&response.body).into_owned()),
    );
    object.set(RESPONSE_BODY_USED.to_string(), JSValue::Boolean(false));
    object.set(
        RESPONSE_BODY_BYTES.to_string(),
        JSArray::from_vec(
            body_bytes
                .into_iter()
                .map(|byte| JSValue::Number(byte as f64))
                .collect(),
        )
        .to_object(),
    );
    Rc::new(RefCell::new(object))
}

fn fetch_response_text(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let body = match consume_response_body(vm, &args, "text")? {
        Ok(body) => body,
        Err(rejection) => return Ok(rejection),
    };
    settle_promise(vm, "resolve", JSValue::String(body))
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
            JSValue::String(format!("Failed to parse JSON: {error}")),
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
    let Some(JSValue::Object(response)) = args.first() else {
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
    match response.get("__orinium_response_body") {
        JSValue::String(body) => Ok(Ok(body)),
        _ => Err(JSError::InternalError(
            "Response body is unavailable".to_string(),
        )),
    }
}

fn consume_response_object(
    vm: &mut VM,
    args: &[JSValue],
    method: &str,
) -> JSResult<Rc<RefCell<JSObject>>> {
    let Some(JSValue::Object(response)) = args.first() else {
        return Err(JSError::TypeError(format!(
            "Response.{method} called on incompatible receiver"
        )));
    };
    if matches!(
        response.borrow().get(RESPONSE_BODY_USED),
        JSValue::Boolean(true)
    ) {
        let rejection = settle_promise(
            vm,
            "reject",
            JSValue::String("Response body has already been consumed".to_string()),
        )?;
        return Err(JSError::Thrown(rejection));
    }
    response
        .borrow_mut()
        .set(RESPONSE_BODY_USED.to_string(), JSValue::Boolean(true));
    Ok(Rc::clone(response))
}

fn settle_promise(vm: &mut VM, method: &str, value: JSValue) -> JSResult<JSValue> {
    let promise = vm.global_object.borrow().get("Promise");
    let JSValue::Object(constructor) = &promise else {
        return Err(JSError::InternalError(
            "Promise constructor is unavailable".to_string(),
        ));
    };
    let settle = constructor.borrow().get(method);
    vm.call(settle, promise, vec![value])
}

fn json_to_js_value(value: serde_json::Value) -> JSValue {
    match value {
        serde_json::Value::Null => JSValue::Null,
        serde_json::Value::Bool(value) => JSValue::Boolean(value),
        serde_json::Value::Number(value) => JSValue::Number(value.as_f64().unwrap_or(f64::NAN)),
        serde_json::Value::String(value) => JSValue::String(value),
        serde_json::Value::Array(values) => {
            JSArray::from_vec(values.into_iter().map(json_to_js_value).collect()).to_object()
        }
        serde_json::Value::Object(properties) => {
            let mut object = JSObject::new();
            for (key, value) in properties {
                object.set(key, json_to_js_value(value));
            }
            JSValue::Object(Rc::new(RefCell::new(object)))
        }
    }
}
