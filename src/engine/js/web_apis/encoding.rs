use base64::Engine as _;
use pixi_byte::value::jsobject::{JSObject, Property};
use pixi_byte::value::jsvalue::JsValueKind;
use pixi_byte::vm::VM;
use pixi_byte::{JSError, JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;

type JsObjectRef = Rc<RefCell<JSObject>>;
type DataViewResult = JSResult<(JsObjectRef, Vec<u8>, usize, bool, usize, usize)>;

const INSTANCE_TAG: &str = "__instance_tag__";

fn internal_property(value: JSValue) -> Property {
    Property {
        value,
        enumerable: false,
        writable: false,
        configurable: false,
        getter: None,
        setter: None,
    }
}

fn read_instance_tag(value: &JSValue) -> Option<String> {
    value.as_object().and_then(|object| {
        object
            .borrow()
            .get(INSTANCE_TAG)
            .as_string()
            .map(str::to_string)
    })
}

/// `Symbol.hasInstance` for host constructors whose instances are created from
/// plain objects/arrays rather than by inheriting `.prototype`. The tag is
/// stored on both the constructor and its instances so `x instanceof Ctor`
/// resolves without a real prototype chain.
fn host_has_instance(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let constructor_tag = args.first().and_then(read_instance_tag);
    let value_tag = args.get(1).and_then(read_instance_tag);
    Ok(JSValue::from_bool(
        constructor_tag.is_some() && constructor_tag == value_tag,
    ))
}

fn register_host_constructor(
    global: &mut JSObject,
    name: &str,
    construct: pixi_byte::value::jsvalue::NativeFunctionType,
) {
    let mut prototype = JSObject::new();
    prototype.define_property(
        INSTANCE_TAG.to_string(),
        Property::read_only(JSValue::from_string(name.to_string())),
    );
    let mut constructor = JSObject::new();
    constructor.set(
        "__construct__".to_string(),
        JSValue::from_native_function(construct),
    );
    constructor.define_property(
        "prototype".to_string(),
        Property::read_only(JSValue::from_object(Rc::new(RefCell::new(prototype)))),
    );
    constructor.define_property(
        INSTANCE_TAG.to_string(),
        Property::read_only(JSValue::from_string(name.to_string())),
    );
    constructor.define_property(
        pixi_byte::value::jsobject::HOST_HAS_INSTANCE.to_string(),
        internal_property(JSValue::from_native_function(host_has_instance)),
    );
    global.set(
        name.to_string(),
        JSValue::from_object(Rc::new(RefCell::new(constructor))),
    );
}

pub(crate) fn install_encoding_apis(engine: &mut pixi_byte::JSEngine) {
    let mut global = engine.global_mut().borrow_mut();
    global.set("atob".to_string(), JSValue::from_native_function(atob));
    global.set("btoa".to_string(), JSValue::from_native_function(btoa));
    global.set(
        "encodeURIComponent".to_string(),
        JSValue::from_native_function(encode_uri_component),
    );
    global.set(
        "decodeURIComponent".to_string(),
        JSValue::from_native_function(decode_uri_component),
    );
    global.set(
        "encodeURI".to_string(),
        JSValue::from_native_function(encode_uri),
    );
    global.set(
        "decodeURI".to_string(),
        JSValue::from_native_function(decode_uri),
    );
    register_host_constructor(&mut global, "TextEncoder", text_encoder_constructor);
    register_host_constructor(&mut global, "TextDecoder", text_decoder_constructor);
    register_host_constructor(&mut global, "ArrayBuffer", array_buffer_constructor);
    register_host_constructor(&mut global, "DataView", data_view_constructor);
    install_typed_arrays(&mut global);
    install_array_buffer_members(&mut global);
}

/// `ArrayBuffer.isView` and `ArrayBuffer.prototype.slice`, which libraries use
/// to recognise and copy binary views.
fn install_array_buffer_members(global: &mut JSObject) {
    let Some(constructor) = global.get("ArrayBuffer").as_object() else {
        return;
    };
    constructor.borrow_mut().set(
        "isView".to_string(),
        JSValue::from_native_function(array_buffer_is_view),
    );
    if let Some(prototype) = constructor.borrow().get("prototype").as_object() {
        prototype.borrow_mut().set(
            "slice".to_string(),
            JSValue::from_native_function(array_buffer_slice),
        );
        ARRAY_BUFFER_PROTOTYPE.with(|slot| *slot.borrow_mut() = Some(Rc::clone(&prototype)));
    }
}

fn percent_encode(input: &str, preserve_uri_syntax: bool) -> String {
    let mut output = String::new();
    for byte in input.bytes() {
        let unescaped = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            )
            || (preserve_uri_syntax
                && matches!(
                    byte,
                    b';' | b',' | b'/' | b'?' | b':' | b'@' | b'&' | b'=' | b'+' | b'$' | b'#'
                ));
        if unescaped {
            output.push(byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

fn percent_decode(input: &str) -> JSResult<String> {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(JSError::TypeError("URI malformed".to_string()));
            }
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3])
                .ok()
                .and_then(|value| u8::from_str_radix(value, 16).ok())
                .ok_or_else(|| JSError::TypeError("URI malformed".to_string()))?;
            output.push(hex);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).map_err(|_| JSError::TypeError("URI malformed".to_string()))
}

fn encode_uri_component(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let input = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    Ok(JSValue::from_string(percent_encode(&input, false)))
}

/// Percent-encodes a query-string name or value (host-side helper for form
/// serialization; same encoding as the JS `encodeURIComponent`).
pub(crate) fn encode_query_component(input: &str) -> String {
    percent_encode(input, false)
}

fn decode_uri_component(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let input = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    Ok(JSValue::from_string(percent_decode(&input)?))
}

fn encode_uri(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let input = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    Ok(JSValue::from_string(percent_encode(&input, true)))
}

fn decode_uri(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let input = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    Ok(JSValue::from_string(percent_decode(&input)?))
}

fn value_bytes(value: &JSValue) -> Vec<u8> {
    if let Some(object) = value.as_object() {
        let object = object.borrow();
        let length = (if object.get("length").is_undefined() {
            object.get("byteLength").to_number()
        } else {
            object.get("length").to_number()
        })
        .max(0.0) as usize;
        (0..length)
            .map(|index| object.get(&index.to_string()).to_number() as u8)
            .collect()
    } else if let Some(value) = value.as_string() {
        value.as_bytes().to_vec()
    } else {
        Vec::new()
    }
}

