use base64::Engine as _;
use pixi_byte::value::jsvalue::JsValueKind;
use pixi_byte::value::jsobject::{JSObject, Property};
use pixi_byte::vm::VM;
use pixi_byte::{JSError, JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) fn install_encoding_apis(engine: &mut pixi_byte::JSEngine) {
    let mut encoder_constructor = JSObject::new();
    encoder_constructor.set(
        "__construct__".to_string(),
        JSValue::from_native_function(text_encoder_constructor),
    );
    let mut decoder_constructor = JSObject::new();
    decoder_constructor.set(
        "__construct__".to_string(),
        JSValue::from_native_function(text_decoder_constructor),
    );
    let mut array_buffer_constructor_object = JSObject::new();
    array_buffer_constructor_object.set(
        "__construct__".to_string(),
        JSValue::from_native_function(array_buffer_constructor),
    );
    let mut uint8_array_constructor_object = JSObject::new();
    uint8_array_constructor_object.set(
        "__construct__".to_string(),
        JSValue::from_native_function(uint8_array_constructor),
    );
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
    global.set(
        "TextEncoder".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(encoder_constructor))),
    );
    global.set(
        "TextDecoder".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(decoder_constructor))),
    );
    global.set(
        "ArrayBuffer".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(array_buffer_constructor_object))),
    );
    global.set(
        "Uint8Array".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(uint8_array_constructor_object))),
    );
    global.set(
        "DataView".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(data_view_constructor_object()))),
    );
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

fn make_array_buffer(bytes: Vec<u8>) -> JSValue {
    let mut object = JSObject::new();
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
    let length = args.get(1).map(JSValue::to_number).unwrap_or(0.0);
    let length = if length.is_finite() && length >= 0.0 {
        length as usize
    } else {
        return Err(JSError::TypeError("Invalid ArrayBuffer length".to_string()));
    };
    Ok(make_array_buffer(vec![0; length]))
}

fn uint8_array_constructor(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let bytes = match args.get(1) {
        Some(value) => match value.as_number() {
            Some(length) if length.is_finite() && length >= 0.0 => {
                vec![0; length as usize]
            }
            _ => {
                let buffer = value.as_object();
                let has_offset = args
                    .get(3)
                    .map(|item| !matches!(item.kind(), JsValueKind::Undefined))
                    .unwrap_or(false);
                if has_offset
                    && buffer.is_some_and(|object| !object.borrow().get("byteLength").is_undefined())
                {
                    let offset = args.get(2).map(JSValue::to_number).unwrap_or(0.0).max(0.0) as usize;
                    let length = args.get(3).map(JSValue::to_number).unwrap_or(0.0).max(0.0) as usize;
                    let base = value_bytes(value);
                    let end = (offset + length).min(base.len());
                    let slice = base[offset.min(base.len())..end].to_vec();
                    let array = vm.array_from_values(
                        slice.iter().copied().map(|byte| JSValue::from_number(byte as f64)).collect(),
                    );
                    install_uint8_descriptors(&array, offset, slice.len(), value.clone(), vm);
                    return Ok(array);
                } else {
                    value_bytes(value)
                }
            }
        },
        None => Vec::new(),
    };
    Ok(make_uint8_array(vm, bytes, None))
}

fn make_uint8_array(vm: &mut VM, bytes: Vec<u8>, buffer: Option<JSValue>) -> JSValue {
    let array = vm.array_from_values(
        bytes
            .iter()
            .copied()
            .map(|byte| JSValue::from_number(byte as f64))
            .collect(),
    );
    install_uint8_descriptors(&array, 0, bytes.len(), buffer.unwrap_or_else(|| make_array_buffer(bytes.clone())), vm);
    array
}

fn install_uint8_descriptors(array: &JSValue, offset: usize, length: usize, buffer: JSValue, _vm: &mut VM) {
    if let Some(object) = array.as_object() {
        object.borrow_mut().define_property(
            "byteLength".to_string(),
            Property::read_only(JSValue::from_number(length as f64)),
        );
        object.borrow_mut().define_property(
            "byteOffset".to_string(),
            Property::read_only(JSValue::from_number(offset as f64)),
        );
        object.borrow_mut().define_property(
            "buffer".to_string(),
            Property::read_only(buffer),
        );
        object.borrow_mut().set(
            "set".to_string(),
            JSValue::from_native_function(uint8_array_set),
        );
    }
}

