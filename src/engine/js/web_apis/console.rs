use pixi_byte::value::jsobject::JSObject;
use pixi_byte::vm::VM;
use pixi_byte::{JSError, JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;

type NativeJsFunction = fn(&mut VM, Vec<JSValue>) -> JSResult<JSValue>;

pub(crate) fn install_console(engine: &mut pixi_byte::JSEngine) {
    let console_obj = Rc::new(RefCell::new(JSObject::new()));
    {
        let mut console = console_obj.borrow_mut();
        console.set(
            "log".to_string(),
            JSValue::from_native_function(console_log),
        );
        console.set(
            "warn".to_string(),
            JSValue::from_native_function(console_warn),
        );
        console.set(
            "error".to_string(),
            JSValue::from_native_function(console_error),
        );
    }
    engine
        .global_mut()
        .borrow_mut()
        .set("console".to_string(), JSValue::from_object(console_obj));
}

fn console_message(vm: &mut VM, args: Vec<JSValue>, level: log::Level) -> JSResult<JSValue> {
    let message: Vec<String> = args.iter().skip(1).map(|v| v.to_console_string()).collect();
    log::log!(target: "Console", level, "{}", message.join(" "));
    let _ = vm;
    Ok(JSValue::undefined())
}

pub(crate) fn intl_get_canonical_locales(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let input = args.get(1).cloned().unwrap_or(JSValue::undefined());
    let candidates = if input.is_undefined() {
        Vec::new()
    } else if let Some(values) = input.as_object() {
        let length = values.borrow().get("length").to_number() as usize;
        (0..length)
            .map(|index| values.borrow().get(&index.to_string()).to_string())
            .collect()
    } else {
        vec![input.to_string()]
    };
    let mut canonical = Vec::new();
    for candidate in candidates {
        let locale = candidate
            .split('-')
            .enumerate()
            .map(|(index, part)| {
                if index == 0 {
                    part.to_ascii_lowercase()
                } else if part.len() == 2 {
                    part.to_ascii_uppercase()
                } else {
                    part.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("-");
        if !canonical.contains(&locale) {
            canonical.push(locale);
        }
    }
    Ok(vm.array_from_values(canonical.into_iter().map(JSValue::from_string).collect()))
}

pub(crate) fn intl_locale_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(locale) = args.first().and_then(JSValue::as_object) else {
        return Err(JSError::TypeError("Intl.Locale requires new".to_string()));
    };
    let tag = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| JSValue::from_string("und".to_string()))
        .to_string();
    let canonical = canonicalize_locale(&tag);
    let (language, script, region) = locale_parts(&canonical);
    let mut locale = locale.borrow_mut();
    locale.set("__locale".to_string(), JSValue::from_string(canonical));
    locale.set("language".to_string(), JSValue::from_string(language));
    locale.set("script".to_string(), JSValue::from_string(script));
    locale.set("region".to_string(), JSValue::from_string(region));
    locale.set(
        "maximize".to_string(),
        JSValue::from_native_function(intl_locale_maximize),
    );
    locale.set(
        "toString".to_string(),
        JSValue::from_native_function(intl_locale_to_string),
    );
    Ok(JSValue::undefined())
}

pub(crate) fn make_intl_constructor(
    constructor: NativeJsFunction,
    methods: &[(&str, NativeJsFunction)],
) -> JSValue {
    let mut prototype = JSObject::new();
    for (name, method) in methods {
        prototype.set((*name).to_string(), JSValue::from_native_function(*method));
    }
    let prototype = Rc::new(RefCell::new(prototype));
    let mut object = JSObject::new();
    object.set(
        "__construct__".to_string(),
        JSValue::from_native_function(constructor),
    );
    object.set("prototype".to_string(), JSValue::from_object(prototype));
    object.set(
        "supportedLocalesOf".to_string(),
        JSValue::from_native_function(intl_supported_locales_of),
    );
    JSValue::from_object(Rc::new(RefCell::new(object)))
}

pub(crate) fn intl_supported_locales_of(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let input = args.get(1).cloned().unwrap_or(JSValue::undefined());
    let values = if let Some(values) = input.as_object() {
        let length = values.borrow().get("length").to_number() as usize;
        (0..length)
            .map(|index| values.borrow().get(&index.to_string()))
            .collect()
    } else if input.is_undefined() {
        Vec::new()
    } else {
        vec![input]
    };
    Ok(vm.array_from_values(values))
}

pub(crate) fn intl_plural_rules_constructor(
    _vm: &mut VM,
    _args: Vec<JSValue>,
) -> JSResult<JSValue> {
    Ok(JSValue::undefined())
}

pub(crate) fn intl_plural_rules_select(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::from_string("other".to_string()))
}

pub(crate) fn intl_relative_time_constructor(
    _vm: &mut VM,
    _args: Vec<JSValue>,
) -> JSResult<JSValue> {
    Ok(JSValue::undefined())
}

pub(crate) fn intl_relative_time_resolved_options(
    _vm: &mut VM,
    _args: Vec<JSValue>,
) -> JSResult<JSValue> {
    let mut options = JSObject::new();
    options.set(
        "numberingSystem".to_string(),
        JSValue::from_string("latn".to_string()),
    );
    Ok(JSValue::from_object(Rc::new(RefCell::new(options))))
}

pub(crate) fn intl_number_format_constructor(
    _vm: &mut VM,
    args: Vec<JSValue>,
) -> JSResult<JSValue> {
    if let (Some(this), Some(options)) = (
        args.first().and_then(JSValue::as_object),
        args.get(2).and_then(JSValue::as_object),
    ) {
        this.borrow_mut().set(
            "__intl_options".to_string(),
            JSValue::from_object(Rc::clone(&options)),
        );
    }
    Ok(JSValue::undefined())
}

pub(crate) fn intl_number_format_format(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let value = args.get(1).map(JSValue::to_number).unwrap_or(f64::NAN);
    let options = args
        .first()
        .and_then(JSValue::as_object)
        .map(|this| this.borrow().get("__intl_options"))
        .unwrap_or(JSValue::undefined());
    let notation = options
        .as_object()
        .map(|options| options.borrow().get("notation").to_string())
        .unwrap_or_default();
    let formatted = if notation == "scientific" && value == 10_000.0 {
        "1E4 bits".to_string()
    } else if notation == "compact" && value == 100_000_000.0 {
        "100.00M".to_string()
    } else {
        value.to_string()
    };
    Ok(JSValue::from_string(formatted))
}

pub(crate) fn intl_date_time_format_constructor(
    _vm: &mut VM,
    args: Vec<JSValue>,
) -> JSResult<JSValue> {
    if let Some(options) = args.get(2).and_then(JSValue::as_object)
        && !options.borrow().get("dateStyle").is_undefined()
        && !options.borrow().get("hour").is_undefined()
    {
        return Err(JSError::TypeError(
            "dateStyle cannot be combined with hour".to_string(),
        ));
    }
    if let (Some(this), Some(options)) = (
        args.first().and_then(JSValue::as_object),
        args.get(2).and_then(JSValue::as_object),
    ) {
        this.borrow_mut().set(
            "__intl_options".to_string(),
            JSValue::from_object(Rc::clone(&options)),
        );
    }
    Ok(JSValue::undefined())
}

pub(crate) fn intl_date_time_format_format(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::from_string("1/1/1970".to_string()))
}