thread_local! {
    /// `ArrayBuffer.prototype` of the most recently installed runtime, so
    /// buffers created by host helpers (fetch bodies, blobs) inherit methods
    /// such as `slice` even though those helpers have no VM handle.
    static ARRAY_BUFFER_PROTOTYPE: RefCell<Option<Rc<RefCell<JSObject>>>> =
        const { RefCell::new(None) };
}

fn make_array_buffer(bytes: Vec<u8>) -> JSValue {
    let mut object = JSObject::new();
    if let Some(prototype) = ARRAY_BUFFER_PROTOTYPE.with(|slot| slot.borrow().clone()) {
        object.set_prototype(Some(prototype));
    }
    object.define_property(
        INSTANCE_TAG.to_string(),
        Property::read_only(JSValue::from_string("ArrayBuffer".to_string())),
    );
    for (index, byte) in bytes.iter().copied().enumerate() {
        object.set(index.to_string(), JSValue::from_number(byte as f64));
    }
    object.define_property(
        "byteLength".to_string(),
        Property::read_only(JSValue::from_number(bytes.len() as f64)),
    );
    JSValue::from_object(Rc::new(RefCell::new(object)))
}

pub(crate) fn make_array_buffer_from_value(value: &JSValue) -> JSValue {
    make_array_buffer(value_bytes(value))
}

fn array_buffer_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    // ToIndex: `new ArrayBuffer()` and `new ArrayBuffer(undefined)` are empty
    // buffers, fractional lengths truncate, and only negative or non-finite
    // lengths are a RangeError.
    let length = match args.get(1) {
        Some(value) => {
            let length = value.to_number();
            if length.is_nan() {
                0
            } else if !length.is_finite() || length < 0.0 {
                return Err(JSError::RangeError(
                    "Invalid ArrayBuffer length".to_string(),
                ));
            } else {
                length.trunc() as usize
            }
        }
        None => 0,
    };
    Ok(make_array_buffer(vec![0; length]))
}

// --- Typed arrays -----------------------------------------------------------
//
// Every typed array shares one `%TypedArray%` constructor object and one
// `%TypedArray%.prototype`, mirroring the spec's internal hierarchy. core-js
// feature-detects exactly that shape (`getPrototypeOf(Int8Array)` must be a
// callable that is not `Function.prototype`) before deciding whether to install
// its own typed-array polyfill; a flat set of unrelated constructors makes it
// install that polyfill over primitives the engine does not support.

#[derive(Clone, Copy)]
enum TypedArrayType {
    Int8,
    Uint8,
    Uint8Clamped,
    Int16,
    Uint16,
    Int32,
    Uint32,
    Float32,
    Float64,
}

impl TypedArrayType {
    fn name(self) -> &'static str {
        match self {
            Self::Int8 => "Int8Array",
            Self::Uint8 => "Uint8Array",
            Self::Uint8Clamped => "Uint8ClampedArray",
            Self::Int16 => "Int16Array",
            Self::Uint16 => "Uint16Array",
            Self::Int32 => "Int32Array",
            Self::Uint32 => "Uint32Array",
            Self::Float32 => "Float32Array",
            Self::Float64 => "Float64Array",
        }
    }

    fn bytes(self) -> usize {
        match self {
            Self::Int8 | Self::Uint8 | Self::Uint8Clamped => 1,
            Self::Int16 | Self::Uint16 => 2,
            Self::Int32 | Self::Uint32 | Self::Float32 => 4,
            Self::Float64 => 8,
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "Int8Array" => Self::Int8,
            "Uint8Array" => Self::Uint8,
            "Uint8ClampedArray" => Self::Uint8Clamped,
            "Int16Array" => Self::Int16,
            "Uint16Array" => Self::Uint16,
            "Int32Array" => Self::Int32,
            "Uint32Array" => Self::Uint32,
            "Float32Array" => Self::Float32,
            "Float64Array" => Self::Float64,
            _ => return None,
        })
    }

    /// ToNumber followed by the element type's conversion, so stored values are
    /// always exactly what the element type can represent.
    fn convert(self, value: f64) -> f64 {
        match self {
            Self::Int8 => to_signed(value, 8) as f64,
            Self::Uint8 => to_unsigned(value, 8) as f64,
            Self::Uint8Clamped => to_uint8_clamped(value),
            Self::Int16 => to_signed(value, 16) as f64,
            Self::Uint16 => to_unsigned(value, 16) as f64,
            Self::Int32 => to_signed(value, 32) as f64,
            Self::Uint32 => to_unsigned(value, 32) as f64,
            Self::Float32 => value as f32 as f64,
            Self::Float64 => value,
        }
    }
}

/// ToUintN: NaN and infinities become 0, other values wrap modulo 2^N.
fn to_unsigned(value: f64, bits: u32) -> u64 {
    if !value.is_finite() {
        return 0;
    }
    let modulo = 2f64.powi(bits as i32);
    let truncated = value.trunc();
    let wrapped = truncated - (truncated / modulo).floor() * modulo;
    wrapped as u64
}

/// ToIntN: ToUintN then reinterpret the top bit as the sign.
fn to_signed(value: f64, bits: u32) -> i64 {
    let unsigned = to_unsigned(value, bits);
    let sign_bit = 1u64 << (bits - 1);
    if unsigned >= sign_bit {
        unsigned as i64 - (1i64 << bits)
    } else {
        unsigned as i64
    }
}

/// ToUint8Clamp: clamp to [0, 255], rounding halves to even.
fn to_uint8_clamped(value: f64) -> f64 {
    if value.is_nan() || value <= 0.0 {
        return 0.0;
    }
    if value >= 255.0 {
        return 255.0;
    }
    let floor = value.floor();
    let fraction = value - floor;
    if fraction < 0.5 {
        floor
    } else if fraction > 0.5 {
        floor + 1.0
    } else if (floor as i64) % 2 == 0 {
        floor
    } else {
        floor + 1.0
    }
}

