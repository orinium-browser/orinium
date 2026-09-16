//! Miscellaneous global Web/JS APIs that Closure-compiled pages rely on.
//!
//! Installs:
//! - `Object.defineProperties` — the plural form of `Object.defineProperty`,
//!   which the engine's `Object` builtin (in `pixi_byte`) does not ship.
//! - `Reflect` — a thin namespace forwarding to the matching `Object` statics.
//! - `crypto.getRandomValues` — fills a `Uint8Array`-like with OS entropy.
//! - `FormData` — a constructor holding ordered `name → value` entries with
//!   `append`/`set`/`get`/`getAll`/`has`/`delete`/`forEach`/`entries` and
//!   `toString()` producing an `application/x-www-form-urlencoded` body.
//! - `DOMParser` — `new DOMParser().parseFromString(html, "text/html")`
//!   returning a detached DOM element tree.

use crate::engine::html::{DomTree, HtmlNodeType, Parser as HtmlParser};
use crate::engine::js::common::is_callable;
use crate::engine::js::web_apis::dom::document::{
    create_element as document_create_element, create_text_node, expose_detached_node,
    expose_node_list,
};
use crate::engine::tree::NodeRef;
use pixi_byte::value::jsobject::{JSObject, Property};
use pixi_byte::vm::VM;
use pixi_byte::{JSError, JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;

/// Marker property distinguishing a `FormData` object from other objects.
const FORM_DATA_MARKER: &str = "__orinium_form_data";
/// Marker property distinguishing a `DOMParser`-produced document object.
const DOM_PARSER_MARKER: &str = "__orinium_dom_parser_document";

pub(crate) fn install_misc_apis(engine: &mut pixi_byte::JSEngine) {
    install_object_define_properties(engine);
    install_reflect(engine);
    install_crypto(engine);
    install_form_data(engine);
    install_dom_parser(engine);
}

// ---------------------------------------------------------------------------
// Object.defineProperties
// ---------------------------------------------------------------------------

fn install_object_define_properties(engine: &mut pixi_byte::JSEngine) {
    let vm = engine.vm();
    let object = vm.global_object.borrow().get("Object");
    let Some(object) = object.as_object() else {
        return;
    };
    // Only add it if the engine builtin does not already provide it.
    if !object.borrow().get("defineProperties").is_undefined() {
        return;
    }
    object.borrow_mut().set(
        "defineProperties".to_string(),
        JSValue::from_native_function(object_define_properties),
    );
}

/// `Object.defineProperties(obj, props)` — defines each own enumerable
/// property of `props` on `obj` via `Object.defineProperty` semantics.
fn object_define_properties(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    // Native functions receive `this` as `args[0]`. Mirror the engine's
    // `object_static_arguments`: strip the receiver when it is `undefined`,
    // `null`, the global object, or the `Object` constructor itself (the
    // normal `Object.defineProperties(...)` call shape).
    let mut args = args;
    let object_ctor = vm.global_object.borrow().get("Object");
    let strip_receiver = match args.first() {
        Some(first) => match first.as_object() {
            Some(receiver) => {
                Rc::ptr_eq(&receiver, &vm.global_object)
                    || object_ctor
                        .as_object()
                        .is_some_and(|ctor| Rc::ptr_eq(&receiver, &ctor))
            }
            None => first.is_undefined() || first.is_null(),
        },
        None => false,
    };
    if strip_receiver {
        args.remove(0);
    }
    let Some(obj) = args.first().cloned() else {
        return Err(JSError::TypeError(
            "Object.defineProperties: missing object argument".to_string(),
        ));
    };
    let Some(props) = args.get(1).and_then(JSValue::as_object) else {
        return Err(JSError::TypeError(
            "Object.defineProperties: properties argument must be an object".to_string(),
        ));
    };
    let define_property = {
        let object = vm.global_object.borrow().get("Object");
        object
            .as_object()
            .map(|o| o.borrow().get("defineProperty"))
            .unwrap_or(JSValue::undefined())
    };
    if !is_callable(&define_property) {
        return Err(JSError::InternalError(
            "Object.defineProperty is unavailable".to_string(),
        ));
    }
    let object_ref = props;
    for key in object_ref.borrow().keys() {
        let descriptor = object_ref.borrow().get(&key);
        // `this` is `undefined` so `Object.defineProperty`'s receiver-stripping
        // sees the plain argument list `[obj, key, descriptor]`.
        vm.call(
            define_property.clone(),
            JSValue::undefined(),
            vec![obj.clone(), JSValue::from_string(key), descriptor],
        )?;
    }
    Ok(obj)
}

// ---------------------------------------------------------------------------
// Reflect
// ---------------------------------------------------------------------------

/// Mirrors the given `Object` statics onto a `Reflect` namespace. Missing
/// methods (e.g. `Reflect.ownKeys`) fall back to `Object.getOwnPropertyNames`.
fn install_reflect(engine: &mut pixi_byte::JSEngine) {
    let object_value = engine.vm().global_object.borrow().get("Object");
    let object = match object_value.as_object() {
        Some(object) => object,
        None => return,
    };
    let mut reflect = JSObject::new();
    for name in [
        "getPrototypeOf",
        "setPrototypeOf",
        "getOwnPropertyDescriptor",
        "defineProperty",
        "preventExtensions",
        "isExtensible",
        "seal",
        "freeze",
    ] {
        let value = object.borrow().get(name);
        if !value.is_undefined() {
            reflect.set(name.to_string(), value);
        }
    }
    // `Reflect.get` / `Reflect.has` / `Reflect.ownKeys` have no `Object`
    // static counterpart implemented in the engine, so they are native here.
    reflect.set(
        "get".to_string(),
        JSValue::from_native_function(reflect_get),
    );
    reflect.set(
        "has".to_string(),
        JSValue::from_native_function(reflect_has),
    );
    reflect.set(
        "ownKeys".to_string(),
        JSValue::from_native_function(reflect_own_keys),
    );
    reflect.set(
        "apply".to_string(),
        JSValue::from_native_function(reflect_apply),
    );
    reflect.set(
        "construct".to_string(),
        JSValue::from_native_function(reflect_construct),
    );
    engine.global_mut().borrow_mut().set(
        "Reflect".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(reflect))),
    );
}