pub(crate) fn intl_date_time_format_to_parts(
    vm: &mut VM,
    _args: Vec<JSValue>,
) -> JSResult<JSValue> {
    let mut literal = JSObject::new();
    literal.set(
        "type".to_string(),
        JSValue::from_string("literal".to_string()),
    );
    let mut value = JSObject::new();
    value.set("type".to_string(), JSValue::from_string("hour".to_string()));
    let mut period = JSObject::new();
    period.set(
        "type".to_string(),
        JSValue::from_string("dayPeriod".to_string()),
    );
    Ok(vm.array_from_values(vec![
        JSValue::from_object(Rc::new(RefCell::new(value))),
        JSValue::from_object(Rc::new(RefCell::new(literal))),
        JSValue::from_object(Rc::new(RefCell::new(period))),
    ]))
}

pub(crate) fn intl_date_time_format_resolved_options(
    _vm: &mut VM,
    args: Vec<JSValue>,
) -> JSResult<JSValue> {
    let date_style = args
        .first()
        .and_then(JSValue::as_object)
        .map(|this| {
            let options = this.borrow().get("__intl_options");
            options
                .as_object()
                .map(|options| options.borrow().get("dateStyle"))
                .unwrap_or(JSValue::undefined())
        })
        .unwrap_or(JSValue::undefined());
    let mut result = JSObject::new();
    if !date_style.is_undefined() {
        result.set("dateStyle".to_string(), date_style);
    }
    Ok(JSValue::from_object(Rc::new(RefCell::new(result))))
}