fn decode_elements(ty: TypedArrayType, bytes: &[u8]) -> Vec<f64> {
    let width = ty.bytes();
    bytes
        .chunks_exact(width)
        .map(|chunk| match ty {
            TypedArrayType::Int8 => chunk[0] as i8 as f64,
            TypedArrayType::Uint8 | TypedArrayType::Uint8Clamped => chunk[0] as f64,
            TypedArrayType::Int16 => i16::from_le_bytes([chunk[0], chunk[1]]) as f64,
            TypedArrayType::Uint16 => u16::from_le_bytes([chunk[0], chunk[1]]) as f64,
            TypedArrayType::Int32 => {
                i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as f64
            }
            TypedArrayType::Uint32 => {
                u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as f64
            }
            TypedArrayType::Float32 => {
                f32::from_bits(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])) as f64
            }
            TypedArrayType::Float64 => f64::from_bits(u64::from_le_bytes([
                chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
            ])),
        })
        .collect()
}

fn encode_elements(ty: TypedArrayType, values: &[f64]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * ty.bytes());
    for value in values {
        match ty {
            TypedArrayType::Int8 => bytes.push(ty.convert(*value) as i8 as u8),
            TypedArrayType::Uint8 | TypedArrayType::Uint8Clamped => {
                bytes.push(ty.convert(*value) as u8)
            }
            TypedArrayType::Int16 => {
                bytes.extend_from_slice(&(ty.convert(*value) as i16).to_le_bytes())
            }
            TypedArrayType::Uint16 => {
                bytes.extend_from_slice(&(ty.convert(*value) as u16).to_le_bytes())
            }
            TypedArrayType::Int32 => {
                bytes.extend_from_slice(&(ty.convert(*value) as i32).to_le_bytes())
            }
            TypedArrayType::Uint32 => {
                bytes.extend_from_slice(&(ty.convert(*value) as u32).to_le_bytes())
            }
            TypedArrayType::Float32 => {
                bytes.extend_from_slice(&(ty.convert(*value) as f32).to_bits().to_le_bytes())
            }
            TypedArrayType::Float64 => {
                bytes.extend_from_slice(&ty.convert(*value).to_bits().to_le_bytes())
            }
        }
    }
    bytes
}

fn typed_array_of(receiver: &JSValue) -> Option<TypedArrayType> {
    read_instance_tag(receiver).and_then(|tag| TypedArrayType::from_name(&tag))
}

fn typed_array_length(receiver: &JSValue) -> usize {
    receiver
        .as_object()
        .map(|object| object.borrow().get("length").to_number().max(0.0) as usize)
        .unwrap_or(0)
}

fn typed_array_element(receiver: &JSValue, index: usize) -> f64 {
    receiver
        .as_object()
        .map(|object| object.borrow().get(&index.to_string()).to_number())
        .unwrap_or(0.0)
}

/// Elements of an array-like (objects with `length`/indices), a string's code
/// units, or nothing for other primitives.
fn array_like_values(value: &JSValue) -> Vec<f64> {
    if let Some(object) = value.as_object() {
        let object = object.borrow();
        let length = object.get("length").to_number().max(0.0) as usize;
        const MAX: usize = 1 << 20;
        let length = length.min(MAX);
        (0..length)
            .map(|index| object.get(&index.to_string()).to_number())
            .collect()
    } else if let Some(text) = value.as_string() {
        text.chars()
            .map(|character| character as u32 as f64)
            .collect()
    } else {
        Vec::new()
    }
}

/// Resolves a `TypedArray` constructor's arguments into element values plus the
/// backing buffer they were read from or copied into.
fn typed_array_source(
    ty: TypedArrayType,
    source: &JSValue,
    offset_arg: Option<&JSValue>,
    length_arg: Option<&JSValue>,
) -> JSResult<(Vec<f64>, JSValue, usize)> {
    if source.is_undefined() || source.is_null() {
        return Ok((Vec::new(), make_array_buffer(Vec::new()), 0));
    }
    if let Some(length) = source.as_number() {
        if length.is_nan() {
            return Ok((Vec::new(), make_array_buffer(Vec::new()), 0));
        }
        if !length.is_finite() || length < 0.0 {
            eprintln!("[debug typed array length] {length}");
            return Err(JSError::RangeError(
                "Invalid typed array length".to_string(),
            ));
        }
        const MAX_TYPED_ARRAY_LENGTH: usize = 1 << 20;
        if length.trunc() as usize > MAX_TYPED_ARRAY_LENGTH {
            return Err(JSError::RangeError(
                "Invalid typed array length".to_string(),
            ));
        }
        let values = vec![0.0; length.trunc() as usize];
        let buffer = make_array_buffer(encode_elements(ty, &values));
        return Ok((values, buffer, 0));
    }
    // An ArrayBuffer argument creates a view over its bytes.
    if read_instance_tag(source).as_deref() == Some("ArrayBuffer") {
        let bytes = value_bytes(source);
        let requested_offset = offset_arg.map(JSValue::to_number).unwrap_or(0.0);
        if !requested_offset.is_finite() || requested_offset < 0.0 {
            return Err(JSError::RangeError(
                "Invalid typed array offset".to_string(),
            ));
        }
        let offset = requested_offset.trunc() as usize;
        let width = ty.bytes();
        if offset > bytes.len() || !offset.is_multiple_of(width) {
            return Err(JSError::RangeError(
                "Invalid typed array offset".to_string(),
            ));
        }
        let available = (bytes.len() - offset) / width;
        let length = length_arg
            .map(JSValue::to_number)
            .filter(|length| length.is_finite())
            .map(|length| (length.max(0.0).trunc() as usize).min(available))
            .unwrap_or(available);
        let values = decode_elements(ty, &bytes[offset..offset + length * width]);
        return Ok((values, source.clone(), offset));
    }
    let values: Vec<f64> = array_like_values(source)
        .into_iter()
        .map(|value| ty.convert(value))
        .collect();
    let buffer = make_array_buffer(encode_elements(ty, &values));
    Ok((values, buffer, 0))
}

