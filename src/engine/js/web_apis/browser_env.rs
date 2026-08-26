use crate::engine::js::common::{
    dom_node, host_read_only_property, is_callable, noop, with_host, with_host_mut,
};
use crate::engine::js::web_apis::console::{
    intl_date_time_format_constructor, intl_date_time_format_format,
    intl_date_time_format_resolved_options, intl_date_time_format_to_parts,
    intl_get_canonical_locales, intl_locale_constructor, intl_number_format_constructor,
    intl_number_format_format, intl_plural_rules_constructor, intl_plural_rules_select,
    intl_relative_time_constructor, intl_relative_time_resolved_options, make_intl_constructor,
};
use crate::engine::js::web_apis::dom::document::{
    add_document_event_listener, remove_document_event_listener,
};
use crate::engine::js::web_apis::dom::element::{get_style, read_only_accessor_property};
use crate::engine::js::web_apis::dom::events::make_event_constructor;
use crate::engine::js::web_apis::storage::make_storage;
use crate::engine::js::web_apis::timers::clear_timer;
use pixi_byte::value::JSArray;
use pixi_byte::value::jsobject::{JSObject, Property};
use pixi_byte::vm::VM;
use pixi_byte::{JSError, JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::engine::js::JsTimer;

/// TODO: Replace the temporary hard-coded browser environment with values
/// provided by the actual browser and platform state.
pub(crate) fn install_browser_environment(engine: &mut pixi_byte::JSEngine) {
    let mut navigator = JSObject::new();
    navigator.define_property(
        "userAgent".to_string(),
        Property::read_only(JSValue::String(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Orinium/0.1".to_string(),
        )),
    );
    navigator.define_property(
        "language".to_string(),
        host_read_only_property(JSValue::String("en-US".to_string())),
    );
    navigator.define_property(
        "languages".to_string(),
        host_read_only_property(
            JSArray::from_vec(vec![JSValue::String("en-US".to_string())]).to_object(),
        ),
    );
    navigator.define_property(
        "platform".to_string(),
        Property::read_only(JSValue::String("Win32".to_string())),
    );
    navigator.define_property(
        "cookieEnabled".to_string(),
        Property::read_only(JSValue::Boolean(true)),
    );
    navigator.define_property(
        "onLine".to_string(),
        Property::read_only(JSValue::Boolean(true)),
    );

    let mut location = JSObject::new();
    location.define_property(
        "href".to_string(),
        read_only_accessor_property(location_href),
    );
    location.define_property(
        "origin".to_string(),
        read_only_accessor_property(location_origin),
    );
    location.define_property(
        "protocol".to_string(),
        read_only_accessor_property(location_protocol),
    );
    location.define_property(
        "host".to_string(),
        read_only_accessor_property(location_host),
    );
    location.define_property(
        "hostname".to_string(),
        read_only_accessor_property(location_hostname),
    );
    location.define_property(
        "port".to_string(),
        read_only_accessor_property(location_port),
    );
    location.define_property(
        "pathname".to_string(),
        read_only_accessor_property(location_pathname),
    );
    location.define_property(
        "search".to_string(),
        read_only_accessor_property(location_search),
    );
    location.define_property(
        "hash".to_string(),
        read_only_accessor_property(location_hash),
    );
    // TODO: Implement Location navigation methods and connect them to WebView navigation.
    location.set("assign".to_string(), JSValue::NativeFunction(noop));
    location.set("replace".to_string(), JSValue::NativeFunction(noop));
    location.set("reload".to_string(), JSValue::NativeFunction(noop));

    let mut history = JSObject::new();
    history.define_property(
        "length".to_string(),
        Property::read_only(JSValue::Number(1.0)),
    );
    history.define_property("state".to_string(), Property::read_only(JSValue::Null));
    // TODO: Implement session history traversal and pushState/replaceState state updates.
    history.set("back".to_string(), JSValue::NativeFunction(noop));
    history.set("forward".to_string(), JSValue::NativeFunction(noop));
    history.set("go".to_string(), JSValue::NativeFunction(noop));
    history.set("pushState".to_string(), JSValue::NativeFunction(noop));
    history.set("replaceState".to_string(), JSValue::NativeFunction(noop));

    let event_constructor = make_event_constructor(false);
    let custom_event_constructor = make_event_constructor(true);

    let mut global = engine.global_mut().borrow_mut();
    global.set(
        "navigator".to_string(),
        JSValue::Object(Rc::new(RefCell::new(navigator))),
    );
    global.set(
        "localStorage".to_string(),
        JSValue::Object(make_storage("local")),
    );
    global.set(
        "sessionStorage".to_string(),
        JSValue::Object(make_storage("session")),
    );
    global.set(
        "location".to_string(),
        JSValue::Object(Rc::new(RefCell::new(location))),
    );
    global.set(
        "history".to_string(),
        JSValue::Object(Rc::new(RefCell::new(history))),
    );
    global.set("devicePixelRatio".to_string(), JSValue::Number(1.0));
    global.set("innerWidth".to_string(), JSValue::Number(800.0));
    global.set("innerHeight".to_string(), JSValue::Number(600.0));
    global.set("outerWidth".to_string(), JSValue::Number(800.0));
    global.set("outerHeight".to_string(), JSValue::Number(600.0));
    // Keep feature detection safe while allowing formatjs to select and load
    // its individual constructor polyfills.
    let mut intl = JSObject::new();
    intl.set(
        "getCanonicalLocales".to_string(),
        JSValue::NativeFunction(intl_get_canonical_locales),
    );
    let mut locale = JSObject::new();
    locale.set(
        "__construct__".to_string(),
        JSValue::NativeFunction(intl_locale_constructor),
    );
    intl.set(
        "Locale".to_string(),
        JSValue::Object(Rc::new(RefCell::new(locale))),
    );
    intl.set(
        "PluralRules".to_string(),
        make_intl_constructor(
            intl_plural_rules_constructor,
            &[("select", intl_plural_rules_select)],
        ),
    );
    intl.set(
        "RelativeTimeFormat".to_string(),
        make_intl_constructor(
            intl_relative_time_constructor,
            &[("resolvedOptions", intl_relative_time_resolved_options)],
        ),
    );
    intl.set(
        "NumberFormat".to_string(),
        make_intl_constructor(
            intl_number_format_constructor,
            &[("format", intl_number_format_format)],
        ),
    );
    intl.set(
        "DateTimeFormat".to_string(),
        make_intl_constructor(
            intl_date_time_format_constructor,
            &[
                ("format", intl_date_time_format_format),
                ("formatToParts", intl_date_time_format_to_parts),
                ("formatRange", noop),
                ("resolvedOptions", intl_date_time_format_resolved_options),
            ],
        ),
    );
    global.set(
        "Intl".to_string(),
        JSValue::Object(Rc::new(RefCell::new(intl))),
    );
    global.set(
        "matchMedia".to_string(),
        JSValue::NativeFunction(match_media),
    );
    global.set(
        "getComputedStyle".to_string(),
        JSValue::NativeFunction(get_computed_style),
    );
    global.set("Event".to_string(), JSValue::Object(event_constructor));
    global.set(
        "CustomEvent".to_string(),
        JSValue::Object(custom_event_constructor),
    );
    global.set(
        "requestAnimationFrame".to_string(),
        JSValue::NativeFunction(request_animation_frame),
    );
    global.set(
        "cancelAnimationFrame".to_string(),
        JSValue::NativeFunction(clear_timer),
    );
}

fn location_url(vm: &VM) -> String {
    with_host(vm, |host| host.document_url.clone()).unwrap_or_else(|| "about:blank".to_string())
}

fn parsed_location(vm: &VM) -> Option<url::Url> {
    url::Url::parse(&location_url(vm)).ok()
}

fn location_href(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::String(location_url(vm)))
}