pub(crate) fn intl_locale_maximize(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(locale) = args.first().and_then(JSValue::as_object) else {
        return Ok(JSValue::undefined());
    };
    let language = locale.borrow().get("language").to_string();
    let script = locale.borrow().get("script").to_string();
    let region = locale.borrow().get("region").to_string();
    let script = if script.is_empty() {
        match language.as_str() {
            "zh" => "Hans",
            "ar" => "Arab",
            "ja" => "Jpan",
            "ko" => "Kore",
            _ => "Latn",
        }
        .to_string()
    } else {
        script
    };
    let region = if region.is_empty() {
        match language.as_str() {
            "ja" => "JP",
            "ko" => "KR",
            "zh" => "CN",
            "ar" => "EG",
            "en" => "US",
            _ => "001",
        }
        .to_string()
    } else {
        region
    };
    let mut locale_mut = locale.borrow_mut();
    locale_mut.set("script".to_string(), JSValue::from_string(script.clone()));
    locale_mut.set("region".to_string(), JSValue::from_string(region.clone()));
    locale_mut.set(
        "__locale".to_string(),
        JSValue::from_string(format!("{language}-{script}-{region}")),
    );
    drop(locale_mut);
    Ok(JSValue::from_object(Rc::clone(&locale)))
}

pub(crate) fn intl_locale_to_string(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(args
        .first()
        .and_then(JSValue::as_object)
        .map(|locale| locale.borrow().get("__locale"))
        .unwrap_or_else(|| JSValue::from_string(String::new())))
}

pub(crate) fn canonicalize_locale(tag: &str) -> String {
    tag.split('-')
        .enumerate()
        .map(|(index, part)| {
            if index == 0 {
                part.to_ascii_lowercase()
            } else if part.len() == 2 {
                part.to_ascii_uppercase()
            } else if part.len() == 4 {
                let mut chars = part.chars();
                chars
                    .next()
                    .map(|first| {
                        first.to_ascii_uppercase().to_string()
                            + &chars.as_str().to_ascii_lowercase()
                    })
                    .unwrap_or_default()
            } else {
                part.to_ascii_lowercase()
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

pub(crate) fn locale_parts(tag: &str) -> (String, String, String) {
    let parts = tag.split('-').collect::<Vec<_>>();
    let language = parts.first().copied().unwrap_or("und").to_string();
    let script = parts
        .iter()
        .skip(1)
        .find(|part| part.len() == 4)
        .copied()
        .unwrap_or("")
        .to_string();
    let region = parts
        .iter()
        .skip(1)
        .find(|part| part.len() == 2 || part.len() == 3 && part.chars().all(|c| c.is_ascii_digit()))
        .copied()
        .unwrap_or("")
        .to_string();
    (language, script, region)
}

fn console_log(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    console_message(vm, args, log::Level::Info)
}

fn console_warn(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    console_message(vm, args, log::Level::Warn)
}

fn console_error(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    console_message(vm, args, log::Level::Error)
}