fn populate_typed_array(
    object: &Rc<RefCell<JSObject>>,
    ty: TypedArrayType,
    values: &[f64],
    buffer: JSValue,
    byte_offset: usize,
) {
    let mut object = object.borrow_mut();
    for (index, value) in values.iter().enumerate() {
        object.set(index.to_string(), JSValue::from_number(*value));
    }
    object.define_property(
        "length".to_string(),
        Property::read_only(JSValue::from_number(values.len() as f64)),
    );
    object.define_property(
        "byteLength".to_string(),
        Property::read_only(JSValue::from_number((values.len() * ty.bytes()) as f64)),
    );
    object.define_property(
        "byteOffset".to_string(),
        Property::read_only(JSValue::from_number(byte_offset as f64)),
    );
    object.define_property("buffer".to_string(), Property::read_only(buffer));
    object.define_property(
        INSTANCE_TAG.to_string(),
        Property::read_only(JSValue::from_string(ty.name().to_string())),
    );
}

fn typed_array_construct(args: Vec<JSValue>, ty: TypedArrayType) -> JSResult<JSValue> {
    let source = args.get(1).cloned().unwrap_or(JSValue::undefined());
    let (values, buffer, byte_offset) = typed_array_source(ty, &source, args.get(2), args.get(3))?;
    let object = args
        .first()
        .and_then(JSValue::as_object)
        .unwrap_or_else(|| Rc::new(RefCell::new(JSObject::new())));
    populate_typed_array(&object, ty, &values, buffer, byte_offset);
    Ok(JSValue::from_object(object))
}

macro_rules! typed_array_constructor_fn {
    ($name:ident, $ty:expr) => {
        fn $name(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
            typed_array_construct(args, $ty)
        }
    };
}

typed_array_constructor_fn!(int8_array_constructor, TypedArrayType::Int8);
typed_array_constructor_fn!(uint8_array_constructor, TypedArrayType::Uint8);
typed_array_constructor_fn!(
    uint8_clamped_array_constructor,
    TypedArrayType::Uint8Clamped
);
typed_array_constructor_fn!(int16_array_constructor, TypedArrayType::Int16);
typed_array_constructor_fn!(uint16_array_constructor, TypedArrayType::Uint16);
typed_array_constructor_fn!(int32_array_constructor, TypedArrayType::Int32);
typed_array_constructor_fn!(uint32_array_constructor, TypedArrayType::Uint32);
typed_array_constructor_fn!(float32_array_constructor, TypedArrayType::Float32);
typed_array_constructor_fn!(float64_array_constructor, TypedArrayType::Float64);

fn typed_array_illegal_invocation(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    eprintln!(
        "[debug %TypedArray%] stack={:?}",
        vm.frames
            .iter()
            .map(|frame| frame
                .function_name
                .as_ref()
                .map(|name| name.as_str())
                .unwrap_or_default())
            .collect::<Vec<_>>()
    );
    Err(JSError::TypeError(
        "%TypedArray% is not directly constructible".to_string(),
    ))
}

/// Relative index (Start/End) resolution shared by `subarray` and `slice`.
fn relative_index(value: Option<&JSValue>, length: usize, default: usize) -> usize {
    let Some(value) = value else {
        return default;
    };
    if value.is_undefined() {
        return default;
    }
    let index = value.to_number();
    if index.is_nan() {
        return 0;
    }
    let resolved = if index < 0.0 {
        (length as f64 + index.trunc()).max(0.0)
    } else {
        index.trunc()
    };
    resolved.min(length as f64) as usize
}

fn typed_array_set(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(receiver) = args.first() else {
        return Err(JSError::TypeError(
            "set called on a non-typed-array".to_string(),
        ));
    };
    let Some(ty) = typed_array_of(receiver) else {
        return Err(JSError::TypeError(
            "set called on a non-typed-array".to_string(),
        ));
    };
    let values: Vec<f64> = array_like_values(args.get(1).unwrap_or(&JSValue::undefined()))
        .into_iter()
        .map(|value| ty.convert(value))
        .collect();
    let offset = args.get(2).map(JSValue::to_number).unwrap_or(0.0).max(0.0) as usize;
    let length = typed_array_length(receiver);
    if offset + values.len() > length {
        return Err(JSError::RangeError(
            "source is too large for the target typed array".to_string(),
        ));
    }
    if let Some(object) = receiver.as_object() {
        let mut object = object.borrow_mut();
        for (index, value) in values.iter().enumerate() {
            object.set((offset + index).to_string(), JSValue::from_number(*value));
        }
    }
    Ok(JSValue::undefined())
}

fn typed_array_subarray(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let receiver = args.first().cloned().unwrap_or(JSValue::undefined());
    let Some(ty) = typed_array_of(&receiver) else {
        return Err(JSError::TypeError(
            "subarray called on a non-typed-array".to_string(),
        ));
    };
    let length = typed_array_length(&receiver);
    let begin = relative_index(args.get(1), length, 0);
    let end = relative_index(args.get(2), length, length).max(begin);
    let values: Vec<f64> = (begin..end)
        .map(|index| typed_array_element(&receiver, index))
        .collect();
    let buffer = receiver
        .as_object()
        .map(|object| object.borrow().get("buffer"))
        .unwrap_or_else(|| make_array_buffer(Vec::new()));
    let byte_offset = receiver
        .as_object()
        .map(|object| object.borrow().get("byteOffset").to_number().max(0.0) as usize)
        .unwrap_or(0)
        + begin * ty.bytes();
    let prototype = receiver
        .as_object()
        .and_then(|object| object.borrow().get_prototype());
    let object = Rc::new(RefCell::new(JSObject::with_prototype(prototype)));
    populate_typed_array(&object, ty, &values, buffer, byte_offset);
    Ok(JSValue::from_object(object))
}

fn typed_array_slice(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let receiver = args.first().cloned().unwrap_or(JSValue::undefined());
    let Some(ty) = typed_array_of(&receiver) else {
        return Err(JSError::TypeError(
            "slice called on a non-typed-array".to_string(),
        ));
    };
    let length = typed_array_length(&receiver);
    let begin = relative_index(args.get(1), length, 0);
    let end = relative_index(args.get(2), length, length).max(begin);
    let values: Vec<f64> = (begin..end)
        .map(|index| typed_array_element(&receiver, index))
        .collect();
    let buffer = make_array_buffer(encode_elements(ty, &values));
    let prototype = receiver
        .as_object()
        .and_then(|object| object.borrow().get_prototype());
    let object = Rc::new(RefCell::new(JSObject::with_prototype(prototype)));
    populate_typed_array(&object, ty, &values, buffer, 0);
    Ok(JSValue::from_object(object))
}