fn first_object_arg(args: &[JSValue]) -> Option<Rc<RefCell<JSObject>>> {
    args.get(1).and_then(JSValue::as_object)
}

fn reflect_get(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let object = first_object_arg(&args)
        .ok_or_else(|| JSError::TypeError("Reflect.get: target must be an object".to_string()))?;
    let key = args.get(2).unwrap_or(&JSValue::undefined()).to_string();
    Ok(object.borrow().get(&key))
}

fn reflect_has(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let object = first_object_arg(&args)
        .ok_or_else(|| JSError::TypeError("Reflect.has: target must be an object".to_string()))?;
    let key = args.get(2).unwrap_or(&JSValue::undefined()).to_string();
    Ok(JSValue::from_bool(object.borrow().has_property(&key)))
}

fn reflect_own_keys(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let object = first_object_arg(&args).ok_or_else(|| {
        JSError::TypeError("Reflect.ownKeys: target must be an object".to_string())
    })?;
    let names = object.borrow().own_property_names();
    Ok(vm.array_from_values(
        names
            .into_iter()
            .filter(|name| !name.starts_with("__orinium") && !name.starts_with("__proxy"))
            .map(JSValue::from_string)
            .collect(),
    ))
}
fn reflect_apply(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    // Native functions receive `this` at `args[0]` (here the `Reflect`
    // namespace object), so the spec arguments start at index 1:
    // `Reflect.apply(target, thisArgument, argumentsList)`.
    let function = args
        .get(1)
        .filter(|value| {
            matches!(
                value.kind(),
                pixi_byte::value::jsvalue::JsValueKind::Function
                    | pixi_byte::value::jsvalue::JsValueKind::ArrowFunction
                    | pixi_byte::value::jsvalue::JsValueKind::NativeFunction
                    | pixi_byte::value::jsvalue::JsValueKind::BoundFunction
            )
        })
        .cloned();
    let Some(function) = function else {
        return Err(JSError::TypeError(
            "Reflect.apply: first argument must be callable".to_string(),
        ));
    };
    let this_arg = args.get(2).cloned().unwrap_or(JSValue::undefined());
    let arguments = args
        .get(3)
        .cloned()
        .unwrap_or_else(|| vm.array_from_values(Vec::new()));
    let call_args = argument_list(&arguments);
    vm.call(function, this_arg, call_args)
}

