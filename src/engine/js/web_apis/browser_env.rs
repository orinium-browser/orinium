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
use crate::engine::js::web_apis::dom::element::{
    get_computed_style_declaration, read_only_accessor_property,
};
use crate::engine::js::web_apis::dom::events::{
    make_event_constructor, make_event_target_constructor,
};
use crate::engine::js::web_apis::storage::make_storage;
use crate::engine::js::web_apis::timers::clear_timer;
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
        Property::read_only(JSValue::from_string(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Orinium/0.1".to_string(),
        )),
    );
    navigator.define_property(
        "language".to_string(),
        host_read_only_property(JSValue::from_string("en-US".to_string())),
    );
    navigator.define_property(
        "languages".to_string(),
        host_read_only_property(
            engine
                .vm()
                .array_from_values(vec![JSValue::from_string("en-US".to_string())]),
        ),
    );
    navigator.define_property(
        "platform".to_string(),
        Property::read_only(JSValue::from_string("Win32".to_string())),
    );
    navigator.define_property(
        "cookieEnabled".to_string(),
        Property::read_only(JSValue::from_bool(true)),
    );
    navigator.define_property(
        "onLine".to_string(),
        Property::read_only(JSValue::from_bool(true)),
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
    location.set("assign".to_string(), JSValue::from_native_function(noop));
    location.set("replace".to_string(), JSValue::from_native_function(noop));
    location.set("reload".to_string(), JSValue::from_native_function(noop));

    let mut history = JSObject::new();
    history.define_property(
        "length".to_string(),
        Property::read_only(JSValue::from_number(1.0)),
    );
    history.define_property("state".to_string(), Property::read_only(JSValue::null()));
    // TODO: Implement session history traversal and pushState/replaceState state updates.
    history.set("back".to_string(), JSValue::from_native_function(noop));
    history.set("forward".to_string(), JSValue::from_native_function(noop));
    history.set("go".to_string(), JSValue::from_native_function(noop));
    history.set("pushState".to_string(), JSValue::from_native_function(noop));
    history.set(
        "replaceState".to_string(),
        JSValue::from_native_function(noop),
    );

    let event_constructor = make_event_constructor(false);
    let custom_event_constructor = make_event_constructor(true);
    let event_target_constructor = make_event_target_constructor();
    // Stub constructors for the remaining UI/event interfaces. YouTube
    // feature-detects `typeof window.<X> === "function"` (notably
    // `KeyboardEvent`, `MouseEvent`) before dispatching synthetic events;
    // undefined ones make kevlar_base.js throw and get swallowed by
    // `_._DumpException`. Each `.prototype` chains to `Event.prototype` so
    // `instanceof Event` still holds.
    let event_prototype = event_constructor
        .try_borrow()
        .ok()
        .and_then(|constructor| constructor.get("prototype").as_object())
        .unwrap_or_else(|| Rc::new(RefCell::new(JSObject::new())));

    let mut global = engine.global_mut().borrow_mut();
    global.set(
        "navigator".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(navigator))),
    );
    global.set(
        "localStorage".to_string(),
        JSValue::from_object(make_storage("local")),
    );
    global.set(
        "sessionStorage".to_string(),
        JSValue::from_object(make_storage("session")),
    );
    global.set(
        "location".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(location))),
    );
    global.set(
        "history".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(history))),
    );
    global.set("devicePixelRatio".to_string(), JSValue::from_number(1.0));
    global.set("innerWidth".to_string(), JSValue::from_number(800.0));
    global.set("innerHeight".to_string(), JSValue::from_number(600.0));
    global.set("outerWidth".to_string(), JSValue::from_number(800.0));
    global.set("outerHeight".to_string(), JSValue::from_number(600.0));
    global.define_property(
        "origin".to_string(),
        read_only_accessor_property(window_origin),
    );
    // Keep feature detection safe while allowing formatjs to select and load
    // its individual constructor polyfills.
    let mut intl = JSObject::new();
    intl.set(
        "getCanonicalLocales".to_string(),
        JSValue::from_native_function(intl_get_canonical_locales),
    );
    let mut locale = JSObject::new();
    locale.set(
        "__construct__".to_string(),
        JSValue::from_native_function(intl_locale_constructor),
    );
    intl.set(
        "Locale".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(locale))),
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
        JSValue::from_object(Rc::new(RefCell::new(intl))),
    );
    global.set(
        "matchMedia".to_string(),
        JSValue::from_native_function(match_media),
    );
    global.set(
        "getComputedStyle".to_string(),
        JSValue::from_native_function(get_computed_style),
    );
    global.set("Event".to_string(), JSValue::from_object(event_constructor));
    global.set(
        "CustomEvent".to_string(),
        JSValue::from_object(custom_event_constructor),
    );
    global.set(
        "EventTarget".to_string(),
        JSValue::from_object(event_target_constructor),
    );
    // Stub constructors for the remaining UI/event interfaces. YouTube
    // feature-detects `typeof window.<X> === "function"` (notably
    // `KeyboardEvent`, `MouseEvent`) before dispatching synthetic events;
    // undefined ones make kevlar_base.js throw and get swallowed by
    // `_._DumpException`. Each `.prototype` chains to `Event.prototype` so
    // `instanceof Event` still holds.
    for name in [
        "UIEvent",
        "MouseEvent",
        "KeyboardEvent",
        "PointerEvent",
        "FocusEvent",
        "WheelEvent",
        "InputEvent",
        "TouchEvent",
        "DragEvent",
        "CompositionEvent",
        "ProgressEvent",
        "ErrorEvent",
        "MessageEvent",
        "AnimationEvent",
        "TransitionEvent",
        "DeviceMotionEvent",
        "DeviceOrientationEvent",
        "ClipboardEvent",
        "BeforeUnloadEvent",
        "HashChangeEvent",
        "PopStateEvent",
        "StorageEvent",
        "SubmitEvent",
    ] {
        let prototype = JSObject::with_prototype(Some(Rc::clone(&event_prototype)));
        let prototype = Rc::new(RefCell::new(prototype));
        let mut constructor = JSObject::new();
        // `__call__` makes `typeof X` report "function" and `__construct__`
        // makes `new X(...)` work (returning an instance linked to `X.prototype`).
        constructor.set("__call__".to_string(), JSValue::from_native_function(noop));
        constructor.set(
            "__construct__".to_string(),
            JSValue::from_native_function(noop),
        );
        constructor.define_property(
            "prototype".to_string(),
            Property::read_only(JSValue::from_object(Rc::clone(&prototype))),
        );
        global.set(
            name.to_string(),
            JSValue::from_object(Rc::new(RefCell::new(constructor))),
        );
    }
    global.set(
        "requestAnimationFrame".to_string(),
        JSValue::from_native_function(request_animation_frame),
    );
    global.set(
        "cancelAnimationFrame".to_string(),
        JSValue::from_native_function(clear_timer),
    );
}