fn typed_array_join(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let receiver = args.first().cloned().unwrap_or(JSValue::undefined());
    if typed_array_of(&receiver).is_none() {
        return Err(JSError::TypeError(
            "join called on a non-typed-array".to_string(),
        ));
    }
    let separator = args
        .get(1)
        .filter(|value| !value.is_undefined())
        .map(|value| value.to_string())
        .unwrap_or_else(|| ",".to_string());
    let length = typed_array_length(&receiver);
    let parts: Vec<String> = (0..length)
        .map(|index| JSValue::from_number(typed_array_element(&receiver, index)).to_string())
        .collect();
    Ok(JSValue::from_string(parts.join(&separator)))
}

fn typed_array_index_of(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let receiver = args.first().cloned().unwrap_or(JSValue::undefined());
    let Some(ty) = typed_array_of(&receiver) else {
        return Err(JSError::TypeError(
            "indexOf called on a non-typed-array".to_string(),
        ));
    };
    let target = ty.convert(args.get(1).map(JSValue::to_number).unwrap_or(f64::NAN));
    let length = typed_array_length(&receiver);
    let begin = relative_index(args.get(2), length, 0).min(length);
    for index in begin..length {
        if typed_array_element(&receiver, index) == target {
            return Ok(JSValue::from_number(index as f64));
        }
    }
    Ok(JSValue::from_number(-1.0))
}

fn typed_array_fill(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let receiver = args.first().cloned().unwrap_or(JSValue::undefined());
    let Some(ty) = typed_array_of(&receiver) else {
        return Err(JSError::TypeError(
            "fill called on a non-typed-array".to_string(),
        ));
    };
    let value = ty.convert(args.get(1).map(JSValue::to_number).unwrap_or(0.0));
    let length = typed_array_length(&receiver);
    let begin = relative_index(args.get(2), length, 0);
    let end = relative_index(args.get(3), length, length).max(begin);
    if let Some(object) = receiver.as_object() {
        let mut object = object.borrow_mut();
        for index in begin..end {
            object.set(index.to_string(), JSValue::from_number(value));
        }
    }
    Ok(receiver)
}

fn register_typed_array(
    global: &mut JSObject,
    ty: TypedArrayType,
    construct: pixi_byte::value::jsvalue::NativeFunctionType,
    shared_prototype: &Rc<RefCell<JSObject>>,
    shared_constructor: &Rc<RefCell<JSObject>>,
) {
    let mut prototype = JSObject::with_prototype(Some(Rc::clone(shared_prototype)));
    prototype.define_property(
        INSTANCE_TAG.to_string(),
        Property::read_only(JSValue::from_string(ty.name().to_string())),
    );
    prototype.define_property(
        "BYTES_PER_ELEMENT".to_string(),
        Property::read_only(JSValue::from_number(ty.bytes() as f64)),
    );
    let prototype = Rc::new(RefCell::new(prototype));

    let mut constructor = JSObject::with_prototype(Some(Rc::clone(shared_constructor)));
    constructor.set(
        "__construct__".to_string(),
        JSValue::from_native_function(construct),
    );
    constructor.define_property(
        "prototype".to_string(),
        Property::read_only(JSValue::from_object(Rc::clone(&prototype))),
    );
    constructor.define_property(
        "BYTES_PER_ELEMENT".to_string(),
        Property::read_only(JSValue::from_number(ty.bytes() as f64)),
    );
    constructor.define_property(
        INSTANCE_TAG.to_string(),
        Property::read_only(JSValue::from_string(ty.name().to_string())),
    );
    constructor.define_property(
        pixi_byte::value::jsobject::HOST_HAS_INSTANCE.to_string(),
        internal_property(JSValue::from_native_function(host_has_instance)),
    );
    global.set(
        ty.name().to_string(),
        JSValue::from_object(Rc::new(RefCell::new(constructor))),
    );
}

fn install_typed_arrays(global: &mut JSObject) {
    let mut shared_prototype = JSObject::new();
    shared_prototype.set(
        "set".to_string(),
        JSValue::from_native_function(typed_array_set),
    );
    shared_prototype.set(
        "subarray".to_string(),
        JSValue::from_native_function(typed_array_subarray),
    );
    shared_prototype.set(
        "slice".to_string(),
        JSValue::from_native_function(typed_array_slice),
    );
    shared_prototype.set(
        "join".to_string(),
        JSValue::from_native_function(typed_array_join),
    );
    shared_prototype.set(
        "indexOf".to_string(),
        JSValue::from_native_function(typed_array_index_of),
    );
    shared_prototype.set(
        "fill".to_string(),
        JSValue::from_native_function(typed_array_fill),
    );
    let shared_prototype = Rc::new(RefCell::new(shared_prototype));

    let mut shared_constructor = JSObject::new();
    shared_constructor.set(
        "__call__".to_string(),
        JSValue::from_native_function(typed_array_illegal_invocation),
    );
    shared_constructor.define_property(
        "prototype".to_string(),
        Property::read_only(JSValue::from_object(Rc::clone(&shared_prototype))),
    );
    let shared_constructor = Rc::new(RefCell::new(shared_constructor));

    register_typed_array(
        global,
        TypedArrayType::Int8,
        int8_array_constructor,
        &shared_prototype,
        &shared_constructor,
    );
    register_typed_array(
        global,
        TypedArrayType::Uint8,
        uint8_array_constructor,
        &shared_prototype,
        &shared_constructor,
    );
    register_typed_array(
        global,
        TypedArrayType::Uint8Clamped,
        uint8_clamped_array_constructor,
        &shared_prototype,
        &shared_constructor,
    );
    register_typed_array(
        global,
        TypedArrayType::Int16,
        int16_array_constructor,
        &shared_prototype,
        &shared_constructor,
    );
    register_typed_array(
        global,
        TypedArrayType::Uint16,
        uint16_array_constructor,
        &shared_prototype,
        &shared_constructor,
    );
    register_typed_array(
        global,
        TypedArrayType::Int32,
        int32_array_constructor,
        &shared_prototype,
        &shared_constructor,
    );
    register_typed_array(
        global,
        TypedArrayType::Uint32,
        uint32_array_constructor,
        &shared_prototype,
        &shared_constructor,
    );
    register_typed_array(
        global,
        TypedArrayType::Float32,
        float32_array_constructor,
        &shared_prototype,
        &shared_constructor,
    );
    register_typed_array(
        global,
        TypedArrayType::Float64,
        float64_array_constructor,
        &shared_prototype,
        &shared_constructor,
    );
}

