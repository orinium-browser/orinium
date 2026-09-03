use pixi_byte::value::jsobject::{JSObject, Property};
use pixi_byte::vm::VM;
use pixi_byte::{JSError, JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;

/// Static constants for DOMException error codes.
const INDEX_SIZE_ERR: f64 = 1.0;
const DOMSTRING_SIZE_ERR: f64 = 2.0;
const HIERARCHY_REQUEST_ERR: f64 = 3.0;
const WRONG_DOCUMENT_ERR: f64 = 4.0;
const INVALID_CHARACTER_ERR: f64 = 5.0;
const NOT_FOUND_ERR: f64 = 8.0;
const NOT_SUPPORTED_ERR: f64 = 9.0;
const INVALID_STATE_ERR: f64 = 11.0;
const SYNTAX_ERR: f64 = 12.0;
const INVALID_MODIFICATION_ERR: f64 = 13.0;
const NAMESPACE_ERR: f64 = 14.0;
const INVALID_ACCESS_ERR: f64 = 15.0;
const TYPE_MISMATCH_ERR: f64 = 17.0;

/// Maps a DOMException name string to its numeric code.
fn name_to_code(name: &str) -> f64 {
    match name {
        "IndexSizeError" => INDEX_SIZE_ERR,
        "DOMStringSizeError" => DOMSTRING_SIZE_ERR,
        "HierarchyRequestError" => HIERARCHY_REQUEST_ERR,
        "WrongDocumentError" => WRONG_DOCUMENT_ERR,
        "InvalidCharacterError" => INVALID_CHARACTER_ERR,
        "NotFoundError" => NOT_FOUND_ERR,
        "NotSupportedError" => NOT_SUPPORTED_ERR,
        "InvalidStateError" => INVALID_STATE_ERR,
        "SyntaxError" => SYNTAX_ERR,
        "InvalidModificationError" => INVALID_MODIFICATION_ERR,
        "NamespaceError" => NAMESPACE_ERR,
        "TypeMismatchError" => TYPE_MISMATCH_ERR,
        _ => 0.0,
    }
}

/// Creates a DOMException constructor object and returns it.
///
/// The prototype chain is set up so that `DOMException.prototype` inherits
/// from `Error.prototype`, enabling `instanceof Error` checks.
pub(crate) fn make_dom_exception_constructor() -> Rc<RefCell<JSObject>> {
    let mut prototype = JSObject::new();
    prototype.set(
        "toString".to_string(),
        JSValue::from_native_function(dom_exception_to_string),
    );
    let prototype = Rc::new(RefCell::new(prototype));

    let mut constructor = JSObject::new();
    constructor.set(
        "__construct__".to_string(),
        JSValue::from_native_function(dom_exception_construct),
    );
    constructor.set(
        "prototype".to_string(),
        JSValue::from_object(Rc::clone(&prototype)),
    );

    // Static constants on the constructor itself
    define_exception_constants(&mut constructor);

    Rc::new(RefCell::new(constructor))
}

/// Defines the legacy `DOMException` numeric code constants on an object so
/// they are reachable from any thrown exception instance.
fn define_exception_constants(obj: &mut JSObject) {
    for (name, value) in [
        ("INDEX_SIZE_ERR", INDEX_SIZE_ERR),
        ("DOMSTRING_SIZE_ERR", DOMSTRING_SIZE_ERR),
        ("HIERARCHY_REQUEST_ERR", HIERARCHY_REQUEST_ERR),
        ("WRONG_DOCUMENT_ERR", WRONG_DOCUMENT_ERR),
        ("INVALID_CHARACTER_ERR", INVALID_CHARACTER_ERR),
        ("NO_DATA_ALLOWED_ERR", 6.0),
        ("NO_MODIFICATION_ALLOWED_ERR", 7.0),
        ("NOT_FOUND_ERR", NOT_FOUND_ERR),
        ("NOT_SUPPORTED_ERR", NOT_SUPPORTED_ERR),
        ("INUSE_ATTRIBUTE_ERR", 10.0),
        ("INVALID_STATE_ERR", INVALID_STATE_ERR),
        ("SYNTAX_ERR", SYNTAX_ERR),
        ("INVALID_MODIFICATION_ERR", INVALID_MODIFICATION_ERR),
        ("NAMESPACE_ERR", NAMESPACE_ERR),
        ("INVALID_ACCESS_ERR", INVALID_ACCESS_ERR),
        ("VALIDATION_ERR", 16.0),
        ("TYPE_MISMATCH_ERR", TYPE_MISMATCH_ERR),
    ] {
        obj.define_property(
            name.to_string(),
            Property::read_only(JSValue::from_number(value)),
        );
    }
}

/// `new DOMException(message?, name?)`
fn dom_exception_construct(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let this = args.first().cloned().unwrap_or(JSValue::undefined());
    let Some(obj) = this.as_object() else {
        return Ok(JSValue::undefined());
    };

    let message = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    let name = args
        .get(2)
        .map(JSValue::to_console_string)
        .unwrap_or_else(|| "Error".to_string());

    let code = name_to_code(&name);

    let mut obj = obj.borrow_mut();
    define_exception_constants(&mut obj);
    obj.define_property(
        "name".to_string(),
        Property::read_only(JSValue::from_string(name)),
    );
    obj.define_property(
        "message".to_string(),
        Property::read_only(JSValue::from_string(message)),
    );
    obj.define_property(
        "code".to_string(),
        Property::read_only(JSValue::from_number(code)),
    );

    Ok(this)
}

/// `DOMException.prototype.toString()`
fn dom_exception_to_string(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let this = args.first().cloned().unwrap_or(JSValue::undefined());
    let Some(obj) = this.as_object() else {
        return Ok(JSValue::from_string("Error".to_string()));
    };
    let name = obj
        .borrow()
        .get("name")
        .as_string_owned()
        .unwrap_or_else(|| "Error".to_string());
    let message = obj
        .borrow()
        .get("message")
        .as_string_owned()
        .unwrap_or_default();
    if message.is_empty() {
        Ok(JSValue::from_string(name))
    } else {
        Ok(JSValue::from_string(format!("{name}: {message}")))
    }
}

/// Builds a DOMException error object and returns a `JSError::Thrown` for use
/// in native function implementations that need to throw a DOMException.
///
/// This constructs the object directly (without calling the JS constructor) so
/// it can be used from Rust code without a VM reference for the constructor
/// lookup.
pub(crate) fn throw_dom_exception(message: &str, name: &str) -> JSError {
    let mut obj = JSObject::new();
    define_exception_constants(&mut obj);
    obj.define_property(
        "name".to_string(),
        Property::read_only(JSValue::from_string(name.to_string())),
    );
    obj.define_property(
        "message".to_string(),
        Property::read_only(JSValue::from_string(message.to_string())),
    );
    obj.define_property(
        "code".to_string(),
        Property::read_only(JSValue::from_number(name_to_code(name))),
    );
    JSError::Thrown(JSValue::from_object(Rc::new(RefCell::new(obj))))
}