fn location_url(vm: &VM) -> String {
    with_host(vm, |host| host.document_url.clone()).unwrap_or_else(|| "about:blank".to_string())
}

/// Serialized origin of the current document, `"null"` when opaque.
fn page_origin(vm: &VM) -> String {
    with_host(vm, |host| host.origin.clone()).unwrap_or_else(|| "null".to_string())
}

fn parsed_location(vm: &VM) -> Option<url::Url> {
    url::Url::parse(&location_url(vm)).ok()
}

fn location_href(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::from_string(location_url(vm)))
}

fn location_origin(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::from_string(page_origin(vm)))
}

fn window_origin(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::from_string(page_origin(vm)))
}

fn location_protocol(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let value = parsed_location(vm)
        .map(|url| format!("{}:", url.scheme()))
        .unwrap_or_default();
    Ok(JSValue::from_string(value))
}

fn location_host(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let value = parsed_location(vm)
        .and_then(|url| url.host_str().map(|host| (host.to_string(), url.port())))
        .map(|(host, port)| port.map_or(host.clone(), |port| format!("{host}:{port}")))
        .unwrap_or_default();
    Ok(JSValue::from_string(value))
}

fn location_hostname(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let value = parsed_location(vm)
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_default();
    Ok(JSValue::from_string(value))
}

fn location_port(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let value = parsed_location(vm)
        .and_then(|url| url.port())
        .map(|port| port.to_string())
        .unwrap_or_default();
    Ok(JSValue::from_string(value))
}

fn location_pathname(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let value = parsed_location(vm)
        .map(|url| url.path().to_string())
        .unwrap_or_default();
    Ok(JSValue::from_string(value))
}

fn location_search(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let value = parsed_location(vm)
        .and_then(|url| url.query().map(|query| format!("?{query}")))
        .unwrap_or_default();
    Ok(JSValue::from_string(value))
}

fn location_hash(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let value = parsed_location(vm)
        .and_then(|url| url.fragment().map(|fragment| format!("#{fragment}")))
        .unwrap_or_default();
    Ok(JSValue::from_string(value))
}

fn get_computed_style(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let element = args.get(1).cloned().unwrap_or(JSValue::undefined());
    if dom_node(vm, &element).is_none() {
        return Err(JSError::TypeError(
            "getComputedStyle requires an Element".to_string(),
        ));
    }
    get_computed_style_declaration(vm, vec![element])
}