fn location_origin(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let origin = parsed_location(vm)
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_else(|| "null".to_string());
    Ok(JSValue::String(origin))
}

fn location_protocol(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let value = parsed_location(vm)
        .map(|url| format!("{}:", url.scheme()))
        .unwrap_or_default();
    Ok(JSValue::String(value))
}

fn location_host(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let value = parsed_location(vm)
        .and_then(|url| url.host_str().map(|host| (host.to_string(), url.port())))
        .map(|(host, port)| port.map_or(host.clone(), |port| format!("{host}:{port}")))
        .unwrap_or_default();
    Ok(JSValue::String(value))
}

fn location_hostname(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let value = parsed_location(vm)
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_default();
    Ok(JSValue::String(value))
}

fn location_port(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let value = parsed_location(vm)
        .and_then(|url| url.port())
        .map(|port| port.to_string())
        .unwrap_or_default();
    Ok(JSValue::String(value))
}

fn location_pathname(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let value = parsed_location(vm)
        .map(|url| url.path().to_string())
        .unwrap_or_default();
    Ok(JSValue::String(value))
}

fn location_search(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let value = parsed_location(vm)
        .and_then(|url| url.query().map(|query| format!("?{query}")))
        .unwrap_or_default();
    Ok(JSValue::String(value))
}