fn uint8_array_set(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(receiver) = args.first() else {
        return Ok(JSValue::undefined());
    };
    let Some(target) = args.get(1) else {
        return Ok(JSValue::undefined());
    };
    let output = value_bytes(receiver);
    let input = value_bytes(target);
    let offset = args.get(2).map(JSValue::to_number).unwrap_or(0.0).max(0.0) as usize;
    if let Some(object) = receiver.as_object() {
        let mut object = object.borrow_mut();
        for (index, byte) in input.iter().copied().enumerate() {
            let position = offset + index;
            if position < output.len() {
                object.set(position.to_string(), JSValue::from_number(byte as f64));
            }
        }
    }
    Ok(JSValue::undefined())
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

fn data_view_constructor_object() -> JSObject {
    let mut constructor = JSObject::new();
    constructor.set(
        "__construct__".to_string(),
        JSValue::from_native_function(data_view_constructor),
    );
    constructor
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
    view.set("getUint8".to_string(), JSValue::from_native_function(dataview_get_u8));
    view.set("getUint16".to_string(), JSValue::from_native_function(dataview_get_u16));
    view.set("getUint32".to_string(), JSValue::from_native_function(dataview_get_u32));
    view.set("getInt8".to_string(), JSValue::from_native_function(dataview_get_i8));
    view.set("getInt16".to_string(), JSValue::from_native_function(dataview_get_i16));
    view.set("getInt32".to_string(), JSValue::from_native_function(dataview_get_i32));
    view.set("getFloat32".to_string(), JSValue::from_native_function(dataview_get_f32));
    view.set("getFloat64".to_string(), JSValue::from_native_function(dataview_get_f64));
    view.set("setUint8".to_string(), JSValue::from_native_function(dataview_set_u8));
    view.set("setUint16".to_string(), JSValue::from_native_function(dataview_set_u16));
    view.set("setUint32".to_string(), JSValue::from_native_function(dataview_set_u32));
    view.set("setInt8".to_string(), JSValue::from_native_function(dataview_set_i8));
    view.set("setInt16".to_string(), JSValue::from_native_function(dataview_set_i16));
    view.set("setInt32".to_string(), JSValue::from_native_function(dataview_set_i32));
    view.set("setFloat32".to_string(), JSValue::from_native_function(dataview_set_f32));
    view.set("setFloat64".to_string(), JSValue::from_native_function(dataview_set_f64));
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
    let full_bytes = value_bytes(&JSValue::from_object(buffer.clone()));
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

fn dataview_view_and_bytes(
    view: &JSValue,
    args: &[JSValue],
    width: usize,
) -> JSResult<(Rc<RefCell<JSObject>>, Vec<u8>, usize, bool, usize, usize)> {
    let (buffer, byte_offset, byte_length, full_len) = dataview_state(view)?;
    let position = args.get(1).map(JSValue::to_number).unwrap_or(0.0).max(0.0) as usize;
    dataview_bounds(byte_offset, byte_length, full_len, position, width)?;
    let little_endian = args.get(2).map(JSValue::to_number).unwrap_or(0.0) != 0.0;
    let bytes = value_bytes(&JSValue::from_object(buffer.clone()));
    // Convert the view-relative position to an absolute buffer index.
    Ok((buffer, bytes, byte_offset + position, little_endian, byte_offset, byte_length))
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

fn dataview_write_state(
    view: &JSValue,
    args: &[JSValue],
    width: usize,
) -> JSResult<(Rc<RefCell<JSObject>>, Vec<u8>, usize, bool, usize, usize)> {
    let (buffer, byte_offset, byte_length, full_len) = dataview_state(view)?;
    let position = args.get(1).map(JSValue::to_number).unwrap_or(0.0).max(0.0) as usize;
    dataview_bounds(byte_offset, byte_length, full_len, position, width)?;
    let little_endian = args.get(3).map(JSValue::to_number).unwrap_or(0.0) != 0.0;
    let bytes = value_bytes(&JSValue::from_object(buffer.clone()));
    // Convert the view-relative position to an absolute buffer index.
    Ok((buffer, bytes, byte_offset + position, little_endian, byte_offset, byte_length))
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
        object.set((position + index).to_string(), JSValue::from_number(byte as f64));
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
        object.set((position + index).to_string(), JSValue::from_number(byte as f64));
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
        object.set((position + index).to_string(), JSValue::from_number(byte as f64));
    }
}