pub(crate) fn match_media(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let media = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    let matches = evaluate_match_media(&media, vm);
    let mut query = JSObject::new();
    query.define_property(
        "media".to_string(),
        Property::read_only(JSValue::from_string(media)),
    );
    query.define_property(
        "matches".to_string(),
        Property::read_only(JSValue::from_bool(matches)),
    );
    query.set("onchange".to_string(), JSValue::null());
    for name in [
        "addEventListener",
        "removeEventListener",
        "addListener",
        "removeListener",
    ] {
        query.set(name.to_string(), JSValue::from_native_function(noop));
    }
    Ok(JSValue::from_object(Rc::new(RefCell::new(query))))
}

fn evaluate_match_media(media: &str, vm: &VM) -> bool {
    let viewport = with_host(vm, |host| host.viewport).unwrap_or((800.0, 600.0));
    let environment = crate::engine::layouter::css_resolver::MediaEnvironment::new(
        (viewport.0 as f32, viewport.1 as f32),
        crate::engine::layouter::types::ColorScheme::Light,
    );
    crate::engine::css::parser::Parser::parse_media_query(media)
        .map(|query| {
            crate::engine::layouter::css_resolver::evaluate_media_query(&query, &environment)
        })
        .unwrap_or(false)
}

fn request_animation_frame(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(callback) = args.get(1).filter(|value| is_callable(value)).cloned() else {
        return Ok(JSValue::from_number(0.0));
    };
    let Some(id) = with_host_mut(vm, |host| {
        host.next_timer_id += 1;
        let id = host.next_timer_id;
        let timestamp = host.time_origin.elapsed().as_secs_f64() * 1_000.0;
        host.timers.push(JsTimer {
            id,
            callback,
            arguments: vec![JSValue::from_number(timestamp)],
            deadline: Instant::now() + Duration::from_millis(16),
            interval: None,
        });
        id
    }) else {
        return Ok(JSValue::from_number(0.0));
    };
    Ok(JSValue::from_number(id as f64))
}

// --- global aliases ---

pub(crate) fn install_global_aliases(engine: &mut pixi_byte::JSEngine) {
    let global = Rc::clone(engine.global_mut());
    let mut global_object = global.borrow_mut();
    global_object.set(
        "addEventListener".to_string(),
        JSValue::from_native_function(add_document_event_listener),
    );
    global_object.set(
        "removeEventListener".to_string(),
        JSValue::from_native_function(remove_document_event_listener),
    );
    global_object.set(
        "dispatchEvent".to_string(),
        JSValue::from_native_function(window_dispatch_event),
    );
    for name in ["window", "self", "globalThis"] {
        global_object.set(name.to_string(), JSValue::from_object(Rc::clone(&global)));
    }
    // `Window` constructor whose `.prototype` is the window object itself, so
    // polyfills feature-detecting `Window.prototype` (webcomponents-sd) see the
    // real window surface.
    let mut window_constructor = JSObject::new();
    window_constructor.define_property(
        "prototype".to_string(),
        Property::read_only(JSValue::from_object(Rc::clone(&global))),
    );
    global_object.set(
        "Window".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(window_constructor))),
    );
}

/// `window.dispatchEvent(event)`: dispatches `event` to the listeners registered
/// through `window.addEventListener` (stored in `host.document_event_listeners`).
pub(crate) fn window_dispatch_event(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(event) = args.get(1).and_then(JSValue::as_object) else {
        return Err(JSError::TypeError(
            "dispatchEvent requires an Event".to_string(),
        ));
    };
    let event_type = event.borrow().get("type").to_string();
    if event_type.is_empty() {
        return Err(JSError::TypeError(
            "Event type must not be empty".to_string(),
        ));
    }

    let window = JSValue::from_object(Rc::clone(&vm.global_object));
    event.borrow_mut().set("target".to_string(), window.clone());
    event
        .borrow_mut()
        .set("currentTarget".to_string(), window.clone());
    event
        .borrow_mut()
        .set("eventPhase".to_string(), JSValue::from_number(2.0));

    let listeners = with_host(vm, |host| {
        host.document_event_listeners
            .get(&event_type)
            .cloned()
            .unwrap_or_default()
    })
    .unwrap_or_default();
    let dispatched = !listeners.is_empty();
    for listener in listeners {
        if crate::engine::js::common::is_callable(&listener) {
            vm.call(
                listener,
                window.clone(),
                vec![JSValue::from_object(Rc::clone(&event))],
            )?;
        }
    }
    Ok(JSValue::from_bool(dispatched))
}