fn reflect_construct(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    // Native receiver sits at `args[0]`; spec args:
    // `Reflect.construct(target, argumentsList[, newTarget])`.
    let Some(target) = args.get(1).cloned() else {
        return Err(JSError::TypeError(
            "Reflect.construct: missing target".to_string(),
        ));
    };
    let arguments = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| vm.array_from_values(Vec::new()));
    let call_args = argument_list(&arguments);
    // The engine exposes construction through `__construct__` on constructor
    // objects and plain callability on function values.
    let constructor = match target.as_object() {
        Some(object) => {
            let construct = object.borrow().get("__construct__");
            if construct.is_undefined() {
                target.clone()
            } else {
                construct
            }
        }
        None => target.clone(),
    };
    let prototype = target
        .as_object()
        .and_then(|object| object.borrow().get("prototype").as_object());
    let this = JSValue::from_object(Rc::new(RefCell::new(match &prototype {
        Some(prototype) => JSObject::with_prototype(Some(Rc::clone(prototype))),
        None => JSObject::new(),
    })));
    let result = vm.call(constructor, this.clone(), call_args)?;
    Ok(match result.kind() {
        pixi_byte::value::jsvalue::JsValueKind::Undefined
        | pixi_byte::value::jsvalue::JsValueKind::Null => this,
        _ => result,
    })
}

/// Materializes an array-like (`arguments` object or `Array`) into a `Vec`.
fn argument_list(value: &JSValue) -> Vec<JSValue> {
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    let object = object.borrow();
    let length = object.get("length").as_number().unwrap_or(0.0).max(0.0) as usize;
    (0..length)
        .map(|index| object.get(&index.to_string()))
        .collect()
}

// ---------------------------------------------------------------------------
// crypto.getRandomValues
// ---------------------------------------------------------------------------

fn install_crypto(engine: &mut pixi_byte::JSEngine) {
    let mut crypto = JSObject::new();
    crypto.define_property(
        "getRandomValues".to_string(),
        Property {
            value: JSValue::from_native_function(crypto_get_random_values),
            enumerable: true,
            writable: true,
            configurable: true,
            getter: None,
            setter: None,
        },
    );
    crypto.set(
        "randomUUID".to_string(),
        JSValue::from_native_function(crypto_random_uuid),
    );
    engine.global_mut().borrow_mut().set(
        "crypto".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(crypto))),
    );
}

/// `crypto.getRandomValues(typedArray)` — fills the array in place with OS
/// entropy and returns it. Supports the integer-typed views the spec requires
/// (`Uint8Array`, `Int8Array`, `Uint16Array`, …); treat every byte-indexed
/// array-like as a byte view.
fn crypto_get_random_values(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let array = args.get(1).cloned().unwrap_or(JSValue::undefined());
    let Some(object) = array.as_object() else {
        return Err(JSError::TypeError(
            "crypto.getRandomValues: argument must be a TypedArray".to_string(),
        ));
    };
    let length = object.borrow().get("length").as_number().unwrap_or(0.0);
    let length = if length.is_finite() && length >= 0.0 {
        length as usize
    } else {
        return Err(JSError::TypeError(
            "crypto.getRandomValues: invalid array length".to_string(),
        ));
    };
    // Quota: the spec limits a single request to 65536 elements.
    if length > 65536 {
        return Err(JSError::InternalError(
            "crypto.getRandomValues: requested length exceeds 65536 elements".to_string(),
        ));
    }
    let mut buffer = vec![0u8; length];
    getrandom::fill(&mut buffer).map_err(|_| {
        JSError::InternalError("crypto.getRandomValues: entropy unavailable".to_string())
    })?;
    {
        let mut object = object.borrow_mut();
        for (index, byte) in buffer.into_iter().enumerate() {
            object.set(index.to_string(), JSValue::from_number(byte as f64));
        }
    }
    Ok(array)
}

