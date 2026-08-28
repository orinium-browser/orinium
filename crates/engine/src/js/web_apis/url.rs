use pixi_byte::value::jsobject::{JSObject, Property};
use pixi_byte::vm::VM;
use pixi_byte::{JSError, JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;

const URL_HREF: &str = "__orinium_url_href";
const SEARCH_PARAMS_DATA: &str = "__orinium_search_params_data";

pub(crate) fn install_url_apis(engine: &mut pixi_byte::JSEngine) {
    let mut url_constructor_object = JSObject::new();
    url_constructor_object.set(
        "__construct__".to_string(),
        JSValue::NativeFunction(url_constructor),
    );
    let mut params_constructor_object = JSObject::new();
    params_constructor_object.set(
        "__construct__".to_string(),
        JSValue::NativeFunction(url_search_params_constructor),
    );
    let mut global = engine.global_mut().borrow_mut();
    global.set(
        "URL".to_string(),
        JSValue::Object(Rc::new(RefCell::new(url_constructor_object))),
    );
    global.set(
        "URLSearchParams".to_string(),
        JSValue::Object(Rc::new(RefCell::new(params_constructor_object))),
    );
}

fn url_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let input = args.get(1).unwrap_or(&JSValue::Undefined).to_string();
    let parsed = url::Url::parse(&input).or_else(|_| {
        let base = args.get(2).unwrap_or(&JSValue::Undefined).to_string();
        url::Url::parse(&base)?.join(&input)
    });
    let parsed = parsed.map_err(|_| JSError::TypeError("Invalid URL".to_string()))?;
    Ok(JSValue::Object(make_url_object(parsed)))
}

fn make_url_object(url: url::Url) -> Rc<RefCell<JSObject>> {
    let mut object = JSObject::new();
    object.set(URL_HREF.to_string(), JSValue::String(url.to_string()));
    for (name, value) in [
        ("href", url.to_string()),
        ("origin", url.origin().ascii_serialization()),
        ("protocol", format!("{}:", url.scheme())),
        (
            "host",
            url.host_str()
                .map(|host| {
                    url.port()
                        .map_or_else(|| host.to_string(), |port| format!("{host}:{port}"))
                })
                .unwrap_or_default(),
        ),
        ("hostname", url.host_str().unwrap_or("").to_string()),
        (
            "port",
            url.port().map(|port| port.to_string()).unwrap_or_default(),
        ),
        ("pathname", url.path().to_string()),
        (
            "search",
            url.query()
                .map(|query| format!("?{query}"))
                .unwrap_or_default(),
        ),
        (
            "hash",
            url.fragment()
                .map(|fragment| format!("#{fragment}"))
                .unwrap_or_default(),
        ),
    ] {
        object.define_property(
            name.to_string(),
            Property::read_only(JSValue::String(value)),
        );
    }
    object.define_property(
        "searchParams".to_string(),
        Property::read_only(JSValue::Object(make_url_search_params(
            url.query().unwrap_or(""),
        ))),
    );
    object.set(
        "toString".to_string(),
        JSValue::NativeFunction(url_to_string),
    );
    object.set("toJSON".to_string(), JSValue::NativeFunction(url_to_string));
    Rc::new(RefCell::new(object))
}

fn url_to_string(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(JSValue::Object(url)) = args.first() else {
        return Err(JSError::TypeError(
            "URL method called on incompatible receiver".to_string(),
        ));
    };
    Ok(url.borrow().get(URL_HREF))
}

fn url_search_params_constructor(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let input = args.get(1).unwrap_or(&JSValue::Undefined).to_string();
    Ok(JSValue::Object(make_url_search_params(
        input.strip_prefix('?').unwrap_or(&input),
    )))
}