fn location_hash(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let value = parsed_location(vm)
        .and_then(|url| url.fragment().map(|fragment| format!("#{fragment}")))
        .unwrap_or_default();
    Ok(JSValue::String(value))
}

fn get_computed_style(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let element = args.get(1).cloned().unwrap_or(JSValue::Undefined);
    if dom_node(vm, &element).is_none() {
        return Err(JSError::TypeError(
            "getComputedStyle requires an Element".to_string(),
        ));
    }
    get_style(vm, vec![element])
}

pub(crate) fn match_media(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let media = args.get(1).unwrap_or(&JSValue::Undefined).to_string();
    let mut query = JSObject::new();
    query.define_property(
        "media".to_string(),
        Property::read_only(JSValue::String(media)),
    );
    query.define_property(
        "matches".to_string(),
        Property::read_only(JSValue::Boolean(false)),
    );
    query.set("onchange".to_string(), JSValue::Null);
    for name in [
        "addEventListener",
        "removeEventListener",
        "addListener",
        "removeListener",
    ] {
        query.set(name.to_string(), JSValue::NativeFunction(noop));
    }
    Ok(JSValue::Object(Rc::new(RefCell::new(query))))
}

fn request_animation_frame(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(callback) = args.get(1).filter(|value| is_callable(value)).cloned() else {
        return Ok(JSValue::Number(0.0));
    };
    let Some(id) = with_host_mut(vm, |host| {
        host.next_timer_id += 1;
        let id = host.next_timer_id;
        let timestamp = host.time_origin.elapsed().as_secs_f64() * 1_000.0;
        host.timers.push(JsTimer {
            id,
            callback,
            arguments: vec![JSValue::Number(timestamp)],
            deadline: Instant::now() + Duration::from_millis(16),
            interval: None,
        });
        id
    }) else {
        return Ok(JSValue::Number(0.0));
    };
    Ok(JSValue::Number(id as f64))
}

// --- global aliases ---

pub(crate) fn install_global_aliases(engine: &mut pixi_byte::JSEngine) {
    let global = Rc::clone(engine.global_mut());
    let mut global_object = global.borrow_mut();
    global_object.set(
        "addEventListener".to_string(),
        JSValue::NativeFunction(add_document_event_listener),
    );
    global_object.set(
        "removeEventListener".to_string(),
        JSValue::NativeFunction(remove_document_event_listener),
    );
    for name in ["window", "self", "globalThis"] {
        global_object.set(name.to_string(), JSValue::Object(Rc::clone(&global)));
    }
}