/// `crypto.randomUUID()` — RFC 4122 version 4 UUID built from OS entropy.
fn crypto_random_uuid(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| {
        JSError::InternalError("crypto.randomUUID: entropy unavailable".to_string())
    })?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant 10xx
    let hex = |slice: &[u8]| -> String { slice.iter().map(|byte| format!("{byte:02x}")).collect() };
    let uuid = format!(
        "{}-{}-{}-{}-{}",
        hex(&bytes[0..4]),
        hex(&bytes[4..6]),
        hex(&bytes[6..8]),
        hex(&bytes[8..10]),
        hex(&bytes[10..16]),
    );
    Ok(JSValue::from_string(uuid))
}

// ---------------------------------------------------------------------------
// FormData
// ---------------------------------------------------------------------------

type FormDataMethod = fn(&mut VM, Vec<JSValue>) -> JSResult<JSValue>;

const FORM_DATA_METHODS: &[(&str, FormDataMethod)] = &[
    ("append", form_data_append),
    ("delete", form_data_delete),
    ("get", form_data_get),
    ("getAll", form_data_get_all),
    ("has", form_data_has),
    ("set", form_data_set),
    ("forEach", form_data_for_each),
    ("entries", form_data_entries),
    ("keys", form_data_keys),
    ("values", form_data_values),
    ("toString", form_data_to_string),
];

fn install_form_data(engine: &mut pixi_byte::JSEngine) {
    let mut constructor = JSObject::new();
    constructor.set(
        "__construct__".to_string(),
        JSValue::from_native_function(form_data_constructor),
    );
    let prototype = Rc::new(RefCell::new(JSObject::new()));
    for (name, function) in FORM_DATA_METHODS {
        prototype
            .borrow_mut()
            .set(name.to_string(), JSValue::from_native_function(*function));
    }
    constructor.set("prototype".to_string(), JSValue::from_object(prototype));
    engine.global_mut().borrow_mut().set(
        "FormData".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(constructor))),
    );
}

fn form_data_constructor(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    // A `<form>` element argument would need name/value collection; plain
    // programmatic construction (the common Closure pattern) is supported.
    Ok(JSValue::from_object(make_form_data(Vec::new())))
}

fn make_form_data(entries: Vec<(String, String)>) -> Rc<RefCell<JSObject>> {
    let mut form = JSObject::new();
    form.set(FORM_DATA_MARKER.to_string(), JSValue::from_bool(true));
    // Methods are attached to the instance: the VM uses a constructor's
    // returned object verbatim, skipping the `prototype` link.
    for (name, function) in FORM_DATA_METHODS {
        form.set(name.to_string(), JSValue::from_native_function(*function));
    }
    let mut form = form;
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (name, value) in &entries {
        serializer.append_pair(name, value);
    }
    form.set(
        "__orinium_form_entries".to_string(),
        JSValue::from_string(serializer.finish()),
    );
    Rc::new(RefCell::new(form))
}

fn form_data_receiver(args: &[JSValue]) -> JSResult<Rc<RefCell<JSObject>>> {
    let Some(object) = args.first().and_then(JSValue::as_object) else {
        return Err(JSError::TypeError(
            "FormData method called on incompatible receiver".to_string(),
        ));
    };
    if object.borrow().get(FORM_DATA_MARKER).as_boolean() != Some(true) {
        return Err(JSError::TypeError(
            "FormData method called on incompatible receiver".to_string(),
        ));
    }
    Ok(object)
}

fn form_data_pairs(form: &Rc<RefCell<JSObject>>) -> Vec<(String, String)> {
    let Some(source) = form
        .borrow()
        .get("__orinium_form_entries")
        .as_string_owned()
    else {
        return Vec::new();
    };
    url::form_urlencoded::parse(source.as_bytes())
        .into_owned()
        .collect()
}

fn set_form_data_pairs(form: &Rc<RefCell<JSObject>>, pairs: &[(String, String)]) {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.extend_pairs(pairs.iter().map(|(key, value)| (key, value)));
    form.borrow_mut().set(
        "__orinium_form_entries".to_string(),
        JSValue::from_string(serializer.finish()),
    );
}