fn array_buffer_is_view(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let is_view = args
        .get(1)
        .and_then(read_instance_tag)
        .is_some_and(|tag| tag == "DataView" || TypedArrayType::from_name(&tag).is_some());
    Ok(JSValue::from_bool(is_view))
}

fn array_buffer_slice(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let bytes = value_bytes(args.first().unwrap_or(&JSValue::undefined()));
    let length = bytes.len();
    let begin = relative_index(args.get(1), length, 0);
    let end = relative_index(args.get(2), length, length).max(begin);
    Ok(make_array_buffer(bytes[begin..end].to_vec()))
}

fn btoa(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let input = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    let mut bytes = Vec::with_capacity(input.len());
    for character in input.chars() {
        if character as u32 > u8::MAX as u32 {
            return Err(JSError::TypeError(
                "btoa input contains characters outside Latin-1".to_string(),
            ));
        }
        bytes.push(character as u8);
    }
    Ok(JSValue::from_string(
        base64::engine::general_purpose::STANDARD.encode(bytes),
    ))
}

fn atob(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let input = args
        .get(1)
        .unwrap_or(&JSValue::undefined())
        .to_string()
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect::<String>();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|_| JSError::TypeError("Invalid base64 input".to_string()))?;
    Ok(JSValue::from_string(
        bytes.into_iter().map(char::from).collect(),
    ))
}

fn text_encoder_constructor(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let mut encoder = JSObject::new();
    encoder.define_property(
        INSTANCE_TAG.to_string(),
        Property::read_only(JSValue::from_string("TextEncoder".to_string())),
    );
    encoder.define_property(
        "encoding".to_string(),
        Property::read_only(JSValue::from_string("utf-8".to_string())),
    );
    encoder.set(
        "encode".to_string(),
        JSValue::from_native_function(text_encode),
    );
    Ok(JSValue::from_object(Rc::new(RefCell::new(encoder))))
}

fn text_encode(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let input = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    Ok(vm.array_from_values(
        input
            .into_bytes()
            .into_iter()
            .map(|byte| JSValue::from_number(byte as f64))
            .collect(),
    ))
}

fn text_decoder_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let label = match args.get(1) {
        Some(value) if !matches!(value.kind(), JsValueKind::Undefined | JsValueKind::Null) => {
            value.to_string()
        }
        _ => String::new(),
    };
    if !label.is_empty()
        && !label.eq_ignore_ascii_case("utf-8")
        && !label.eq_ignore_ascii_case("utf8")
    {
        return Err(JSError::TypeError(format!(
            "Only UTF-8 TextDecoder is supported (label={label:?})"
        )));
    }
    let mut decoder = JSObject::new();
    decoder.define_property(
        INSTANCE_TAG.to_string(),
        Property::read_only(JSValue::from_string("TextDecoder".to_string())),
    );
    decoder.define_property(
        "encoding".to_string(),
        Property::read_only(JSValue::from_string("utf-8".to_string())),
    );
    decoder.set(
        "decode".to_string(),
        JSValue::from_native_function(text_decode),
    );
    Ok(JSValue::from_object(Rc::new(RefCell::new(decoder))))
}

fn text_decode(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(value) = args.get(1) else {
        return Ok(JSValue::from_string(String::new()));
    };
    let bytes = value_bytes(value);
    Ok(JSValue::from_string(
        String::from_utf8_lossy(&bytes).into_owned(),
    ))
}

fn data_view_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(buffer_value) = args.get(1) else {
        return Err(JSError::TypeError(
            "DataView: missing ArrayBuffer".to_string(),
        ));
    };
    if buffer_value.is_undefined() || buffer_value.as_object().is_none() {
        return Err(JSError::TypeError(
            "DataView: first argument must be an ArrayBuffer".to_string(),
        ));
    }
    let full_bytes = value_bytes(buffer_value);
    let requested_offset = args.get(2).map(JSValue::to_number).unwrap_or(0.0).max(0.0) as usize;
    let byte_offset = requested_offset.min(full_bytes.len());
    let requested_length = args.get(3).map(JSValue::to_number).unwrap_or(f64::MAX);
    let byte_length = if requested_length.is_finite() {
        (requested_length.max(0.0) as usize).min(full_bytes.len() - byte_offset)
    } else {
        full_bytes.len() - byte_offset
    };
    let mut view = JSObject::new();
    view.define_property(
        INSTANCE_TAG.to_string(),
        Property::read_only(JSValue::from_string("DataView".to_string())),
    );
    view.set("__buf".to_string(), buffer_value.clone());
    view.define_property(
        "buffer".to_string(),
        Property::read_only(buffer_value.clone()),
    );
    view.define_property(
        "byteOffset".to_string(),
        Property::read_only(JSValue::from_number(byte_offset as f64)),
    );
    view.define_property(
        "byteLength".to_string(),
        Property::read_only(JSValue::from_number(byte_length as f64)),
    );
    view.set(
        "getUint8".to_string(),
        JSValue::from_native_function(dataview_get_u8),
    );
    view.set(
        "getUint16".to_string(),
        JSValue::from_native_function(dataview_get_u16),
    );
    view.set(
        "getUint32".to_string(),
        JSValue::from_native_function(dataview_get_u32),
    );
    view.set(
        "getInt8".to_string(),
        JSValue::from_native_function(dataview_get_i8),
    );
    view.set(
        "getInt16".to_string(),
        JSValue::from_native_function(dataview_get_i16),
    );
    view.set(
        "getInt32".to_string(),
        JSValue::from_native_function(dataview_get_i32),
    );
    view.set(
        "getFloat32".to_string(),
        JSValue::from_native_function(dataview_get_f32),
    );
    view.set(
        "getFloat64".to_string(),
        JSValue::from_native_function(dataview_get_f64),
    );
    view.set(
        "setUint8".to_string(),
        JSValue::from_native_function(dataview_set_u8),
    );
    view.set(
        "setUint16".to_string(),
        JSValue::from_native_function(dataview_set_u16),
    );
    view.set(
        "setUint32".to_string(),
        JSValue::from_native_function(dataview_set_u32),
    );
    view.set(
        "setInt8".to_string(),
        JSValue::from_native_function(dataview_set_i8),
    );
    view.set(
        "setInt16".to_string(),
        JSValue::from_native_function(dataview_set_i16),
    );
    view.set(
        "setInt32".to_string(),
        JSValue::from_native_function(dataview_set_i32),
    );
    view.set(
        "setFloat32".to_string(),
        JSValue::from_native_function(dataview_set_f32),
    );
    view.set(
        "setFloat64".to_string(),
        JSValue::from_native_function(dataview_set_f64),
    );
    Ok(JSValue::from_object(Rc::new(RefCell::new(view))))
}