fn make_url_search_params(source: &str) -> Rc<RefCell<JSObject>> {
    let mut object = JSObject::new();
    object.set(
        SEARCH_PARAMS_DATA.to_string(),
        JSValue::String(source.to_string()),
    );
    object.set(
        "get".to_string(),
        JSValue::NativeFunction(search_params_get),
    );
    object.set(
        "has".to_string(),
        JSValue::NativeFunction(search_params_has),
    );
    object.set(
        "set".to_string(),
        JSValue::NativeFunction(search_params_set),
    );
    object.set(
        "append".to_string(),
        JSValue::NativeFunction(search_params_append),
    );
    object.set(
        "delete".to_string(),
        JSValue::NativeFunction(search_params_delete),
    );
    object.set(
        "toString".to_string(),
        JSValue::NativeFunction(search_params_to_string),
    );
    Rc::new(RefCell::new(object))
}

fn search_params_receiver(args: &[JSValue]) -> JSResult<Rc<RefCell<JSObject>>> {
    let Some(JSValue::Object(params)) = args.first() else {
        return Err(JSError::TypeError(
            "URLSearchParams method called on incompatible receiver".to_string(),
        ));
    };
    if !matches!(params.borrow().get(SEARCH_PARAMS_DATA), JSValue::String(_)) {
        return Err(JSError::TypeError(
            "URLSearchParams method called on incompatible receiver".to_string(),
        ));
    }
    Ok(Rc::clone(params))
}

fn search_params_pairs(params: &Rc<RefCell<JSObject>>) -> Vec<(String, String)> {
    let JSValue::String(source) = params.borrow().get(SEARCH_PARAMS_DATA) else {
        return Vec::new();
    };
    url::form_urlencoded::parse(source.as_bytes())
        .into_owned()
        .collect()
}

fn set_search_params_pairs(params: &Rc<RefCell<JSObject>>, pairs: &[(String, String)]) {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.extend_pairs(pairs.iter().map(|(key, value)| (key, value)));
    params.borrow_mut().set(
        SEARCH_PARAMS_DATA.to_string(),
        JSValue::String(serializer.finish()),
    );
}

fn search_params_get(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let params = search_params_receiver(&args)?;
    let name = args.get(1).unwrap_or(&JSValue::Undefined).to_string();
    Ok(search_params_pairs(&params)
        .into_iter()
        .find_map(|(key, value)| (key == name).then_some(JSValue::String(value)))
        .unwrap_or(JSValue::Null))
}

fn search_params_has(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let params = search_params_receiver(&args)?;
    let name = args.get(1).unwrap_or(&JSValue::Undefined).to_string();
    Ok(JSValue::Boolean(
        search_params_pairs(&params)
            .into_iter()
            .any(|(key, _)| key == name),
    ))
}

fn search_params_set(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let params = search_params_receiver(&args)?;
    let name = args.get(1).unwrap_or(&JSValue::Undefined).to_string();
    let value = args.get(2).unwrap_or(&JSValue::Undefined).to_string();
    let mut pairs = search_params_pairs(&params);
    pairs.retain(|(key, _)| key != &name);
    pairs.push((name, value));
    set_search_params_pairs(&params, &pairs);
    Ok(JSValue::Undefined)
}

fn search_params_append(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let params = search_params_receiver(&args)?;
    let name = args.get(1).unwrap_or(&JSValue::Undefined).to_string();
    let value = args.get(2).unwrap_or(&JSValue::Undefined).to_string();
    let mut pairs = search_params_pairs(&params);
    pairs.push((name, value));
    set_search_params_pairs(&params, &pairs);
    Ok(JSValue::Undefined)
}

fn search_params_delete(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let params = search_params_receiver(&args)?;
    let name = args.get(1).unwrap_or(&JSValue::Undefined).to_string();
    let mut pairs = search_params_pairs(&params);
    pairs.retain(|(key, _)| key != &name);
    set_search_params_pairs(&params, &pairs);
    Ok(JSValue::Undefined)
}

fn search_params_to_string(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let params = search_params_receiver(&args)?;
    Ok(params.borrow().get(SEARCH_PARAMS_DATA))
}