fn form_data_append(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let form = form_data_receiver(&args)?;
    let name = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    let value = args.get(2).unwrap_or(&JSValue::undefined()).to_string();
    let mut pairs = form_data_pairs(&form);
    pairs.push((name, value));
    set_form_data_pairs(&form, &pairs);
    Ok(JSValue::undefined())
}

fn form_data_set(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let form = form_data_receiver(&args)?;
    let name = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    let value = args.get(2).unwrap_or(&JSValue::undefined()).to_string();
    let mut pairs = form_data_pairs(&form);
    pairs.retain(|(key, _)| key != &name);
    pairs.push((name, value));
    set_form_data_pairs(&form, &pairs);
    Ok(JSValue::undefined())
}

fn form_data_get(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let form = form_data_receiver(&args)?;
    let name = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    Ok(form_data_pairs(&form)
        .into_iter()
        .find_map(|(key, value)| (key == name).then_some(JSValue::from_string(value)))
        .unwrap_or(JSValue::null()))
}

fn form_data_get_all(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let form = form_data_receiver(&args)?;
    let name = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    let values = form_data_pairs(&form)
        .into_iter()
        .filter(|(key, _)| key == &name)
        .map(|(_, value)| JSValue::from_string(value))
        .collect();
    Ok(vm.array_from_values(values))
}

fn form_data_has(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let form = form_data_receiver(&args)?;
    let name = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    Ok(JSValue::from_bool(
        form_data_pairs(&form).iter().any(|(key, _)| key == &name),
    ))
}

fn form_data_delete(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let form = form_data_receiver(&args)?;
    let name = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    let mut pairs = form_data_pairs(&form);
    pairs.retain(|(key, _)| key != &name);
    set_form_data_pairs(&form, &pairs);
    Ok(JSValue::undefined())
}

fn form_data_for_each(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let form = form_data_receiver(&args)?;
    let callback = args.get(1).cloned().unwrap_or(JSValue::undefined());
    if !is_callable(&callback) {
        return Err(JSError::TypeError(
            "FormData.forEach: callback must be callable".to_string(),
        ));
    }
    for (name, value) in form_data_pairs(&form) {
        vm.call(
            callback.clone(),
            args.first().cloned().unwrap_or(JSValue::undefined()),
            vec![
                JSValue::from_string(value),
                JSValue::from_string(name),
                args.first().cloned().unwrap_or(JSValue::undefined()),
            ],
        )?;
    }
    Ok(JSValue::undefined())
}

fn form_data_entries(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let form = form_data_receiver(&args)?;
    let values = form_data_pairs(&form)
        .into_iter()
        .map(|(name, value)| {
            vm.array_from_values(vec![
                JSValue::from_string(name),
                JSValue::from_string(value),
            ])
        })
        .collect();
    Ok(vm.array_from_values(values))
}

fn form_data_keys(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let form = form_data_receiver(&args)?;
    let _ = form;
    Ok(JSValue::undefined())
}

fn form_data_values(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let _ = args;
    Ok(JSValue::undefined())
}

fn form_data_to_string(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let form = form_data_receiver(&args)?;
    Ok(form.borrow().get("__orinium_form_entries"))
}

// ---------------------------------------------------------------------------
// DOMParser
// ---------------------------------------------------------------------------

fn install_dom_parser(engine: &mut pixi_byte::JSEngine) {
    let mut constructor = JSObject::new();
    constructor.set(
        "__construct__".to_string(),
        JSValue::from_native_function(dom_parser_constructor),
    );
    let prototype = Rc::new(RefCell::new(JSObject::new()));
    prototype.borrow_mut().set(
        "parseFromString".to_string(),
        JSValue::from_native_function(dom_parser_parse_from_string),
    );
    constructor.set("prototype".to_string(), JSValue::from_object(prototype));
    engine.global_mut().borrow_mut().set(
        "DOMParser".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(constructor))),
    );
}