fn dataview_state(view: &JSValue) -> JSResult<(Rc<RefCell<JSObject>>, usize, usize, usize)> {
    let object = view
        .as_object()
        .ok_or_else(|| JSError::TypeError("DataView method called on non-object".to_string()))?;
    let buffer = object.borrow().get("__buf").as_object().ok_or_else(|| {
        JSError::TypeError("DataView method called without backing buffer".to_string())
    })?;
    let byte_offset = object.borrow().get("byteOffset").to_number().max(0.0) as usize;
    let byte_length = object.borrow().get("byteLength").to_number().max(0.0) as usize;
    let full_bytes = value_bytes(&JSValue::from_object(Rc::clone(&buffer)));
    Ok((buffer, byte_offset, byte_length, full_bytes.len()))
}

fn dataview_bounds(
    byte_offset: usize,
    byte_length: usize,
    full_len: usize,
    position: usize,
    width: usize,
) -> JSResult<()> {
    // `position` is view-relative (relative to `byte_offset`), so the only
    // check needed is that the access fits inside the view. `byte_length` is
    // already clamped to `full_len - byte_offset` at construction, which also
    // keeps the absolute index `byte_offset + position` inside the buffer.
    let Some(end) = position.checked_add(width) else {
        return Err(JSError::RangeError(
            "DataView offset is outside the bounds of the view".to_string(),
        ));
    };
    if end > byte_length || byte_offset + end > full_len {
        return Err(JSError::RangeError(
            "DataView offset is outside the bounds of the view".to_string(),
        ));
    }
    Ok(())
}

fn dataview_view_and_bytes(view: &JSValue, args: &[JSValue], width: usize) -> DataViewResult {
    let (buffer, byte_offset, byte_length, full_len) = dataview_state(view)?;
    let position = args.get(1).map(JSValue::to_number).unwrap_or(0.0).max(0.0) as usize;
    dataview_bounds(byte_offset, byte_length, full_len, position, width)?;
    let little_endian = args.get(2).map(JSValue::to_number).unwrap_or(0.0) != 0.0;
    let bytes = value_bytes(&JSValue::from_object(Rc::clone(&buffer)));
    // Convert the view-relative position to an absolute buffer index.
    Ok((
        buffer,
        bytes,
        byte_offset + position,
        little_endian,
        byte_offset,
        byte_length,
    ))
}

fn dataview_get_u8(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(view) = args.first() else {
        return Ok(JSValue::undefined());
    };
    let (_, bytes, position, _, _, _) = dataview_view_and_bytes(view, &args, 1)?;
    Ok(JSValue::from_number(bytes[position] as f64))
}

fn dataview_get_u16(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(view) = args.first() else {
        return Ok(JSValue::undefined());
    };
    let (_, bytes, position, little_endian, _, _) = dataview_view_and_bytes(view, &args, 2)?;
    let value = read_u16(&bytes, position, little_endian);
    Ok(JSValue::from_number(value as f64))
}

fn dataview_get_u32(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(view) = args.first() else {
        return Ok(JSValue::undefined());
    };
    let (_, bytes, position, little_endian, _, _) = dataview_view_and_bytes(view, &args, 4)?;
    let value = read_u32(&bytes, position, little_endian);
    Ok(JSValue::from_number(value as f64))
}

fn dataview_get_i8(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(view) = args.first() else {
        return Ok(JSValue::undefined());
    };
    let (_, bytes, position, _, _, _) = dataview_view_and_bytes(view, &args, 1)?;
    Ok(JSValue::from_number(bytes[position] as i8 as f64))
}

fn dataview_get_i16(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(view) = args.first() else {
        return Ok(JSValue::undefined());
    };
    let (_, bytes, position, little_endian, _, _) = dataview_view_and_bytes(view, &args, 2)?;
    let value = read_u16(&bytes, position, little_endian) as i16;
    Ok(JSValue::from_number(value as f64))
}

fn dataview_get_i32(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(view) = args.first() else {
        return Ok(JSValue::undefined());
    };
    let (_, bytes, position, little_endian, _, _) = dataview_view_and_bytes(view, &args, 4)?;
    let value = read_u32(&bytes, position, little_endian) as i32;
    Ok(JSValue::from_number(value as f64))
}

fn dataview_get_f32(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(view) = args.first() else {
        return Ok(JSValue::undefined());
    };
    let (_, bytes, position, little_endian, _, _) = dataview_view_and_bytes(view, &args, 4)?;
    let word = read_u32(&bytes, position, little_endian);
    Ok(JSValue::from_number(f32::from_bits(word) as f64))
}

fn dataview_get_f64(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(view) = args.first() else {
        return Ok(JSValue::undefined());
    };
    let (_, bytes, position, little_endian, _, _) = dataview_view_and_bytes(view, &args, 8)?;
    let value = read_u64(&bytes, position, little_endian);
    Ok(JSValue::from_number(f64::from_bits(value)))
}

