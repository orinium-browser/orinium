use base64::Engine as _;
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
            _ => value_bytes(value),
        },
        None => Vec::new(),
    };
    let array = vm.array_from_values(
        bytes
            .iter()
            .copied()
            .map(|byte| JSValue::from_number(byte as f64))
            .collect(),
    );
    if let Some(object) = array.as_object() {
        object.borrow_mut().define_property(
            "byteLength".to_string(),
            Property::read_only(JSValue::from_number(bytes.len() as f64)),
        );
        object.borrow_mut().define_property(
            "buffer".to_string(),
            Property::read_only(make_array_buffer(bytes)),
        );
    }
    Ok(array)
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
    let label = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    if !label.is_empty()
        && !label.eq_ignore_ascii_case("utf-8")
        && !label.eq_ignore_ascii_case("utf8")
    {
        return Err(JSError::TypeError(
            "Only UTF-8 TextDecoder is supported".to_string(),
        ));
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