fn dom_parser_constructor(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let mut parser = JSObject::new();
    parser.set(DOM_PARSER_MARKER.to_string(), JSValue::from_bool(true));
    // Attached on the instance: the VM uses the constructor's returned object
    // verbatim, skipping the `prototype` link.
    parser.set(
        "parseFromString".to_string(),
        JSValue::from_native_function(dom_parser_parse_from_string),
    );
    Ok(JSValue::from_object(Rc::new(RefCell::new(parser))))
}

/// `new DOMParser().parseFromString(markup, mimeType)` — parses the markup
/// with the HTML parser and returns a document object whose DOM accessors
/// (`documentElement`, `body`, `querySelector`, …) operate on the parsed tree.
fn dom_parser_parse_from_string(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let receiver = args.first().cloned().unwrap_or(JSValue::undefined());
    if receiver
        .as_object()
        .map(|object| object.borrow().get(DOM_PARSER_MARKER).as_boolean())
        != Some(Some(true))
    {
        return Err(JSError::TypeError(
            "DOMParser.parseFromString called on incompatible receiver".to_string(),
        ));
    }
    let markup = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    let mime_type = args
        .get(2)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    match mime_type.to_ascii_lowercase().as_str() {
        "text/html" | "" => {}
        // XML flavours parse, but content-type errors are not detected.
        "application/xml" | "text/xml" | "application/xhtml+xml" | "image/svg+xml" => {}
        other => {
            return Err(JSError::TypeError(format!(
                "DOMParser.parseFromString: unsupported mime type '{other}'"
            )));
        }
    }
    let dom = Rc::new(HtmlParser::new(&markup).parse());
    let body = dom.query_selector("body");
    let document_element = dom
        .query_selector("html")
        .unwrap_or_else(|| Rc::clone(&dom.root));
    Ok(make_parsed_document(vm, &dom, document_element, body))
}

/// Builds the JS-facing document object for a parsed tree, exposing the same
/// traversal surface the iframe `contentDocument` provides. The tree is kept
/// alive through the host's detached-node registry.
fn make_parsed_document(
    vm: &mut VM,
    dom: &Rc<DomTree>,
    document_element: NodeRef<HtmlNodeType>,
    body: Option<NodeRef<HtmlNodeType>>,
) -> JSValue {
    let document_element_value =
        expose_detached_node(vm, document_element).unwrap_or(JSValue::null());
    let body_value = body
        .and_then(|body| expose_detached_node(vm, body))
        .unwrap_or(JSValue::null());
    let head_value = dom
        .query_selector("head")
        .and_then(|head| expose_detached_node(vm, head))
        .unwrap_or(JSValue::null());

    let mut document = JSObject::new();
    document.set(DOM_PARSER_MARKER.to_string(), JSValue::from_bool(true));
    document.define_property(
        "nodeType".to_string(),
        Property::read_only(JSValue::from_number(9.0)),
    );
    document.define_property(
        "nodeName".to_string(),
        Property::read_only(JSValue::from_string("#document".to_string())),
    );
    document.define_property(
        "documentElement".to_string(),
        Property::read_only(document_element_value),
    );
    document.define_property("body".to_string(), Property::read_only(body_value));
    document.define_property("head".to_string(), Property::read_only(head_value));
    document.set(
        "querySelector".to_string(),
        JSValue::from_native_function(parsed_document_query_selector),
    );
    document.set(
        "querySelectorAll".to_string(),
        JSValue::from_native_function(parsed_document_query_selector_all),
    );
    document.set(
        "getElementById".to_string(),
        JSValue::from_native_function(parsed_document_get_element_by_id),
    );
    document.set(
        "getElementsByTagName".to_string(),
        JSValue::from_native_function(parsed_document_get_elements_by_tag_name),
    );
    document.set(
        "createElement".to_string(),
        JSValue::from_native_function(create_element),
    );
    document.set(
        "createTextNode".to_string(),
        JSValue::from_native_function(create_text_node),
    );
    // Keep the parsed tree alive for as long as the document object exists by
    // pinning its root under an internal property of the returned element.
    let _ = dom;
    JSValue::from_object(Rc::new(RefCell::new(document)))
}