fn dataview_set_u8(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(view) = args.first() else {
        return Ok(JSValue::undefined());
    };
    let (buffer, _, position, _, _, _) = dataview_write_state(view, &args, 1)?;
    let value = args.get(2).map(JSValue::to_number).unwrap_or(0.0) as u8;
    buffer
        .borrow_mut()
        .set(position.to_string(), JSValue::from_number(value as f64));
    Ok(JSValue::undefined())
}

fn dataview_set_u16(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(view) = args.first() else {
        return Ok(JSValue::undefined());
    };
    let (buffer, _, position, little_endian, _, _) = dataview_write_state(view, &args, 2)?;
    let value = args.get(2).map(JSValue::to_number).unwrap_or(0.0) as u16;
    write_u16(&buffer, position, little_endian, value);
    Ok(JSValue::undefined())
}

fn dataview_set_u32(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(view) = args.first() else {
        return Ok(JSValue::undefined());
    };
    let (buffer, _, position, little_endian, _, _) = dataview_write_state(view, &args, 4)?;
    let value = args.get(2).map(JSValue::to_number).unwrap_or(0.0) as u32;
    write_u32(&buffer, position, little_endian, value);
    Ok(JSValue::undefined())
}

fn dataview_set_i8(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(view) = args.first() else {
        return Ok(JSValue::undefined());
    };
    let (buffer, _, position, _, _, _) = dataview_write_state(view, &args, 1)?;
    // Write as unsigned bits so the stored f64 round-trips through
    // `value_bytes` (which truncates each slot with `as u8`).
    let value = args.get(2).map(JSValue::to_number).unwrap_or(0.0) as i8 as u8;
    buffer
        .borrow_mut()
        .set(position.to_string(), JSValue::from_number(value as f64));
    Ok(JSValue::undefined())
}

fn dataview_set_i16(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(view) = args.first() else {
        return Ok(JSValue::undefined());
    };
    let (buffer, _, position, little_endian, _, _) = dataview_write_state(view, &args, 2)?;
    let value = args.get(2).map(JSValue::to_number).unwrap_or(0.0) as i16;
    write_u16(&buffer, position, little_endian, value as u16);
    Ok(JSValue::undefined())
}

fn dataview_set_i32(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(view) = args.first() else {
        return Ok(JSValue::undefined());
    };
    let (buffer, _, position, little_endian, _, _) = dataview_write_state(view, &args, 4)?;
    let value = args.get(2).map(JSValue::to_number).unwrap_or(0.0) as i32;
    write_u32(&buffer, position, little_endian, value as u32);
    Ok(JSValue::undefined())
}

fn dataview_set_f32(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(view) = args.first() else {
        return Ok(JSValue::undefined());
    };
    let (buffer, _, position, little_endian, _, _) = dataview_write_state(view, &args, 4)?;
    let value = args.get(2).map(JSValue::to_number).unwrap_or(0.0) as f32;
    write_u32(&buffer, position, little_endian, value.to_bits());
    Ok(JSValue::undefined())
}

fn dataview_set_f64(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(view) = args.first() else {
        return Ok(JSValue::undefined());
    };
    let (buffer, _, position, little_endian, _, _) = dataview_write_state(view, &args, 8)?;
    let value = args.get(2).map(JSValue::to_number).unwrap_or(0.0);
    write_u64(&buffer, position, little_endian, value.to_bits());
    Ok(JSValue::undefined())
}

fn dataview_write_state(view: &JSValue, args: &[JSValue], width: usize) -> DataViewResult {
    let (buffer, byte_offset, byte_length, full_len) = dataview_state(view)?;
    let position = args.get(1).map(JSValue::to_number).unwrap_or(0.0).max(0.0) as usize;
    dataview_bounds(byte_offset, byte_length, full_len, position, width)?;
    let little_endian = args.get(3).map(JSValue::to_number).unwrap_or(0.0) != 0.0;
    let bytes = value_bytes(&JSValue::from_object(Rc::clone(&buffer)));
    // Convert the view-relative position to an absolute buffer index.
    Ok((
        buffer,
        bytes,
        byte_offset + position,
        little_endian,
        byte_offset,
        byte_length,
    ))
}

fn read_u16(bytes: &[u8], position: usize, little_endian: bool) -> u16 {
    let pair = [bytes[position], bytes[position + 1]];
    if little_endian {
        u16::from_le_bytes(pair)
    } else {
        u16::from_be_bytes(pair)
    }
}

fn read_u32(bytes: &[u8], position: usize, little_endian: bool) -> u32 {
    let quad = bytes[position..position + 4].try_into().unwrap();
    if little_endian {
        u32::from_le_bytes(quad)
    } else {
        u32::from_be_bytes(quad)
    }
}

fn read_u64(bytes: &[u8], position: usize, little_endian: bool) -> u64 {
    let octet = bytes[position..position + 8].try_into().unwrap();
    if little_endian {
        u64::from_le_bytes(octet)
    } else {
        u64::from_be_bytes(octet)
    }
}

fn write_u16(buffer: &Rc<RefCell<JSObject>>, position: usize, little_endian: bool, value: u16) {
    let bytes = if little_endian {
        value.to_le_bytes()
    } else {
        value.to_be_bytes()
    };
    let mut object = buffer.borrow_mut();
    for (index, byte) in bytes.iter().copied().enumerate() {
        object.set(
            (position + index).to_string(),
            JSValue::from_number(byte as f64),
        );
    }
}

fn write_u32(buffer: &Rc<RefCell<JSObject>>, position: usize, little_endian: bool, value: u32) {
    let bytes = if little_endian {
        value.to_le_bytes()
    } else {
        value.to_be_bytes()
    };
    let mut object = buffer.borrow_mut();
    for (index, byte) in bytes.iter().copied().enumerate() {
        object.set(
            (position + index).to_string(),
            JSValue::from_number(byte as f64),
        );
    }
}

fn write_u64(buffer: &Rc<RefCell<JSObject>>, position: usize, little_endian: bool, value: u64) {
    let bytes = if little_endian {
        value.to_le_bytes()
    } else {
        value.to_be_bytes()
    };
    let mut object = buffer.borrow_mut();
    for (index, byte) in bytes.iter().copied().enumerate() {
        object.set(
            (position + index).to_string(),
            JSValue::from_number(byte as f64),
        );
    }
}