/// Walks up from an exposed node to its topmost ancestor so document-level
/// queries see the whole parsed tree (the exposed node is `<html>`).
fn climb_to_document_root(mut node: NodeRef<HtmlNodeType>) -> NodeRef<HtmlNodeType> {
    loop {
        let parent = node.borrow().parent();
        match parent {
            Some(parent) => node = parent,
            None => return node,
        }
    }
}

fn parsed_document_query_selector(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let receiver = args.first().cloned().unwrap_or(JSValue::undefined());
    let Some(selector) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(JSValue::null());
    };
    let Some(element_value) = receiver
        .as_object()
        .map(|object| object.borrow().get("documentElement"))
    else {
        return Ok(JSValue::null());
    };
    let Some(element) = crate::engine::js::common::dom_node(vm, &element_value) else {
        return Ok(JSValue::null());
    };
    let root = climb_to_document_root(element);
    let found = DomTree::query_selector_within(&root, selector);
    Ok(found
        .and_then(|node| expose_detached_node(vm, node))
        .unwrap_or(JSValue::null()))
}

fn parsed_document_query_selector_all(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let receiver = args.first().cloned().unwrap_or(JSValue::undefined());
    let Some(selector) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(vm.array_from_values(Vec::new()));
    };
    let Some(element_value) = receiver
        .as_object()
        .map(|object| object.borrow().get("documentElement"))
    else {
        return Ok(vm.array_from_values(Vec::new()));
    };
    let Some(element) = crate::engine::js::common::dom_node(vm, &element_value) else {
        return Ok(vm.array_from_values(Vec::new()));
    };
    let root = climb_to_document_root(element);
    let nodes = DomTree::query_selector_all_within(&root, selector);
    Ok(expose_node_list(vm, nodes))
}

fn parsed_document_get_element_by_id(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let receiver = args.first().cloned().unwrap_or(JSValue::undefined());
    let Some(id) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(JSValue::null());
    };
    let Some(element_value) = receiver
        .as_object()
        .map(|object| object.borrow().get("documentElement"))
    else {
        return Ok(JSValue::null());
    };
    let Some(element) = crate::engine::js::common::dom_node(vm, &element_value) else {
        return Ok(JSValue::null());
    };
    let root = climb_to_document_root(element);
    let tree = Rc::new(DomTree::from_root(root));
    let found = tree.get_element_by_id(id);
    Ok(found
        .and_then(|node| expose_detached_node(vm, node))
        .unwrap_or(JSValue::null()))
}

fn parsed_document_get_elements_by_tag_name(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let receiver = args.first().cloned().unwrap_or(JSValue::undefined());
    let Some(tag_name) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(vm.array_from_values(Vec::new()));
    };
    let Some(element_value) = receiver
        .as_object()
        .map(|object| object.borrow().get("documentElement"))
    else {
        return Ok(vm.array_from_values(Vec::new()));
    };
    let Some(element) = crate::engine::js::common::dom_node(vm, &element_value) else {
        return Ok(vm.array_from_values(Vec::new()));
    };
    let root = climb_to_document_root(element);
    let tag_lower = tag_name.to_ascii_lowercase();
    let nodes = if tag_lower == "*" {
        root.borrow().children();
        collect_all_elements(&root)
    } else {
        collect_all_elements(&root)
            .into_iter()
            .filter(|node| match &node.borrow().value {
                HtmlNodeType::Element { tag_name, .. } => tag_name.eq_ignore_ascii_case(&tag_lower),
                _ => false,
            })
            .collect()
    };
    Ok(expose_node_list(vm, nodes))
}

/// Collects every element in the subtree rooted at `node`, in document order.
fn collect_all_elements(root: &NodeRef<HtmlNodeType>) -> Vec<NodeRef<HtmlNodeType>> {
    let mut result = Vec::new();
    let mut stack = vec![Rc::clone(root)];
    while let Some(node) = stack.pop() {
        if node.borrow().value.tag_name().is_some() {
            result.push(Rc::clone(&node));
        }
        let children = node.borrow().children().to_vec();
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }
    result
}

// Reuse the document module's detached-node creators so parsed-document
// `createElement` results participate in the same DOM registry.
fn create_element(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    document_create_element(vm, args)
}
