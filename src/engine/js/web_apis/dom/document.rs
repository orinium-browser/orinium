use crate::engine::html::{DomTree, HtmlNodeType, Parser as HtmlParser};
use crate::engine::js::JsHost;
use crate::engine::js::common::{
    dom_node, is_callable, mark_dom_dirty, node_dom_id, noop, with_host, with_host_mut,
};
use crate::engine::js::web_apis::browser_env::window_dispatch_event;
use crate::engine::js::web_apis::dom::custom_elements::link_custom_element_prototype;
use crate::engine::js::web_apis::dom::dom_exception::throw_dom_exception;
use crate::engine::js::web_apis::dom::element::{
    accessor_property, define_node_constants, make_comment_node, make_doctype_node,
    make_document_fragment, make_element, make_node_interface, make_processing_instruction_node,
    make_text_node, read_only_accessor_property,
};
use crate::engine::js::web_apis::dom::node_iterator;
use crate::engine::js::web_apis::dom::node_iterator::{make_node_iterator, make_tree_walker};
use crate::engine::tree::{NodeRef, TreeNode};
use pixi_byte::value::jsobject::{JSObject, Property};
use pixi_byte::vm::VM;
use pixi_byte::{JSError, JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) fn install_document(engine: &mut pixi_byte::JSEngine) {
    let document_obj = Rc::new(RefCell::new(JSObject::new()));
    {
        let mut document = document_obj.borrow_mut();
        document.define_property(
            "nodeType".to_string(),
            Property::read_only(JSValue::from_number(9.0)),
        );
        document.define_property(
            "nodeName".to_string(),
            Property::read_only(JSValue::from_string("#document".to_string())),
        );
        define_node_constants(&mut document);
        document.define_property(
            "documentElement".to_string(),
            read_only_accessor_property(get_document_element),
        );
        document.define_property(
            "childNodes".to_string(),
            read_only_accessor_property(get_document_child_nodes),
        );
        document.define_property(
            "firstChild".to_string(),
            read_only_accessor_property(get_document_first_child),
        );
        document.define_property(
            "lastChild".to_string(),
            read_only_accessor_property(get_document_last_child),
        );
        document.define_property(
            "body".to_string(),
            read_only_accessor_property(get_document_body),
        );
        document.define_property(
            "head".to_string(),
            read_only_accessor_property(get_document_head),
        );
        document.define_property(
            "activeElement".to_string(),
            read_only_accessor_property(get_active_element),
        );
        document.define_property(
            "defaultView".to_string(),
            read_only_accessor_property(get_document_default_view),
        );
        document.define_property(
            "readyState".to_string(),
            read_only_accessor_property(get_document_ready_state),
        );
        document.define_property(
            "origin".to_string(),
            read_only_accessor_property(get_document_origin),
        );
        document.define_property(
            "implementation".to_string(),
            read_only_accessor_property(get_document_implementation),
        );
        document.define_property(
            "cookie".to_string(),
            accessor_property(get_document_cookie, set_document_cookie),
        );
        document.set(
            "hasFocus".to_string(),
            JSValue::from_native_function(document_has_focus),
        );
        document.set(
            "elementFromPoint".to_string(),
            JSValue::from_native_function(document_element_from_point),
        );
        document.set(
            "elementsFromPoint".to_string(),
            JSValue::from_native_function(document_elements_from_point),
        );
        document.set(
            "execCommand".to_string(),
            JSValue::from_native_function(document_exec_command),
        );
        document.set(
            "getSelection".to_string(),
            JSValue::from_native_function(document_get_selection),
        );
        document.define_property(
            "pictureInPictureElement".to_string(),
            read_only_accessor_property(get_picture_in_picture_element),
        );
        document.define_property(
            "fullscreenElement".to_string(),
            read_only_accessor_property(get_fullscreen_element),
        );
        document.define_property(
            "pictureInPictureEnabled".to_string(),
            Property::read_only(JSValue::from_bool(true)),
        );
        document.define_property(
            "fullscreenEnabled".to_string(),
            Property::read_only(JSValue::from_bool(true)),
        );
        document.set(
            "getElementById".to_string(),
            JSValue::from_native_function(get_element_by_id),
        );
        document.set(
            "querySelector".to_string(),
            JSValue::from_native_function(document_query_selector),
        );
        document.set(
            "querySelectorAll".to_string(),
            JSValue::from_native_function(document_query_selector_all),
        );
        document.set(
            "getElementsByTagName".to_string(),
            JSValue::from_native_function(document_get_elements_by_tag_name),
        );
        document.set(
            "getElementsByClassName".to_string(),
            JSValue::from_native_function(document_get_elements_by_class_name),
        );
        document.define_property(
            "forms".to_string(),
            read_only_accessor_property(get_document_forms),
        );
        document.set(
            "createElement".to_string(),
            JSValue::from_native_function(create_element),
        );
        document.set(
            "importNode".to_string(),
            JSValue::from_native_function(crate::engine::js::web_apis::dom::element::import_node),
        );
        document.set(
            "createElementNS".to_string(),
            JSValue::from_native_function(create_element_ns),
        );
        document.set(
            "createNodeIterator".to_string(),
            JSValue::from_native_function(create_node_iterator),
        );
        document.set(
            "createTreeWalker".to_string(),
            JSValue::from_native_function(create_tree_walker),
        );
        document.set(
            "createTextNode".to_string(),
            JSValue::from_native_function(create_text_node),
        );
        document.set(
            "createDocumentFragment".to_string(),
            JSValue::from_native_function(create_document_fragment),
        );
        document.set(
            "createComment".to_string(),
            JSValue::from_native_function(create_comment),
        );
        document.set(
            "createProcessingInstruction".to_string(),
            JSValue::from_native_function(create_processing_instruction),
        );
        document.set(
            "createEvent".to_string(),
            JSValue::from_native_function(super::events::make_create_event),
        );
        document.set(
            "addEventListener".to_string(),
            JSValue::from_native_function(add_document_event_listener),
        );
        document.set(
            "removeEventListener".to_string(),
            JSValue::from_native_function(remove_document_event_listener),
        );
        document.set(
            "dispatchEvent".to_string(),
            JSValue::from_native_function(window_dispatch_event),
        );
        document.set(
            "write".to_string(),
            JSValue::from_native_function(document_write),
        );
        document.set(
            "writeln".to_string(),
            JSValue::from_native_function(document_writeln),
        );
        document.set(
            "close".to_string(),
            JSValue::from_native_function(document_close),
        );
    }
    let _ = with_host_mut(engine.vm(), |host| {
        host.document = Some(Rc::clone(&document_obj));
    });
    install_document_implementation(engine);
    engine
        .global_mut()
        .borrow_mut()
        .set("document".to_string(), JSValue::from_object(document_obj));

    if let Some(element_constructor) =
        with_host(engine.vm(), |host| Rc::clone(&host.element_constructor))
    {
        let mut global = engine.global_mut().borrow_mut();
        global.set(
            "Element".to_string(),
            JSValue::from_object(Rc::clone(&element_constructor)),
        );
        global.set(
            "HTMLElement".to_string(),
            JSValue::from_object(element_constructor),
        );
    }

    let mut iframe_constructor = JSObject::new();
    iframe_constructor.set(
        "__host_has_instance__".to_string(),
        JSValue::from_native_function(html_iframe_element_has_instance),
    );
    engine.global_mut().borrow_mut().set(
        "HTMLIFrameElement".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(iframe_constructor))),
    );

    // HTMLTemplateElement constructor with a prototype chained onto the
    // element prototype. Polymer and webcomponents-sd feature-detect
    // `window.HTMLTemplateElement` and read `template.content`.
    if let Some(element_prototype) =
        with_host(engine.vm(), |host| Rc::clone(&host.element_prototype))
    {
        let template_prototype = JSObject::with_prototype(Some(element_prototype));
        let template_prototype = Rc::new(RefCell::new(template_prototype));
        let mut template_constructor = JSObject::new();
        // `__call__` makes `typeof X` report "function" and `__construct__`
        // makes `new X(...)` work (returning an instance linked to `X.prototype`).
        template_constructor.set("__call__".to_string(), JSValue::from_native_function(noop));
        template_constructor.set(
            "__construct__".to_string(),
            JSValue::from_native_function(noop),
        );
        template_constructor.set(
            "__host_has_instance__".to_string(),
            JSValue::from_native_function(html_template_element_has_instance),
        );
        template_constructor.define_property(
            "prototype".to_string(),
            Property::read_only(JSValue::from_object(Rc::clone(&template_prototype))),
        );
        engine.global_mut().borrow_mut().set(
            "HTMLTemplateElement".to_string(),
            JSValue::from_object(Rc::new(RefCell::new(template_constructor))),
        );
    }

    // `Image`: `new Image(width, height)` creates an `<img>` element (same as
    // `document.createElement('img')`). YouTube's prewarm script does
    // `new Image().src = "https://…"`.
    engine.global_mut().borrow_mut().set(
        "Image".to_string(),
        JSValue::from_native_function(image_constructor),
    );

    // DOMException constructor
    engine.global_mut().borrow_mut().set(
        "DOMException".to_string(),
        JSValue::from_object(super::dom_exception::make_dom_exception_constructor()),
    );
    // Link DOMException.prototype -> Error.prototype so instanceof checks work.
    // We use Object.setPrototypeOf via eval since pixi_byte doesn't expose
    // the prototype chain through JSObject APIs.
    let _ = engine.eval("Object.setPrototypeOf(DOMException.prototype, Error.prototype)");

    // `ShadowRoot` interface constructor. `new ShadowRoot()` is illegal, but
    // scripts feature-detect `typeof ShadowRoot === 'function'` and check
    // `shadowRoot instanceof ShadowRoot`; the host hook matches the exposed
    // shadow-root wrapper objects (their `__orinium_is_shadow_root` marker).
    {
        let shadow_prototype = Rc::new(RefCell::new(JSObject::new()));
        if let Some(node_prototype) = with_host(engine.vm(), |host| host.node_prototype.clone())
            .flatten()
        {
            shadow_prototype
                .borrow_mut()
                .set_prototype(Some(node_prototype));
        }
        let mut shadow_constructor = JSObject::new();
        // `__call__` makes `typeof ShadowRoot` report "function".
        shadow_constructor.set("__call__".to_string(), JSValue::from_native_function(shadow_root_construct_error));
        shadow_constructor.set(
            "__construct__".to_string(),
            JSValue::from_native_function(shadow_root_construct_error),
        );
        shadow_constructor.define_property(
            "prototype".to_string(),
            Property::read_only(JSValue::from_object(Rc::clone(&shadow_prototype))),
        );
        shadow_constructor.set(
            "__host_has_instance__".to_string(),
            JSValue::from_native_function(shadow_root_has_instance),
        );
        // Mark the interface prototype so exposed wrappers can be recognized.
        shadow_prototype.borrow_mut().set(
            "__orinium_shadow_root_interface".to_string(),
            JSValue::from_bool(true),
        );
        engine.global_mut().borrow_mut().set(
            "ShadowRoot".to_string(),
            JSValue::from_object(Rc::new(RefCell::new(shadow_constructor))),
        );
        // Exposed shadow-root wrappers chain this interface prototype so
        // `instanceof ShadowRoot` and feature detection see the full chain.
        let _ = with_host_mut(engine.vm(), |host| {
            host.shadow_root_prototype = Some(shadow_prototype);
        });
    }

    // Miscellaneous DOM interface constructors. Polymer and the
    // webcomponents polyfills feature-detect these as `typeof X === 'function'`
    // / `instanceof` before using them; missing ones make large chunks of
    // kevlar_base.js throw and get swallowed by `_._DumpException`, which
    // then never reaches `_.nb`/`customElements.define`.
    // Each constructor gets a prototype chaining to the element prototype
    // (matching the naked `Element`/`HTMLElement` exposure above) so
    // `SVGElement.prototype` etc. resolve.
    if let Some(element_prototype) =
        with_host(engine.vm(), |host| Rc::clone(&host.element_prototype))
    {
        for (name, _tag) in [
            ("SVGElement", "svg"),
            ("SVGSVGElement", "svg"),
            ("SVGGraphicsElement", "svg"),
            ("SVGPathElement", "path"),
            ("SVGUseElement", "use"),
            ("SVGTextElement", "text"),
            ("SVGImageElement", "svg"),
            ("SVGRectElement", "rect"),
            ("SVGCircleElement", "circle"),
            ("SVGEllipseElement", "ellipse"),
            ("SVGLineElement", "line"),
            ("SVGPolygonElement", "polygon"),
            ("SVGPolylineElement", "polyline"),
            ("SVGDefsElement", "defs"),
            ("SVGLinearGradientElement", "linearGradient"),
            ("SVGRadialGradientElement", "radialGradient"),
            ("SVGStopElement", "stop"),
            ("SVGFilterElement", "filter"),
            ("SVGTextPathElement", "textPath"),
        ] {
            let prototype = JSObject::with_prototype(Some(Rc::clone(&element_prototype)));
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
            engine.global_mut().borrow_mut().set(
                name.to_string(),
                JSValue::from_object(Rc::new(RefCell::new(constructor))),
            );
        }
        for name in [
            "HTMLDocument",
            "HTMLSlotElement",
            "HTMLUnknownElement",
            "HTMLLinkElement",
            "HTMLStyleElement",
            "HTMLHeadElement",
            "HTMLBodyElement",
            "HTMLDivElement",
            "HTMLSpanElement",
            "HTMLParagraphElement",
            "HTMLAnchorElement",
            "HTMLButtonElement",
            "HTMLInputElement",
            "HTMLFormElement",
            "HTMLUListElement",
            "HTMLLIElement",
            "HTMLHRElement",
            "HTMLBRElement",
            "HTMLHeadingElement",
            "HTMLVideoElement",
            "HTMLAudioElement",
            "HTMLMediaElement",
            "HTMLCanvasElement",
            "HTMLSourceElement",
            "HTMLMetaElement",
            "HTMLLabelElement",
            "HTMLSelectElement",
            "HTMLTextAreaElement",
            "HTMLOptionElement",
            "HTMLImageElement",
            "HtmlElement",
            "HTMLPictureElement",
            "HTMLEmbedElement",
            "HTMLObjectElement",
            "HTMLTrackElement",
            "HTMLTableElement",
            "HTMLTableRowElement",
            "HTMLTableCellElement",
            "HTMLTableSectionElement",
            "HTMLModElement",
            "HTMLHRElement",
        ] {
            let prototype = JSObject::with_prototype(Some(Rc::clone(&element_prototype)));
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
            engine.global_mut().borrow_mut().set(
                name.to_string(),
                JSValue::from_object(Rc::new(RefCell::new(constructor))),
            );
        }
    }

    // Node constants
    let (node_prototype, node_constructor) = make_node_interface();
    {
        let mut node_obj = node_constructor.borrow_mut();
        node_obj.define_property(
            "ELEMENT_NODE".to_string(),
            Property::read_only(JSValue::from_number(1.0)),
        );
        node_obj.define_property(
            "ATTRIBUTE_NODE".to_string(),
            Property::read_only(JSValue::from_number(2.0)),
        );
        node_obj.define_property(
            "TEXT_NODE".to_string(),
            Property::read_only(JSValue::from_number(3.0)),
        );
        node_obj.define_property(
            "CDATA_SECTION_NODE".to_string(),
            Property::read_only(JSValue::from_number(4.0)),
        );
        node_obj.define_property(
            "PROCESSING_INSTRUCTION_NODE".to_string(),
            Property::read_only(JSValue::from_number(7.0)),
        );
        node_obj.define_property(
            "COMMENT_NODE".to_string(),
            Property::read_only(JSValue::from_number(8.0)),
        );
        node_obj.define_property(
            "DOCUMENT_NODE".to_string(),
            Property::read_only(JSValue::from_number(9.0)),
        );
        node_obj.define_property(
            "DOCUMENT_TYPE_NODE".to_string(),
            Property::read_only(JSValue::from_number(10.0)),
        );
        node_obj.define_property(
            "DOCUMENT_FRAGMENT_NODE".to_string(),
            Property::read_only(JSValue::from_number(11.0)),
        );
    }
    let _ = with_host_mut(engine.vm(), |host| {
        host.node_prototype = Some(Rc::clone(&node_prototype));
    });
    engine
        .global_mut()
        .borrow_mut()
        .set("Node".to_string(), JSValue::from_object(node_constructor));
    // Wire Element.prototype -> Node.prototype so element instances inherit
    // Node methods (getRootNode, contains, …).
    let _ = engine.eval("Object.setPrototypeOf(Element.prototype, Node.prototype)");

    // Character-data constructors: `Text`, `Comment`, `CDATASection`,
    // `ProcessingInstruction` and `CharacterData`, plus `Document`.
    // WebComponents polyfills read `window.Text.prototype` (via `Object.create`)
    // while feature-detecting, so each constructor carries a prototype object.
    for name in [
        "Text",
        "Comment",
        "CDATASection",
        "ProcessingInstruction",
        "CharacterData",
        "Document",
        "DocumentType",
    ] {
        let mut prototype = JSObject::new();
        prototype.set_prototype(Some(Rc::clone(&node_prototype)));
        let prototype = Rc::new(RefCell::new(prototype));
        let mut constructor = JSObject::new();
        constructor.define_property(
            "prototype".to_string(),
            Property::read_only(JSValue::from_object(Rc::clone(&prototype))),
        );
        engine.global_mut().borrow_mut().set(
            name.to_string(),
            JSValue::from_object(Rc::new(RefCell::new(constructor))),
        );
    }

    // DocumentFragment: instances get a dedicated prototype (chained onto
    // Node.prototype) so `fragment instanceof DocumentFragment` works and
    // `template.content`/document.createDocumentFragment results are recognized.
    let fragment_prototype = {
        let mut inner = JSObject::new();
        inner.set_prototype(Some(Rc::clone(&node_prototype)));
        Rc::new(RefCell::new(inner))
    };
    {
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
            Property::read_only(JSValue::from_object(Rc::clone(&fragment_prototype))),
        );
        engine.global_mut().borrow_mut().set(
            "DocumentFragment".to_string(),
            JSValue::from_object(Rc::new(RefCell::new(constructor))),
        );
    }
    let _ = with_host_mut(engine.vm(), |host| {
        host.fragment_prototype = Some(Rc::clone(&fragment_prototype));
    });
}

/// `new ShadowRoot()` is illegal; shadow roots come from `attachShadow`.
fn shadow_root_construct_error(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Err(JSError::TypeError(
        "Illegal constructor: ShadowRoot cannot be constructed directly".to_string(),
    ))
}

/// `instanceof ShadowRoot` hook: matches exposed shadow-root wrappers, which
/// carry the `mode` property and `#shadow-root` node name.
fn shadow_root_has_instance(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let matches = args
        .get(1)
        .and_then(JSValue::as_object)
        .map(|object| {
            let borrowed = object.borrow();
            borrowed.get("nodeName").as_string() == Some("#shadow-root")
        })
        .unwrap_or(false);
    Ok(JSValue::from_bool(matches))
}

fn html_iframe_element_has_instance(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = args.get(1).and_then(|value| dom_node(vm, value)) else {
        return Ok(JSValue::from_bool(false));
    };
    let is_iframe = node.borrow().value.tag_name() == Some("iframe");
    Ok(JSValue::from_bool(is_iframe))
}

fn html_template_element_has_instance(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = args.get(1).and_then(|value| dom_node(vm, value)) else {
        return Ok(JSValue::from_bool(false));
    };
    let is_template = node.borrow().value.tag_name() == Some("template");
    Ok(JSValue::from_bool(is_template))
}

pub(crate) fn add_document_event_listener(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(event_type) = args.get(1).and_then(JSValue::as_string_owned) else {
        return Ok(JSValue::undefined());
    };
    let Some(listener) = args.get(2).filter(|value| is_callable(value)).cloned() else {
        return Ok(JSValue::undefined());
    };

    let _ = with_host_mut(vm, |host| {
        let listeners = host
            .document_event_listeners
            .entry(event_type.clone())
            .or_default();
        if !listeners
            .iter()
            .any(|candidate| candidate.strict_equals(&listener))
        {
            listeners.push(listener);
        }
    });
    Ok(JSValue::undefined())
}

pub(crate) fn remove_document_event_listener(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(event_type) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(JSValue::undefined());
    };
    let Some(listener) = args.get(2) else {
        return Ok(JSValue::undefined());
    };
    let _ = with_host_mut(vm, |host| {
        if let Some(listeners) = host.document_event_listeners.get_mut(event_type) {
            listeners.retain(|candidate| !candidate.strict_equals(listener));
        }
    });
    Ok(JSValue::undefined())
}

fn get_element_by_id(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(id) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(JSValue::null());
    };

    let Some(node) = with_host(vm, |host| host.dom.get_element_by_id(id)).flatten() else {
        return Ok(JSValue::null());
    };
    Ok(expose_node(vm, node).unwrap_or(JSValue::null()))
}

fn document_query_selector(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(selector) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(JSValue::null());
    };
    let Some(node) = with_host(vm, |host| host.dom.query_selector(selector)).flatten() else {
        return Ok(JSValue::null());
    };
    Ok(expose_node(vm, node).unwrap_or(JSValue::null()))
}

fn document_query_selector_all(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(selector) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(vm.array_from_values(Vec::new()));
    };
    let nodes = with_host(vm, |host| host.dom.query_selector_all(selector)).unwrap_or_default();
    Ok(expose_node_list(vm, nodes))
}

fn document_get_elements_by_tag_name(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let tag_name = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    let nodes = with_host(vm, |host| {
        if tag_name == "*" {
            host.dom.find_all(|node| node.tag_name().is_some())
        } else {
            host.dom
                .get_elements_by_tag_name(&tag_name.to_ascii_lowercase())
        }
    })
    .unwrap_or_default();
    Ok(expose_node_list(vm, nodes))
}

pub(crate) fn class_selector(value: &JSValue) -> String {
    value
        .to_string()
        .split_whitespace()
        .filter(|class| !class.is_empty())
        .map(|class| format!(".{class}"))
        .collect()
}

fn document_get_elements_by_class_name(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let selector = class_selector(args.get(1).unwrap_or(&JSValue::undefined()));
    if selector.is_empty() {
        return Ok(vm.array_from_values(Vec::new()));
    }
    let nodes = with_host(vm, |host| host.dom.query_selector_all(&selector)).unwrap_or_default();
    Ok(expose_node_list(vm, nodes))
}

/// `document.forms` — the collection of `<form>` elements in the document.
/// Exposed as an array-like so `document.forms[0]` and `document.forms.length`
/// work; Reddit's challenge page relies on `document.forms[0]`.
fn get_document_forms(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let nodes = with_host(vm, |host| host.dom.get_elements_by_tag_name("form")).unwrap_or_default();
    Ok(expose_node_list(vm, nodes))
}

/// `new Image(width, height)` — creates an `<img>` element just like
/// `document.createElement('img')`. Matching browsers, `Image.prototype`
/// inherits from `HTMLElement.prototype`, so `new Image() instanceof
/// HTMLElement` is true.
fn image_constructor(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let node = TreeNode::new(HtmlNodeType::Element {
        tag_name: "img".to_string(),
        attributes: Vec::new(),
    });
    let value = expose_detached_node(vm, node).unwrap_or(JSValue::null());
    if let (Some(node_id), Some(width), Some(height)) = (
        node_dom_id(&value),
        args.first().and_then(|a| a.as_number()),
        args.get(1).and_then(|a| a.as_number()),
    ) {
        let _ = with_host_mut(vm, |host| {
            if let Some(node) = host.refs.get(&node_id).and_then(|n| n.upgrade()) {
                node.borrow_mut().value.set_attr("width", width.to_string());
                node.borrow_mut()
                    .value
                    .set_attr("height", height.to_string());
            }
        });
    }
    Ok(value)
}

pub(crate) fn create_element(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(tag_name) = args.get(1).and_then(JSValue::as_string) else {
        return Err(throw_dom_exception(
            "Must provide a tag name",
            "SyntaxError",
        ));
    };
    let tag_name = tag_name.trim().to_ascii_lowercase();
    if !is_valid_element_name(&tag_name) {
        return Err(throw_dom_exception(
            "The tag name provided ('{tag_name}') is not a valid name",
            "InvalidCharacterError",
        ));
    }
    let node = TreeNode::new(HtmlNodeType::Element {
        tag_name,
        attributes: Vec::new(),
    });
    Ok(expose_detached_node(vm, node).unwrap_or(JSValue::null()))
}

fn create_element_ns(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let namespace = match args.get(1) {
        Some(value) if value.is_string() => value.as_string().unwrap_or("").to_string(),
        Some(value) if value.is_null() || value.is_undefined() => String::new(),
        Some(value) => value.to_console_string(),
        None => String::new(),
    };
    let Some(qualified_name) = args.get(2).and_then(JSValue::as_string) else {
        return Err(throw_dom_exception(
            "Must provide a qualified name",
            "SyntaxError",
        ));
    };
    // The qualified name is preserved exactly (case and prefix) so it can be
    // reported through tagName/nodeName/localName/prefix per the DOM spec.
    let qualified_name = qualified_name.trim().to_string();
    let (prefix, local_name) = match validate_qualified_name(&qualified_name) {
        Ok(parts) => parts,
        Err(code) => {
            return Err(throw_dom_exception(
                "The qualified name provided is not a valid name",
                exception_name_for_code(code),
            ));
        }
    };
    validate_namespace(&prefix, &local_name, &namespace).map_err(|code| {
        throw_dom_exception(
            "The qualified name and namespace provided are not valid",
            exception_name_for_code(code),
        )
    })?;

    let node = TreeNode::new(HtmlNodeType::Element {
        tag_name: qualified_name,
        attributes: Vec::new(),
    });
    let value = expose_detached_node(vm, node).unwrap_or(JSValue::null());
    if let Some(dom_id) = node_dom_id(&value) {
        let _ = with_host_mut(vm, |host| {
            host.namespaces.insert(dom_id, namespace);
        });
    }
    Ok(value)
}

fn create_node_iterator(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let root = args.get(1).cloned().unwrap_or(JSValue::undefined());
    let what_to_show = args
        .get(2)
        .map(JSValue::to_number)
        .unwrap_or(node_iterator::SHOW_ALL as f64) as u32;
    let filter = args.get(3).cloned();
    make_node_iterator(vm, root, what_to_show, filter)
}

fn create_tree_walker(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let root = args.get(1).cloned().unwrap_or(JSValue::undefined());
    let what_to_show = args
        .get(2)
        .map(JSValue::to_number)
        .unwrap_or(node_iterator::SHOW_ALL as f64) as u32;
    let filter = args.get(3).cloned();
    make_tree_walker(vm, root, what_to_show, filter)
}

pub(crate) fn create_text_node(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let text = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    let node = TreeNode::new(HtmlNodeType::Text(text));
    Ok(expose_detached_node(vm, node).unwrap_or(JSValue::null()))
}

fn create_document_fragment(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let node = TreeNode::new(HtmlNodeType::DocumentFragment);
    Ok(expose_detached_node(vm, node).unwrap_or(JSValue::null()))
}

fn create_comment(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let data = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    let node = TreeNode::new(HtmlNodeType::Comment(data));
    Ok(expose_detached_node(vm, node).unwrap_or(JSValue::null()))
}

fn create_processing_instruction(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let target = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    let data = args
        .get(2)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    if target.is_empty() {
        return Err(throw_dom_exception(
            "Processing instruction target must not be empty",
            "SyntaxError",
        ));
    }
    let node = TreeNode::new(HtmlNodeType::ProcessingInstruction { target, data });
    Ok(expose_detached_node(vm, node).unwrap_or(JSValue::null()))
}

fn get_document_element(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let node = with_host(vm, |host| {
        host.dom
            .root
            .borrow()
            .children()
            .iter()
            .find(|child| matches!(child.borrow().value, HtmlNodeType::Element { .. }))
            .cloned()
    })
    .flatten();
    Ok(node
        .and_then(|node| expose_node(vm, node))
        .unwrap_or(JSValue::null()))
}

fn get_document_child_nodes(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let children =
        with_host(vm, |host| host.dom.root.borrow().children().to_vec()).unwrap_or_default();
    Ok(expose_node_list(vm, children))
}

fn get_document_first_child(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(with_host(vm, |host| {
        host.dom.root.borrow().children().first().cloned()
    })
    .flatten()
    .and_then(|node| expose_node(vm, node))
    .unwrap_or(JSValue::null()))
}

fn get_document_last_child(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(
        with_host(vm, |host| host.dom.root.borrow().children().last().cloned())
            .flatten()
            .and_then(|node| expose_node(vm, node))
            .unwrap_or(JSValue::null()),
    )
}

fn get_document_body(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let node = with_host(vm, |host| host.dom.query_selector("body")).flatten();
    Ok(node
        .and_then(|node| expose_node(vm, node))
        .unwrap_or(JSValue::null()))
}

fn get_document_head(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let node = with_host(vm, |host| host.dom.query_selector("head")).flatten();
    Ok(node
        .and_then(|node| expose_node(vm, node))
        .unwrap_or(JSValue::null()))
}

fn get_active_element(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let active = with_host(vm, |host| {
        host.active_element
            .and_then(|dom_id| host.objects.get(&dom_id).cloned())
    })
    .flatten();
    if let Some(active) = active {
        return Ok(JSValue::from_object(active));
    }
    get_document_body(vm, Vec::new())
}

fn get_document_default_view(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::from_object(Rc::clone(&vm.global_object)))
}

fn get_document_ready_state(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let complete = with_host(vm, |host| host.dom_content_loaded_fired).unwrap_or(false);
    Ok(JSValue::from_string(
        if complete { "complete" } else { "loading" }.to_string(),
    ))
}

fn get_document_origin(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let origin = with_host(vm, |host| host.origin.clone()).unwrap_or_else(|| "null".to_string());
    Ok(JSValue::from_string(origin))
}

fn get_document_implementation(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let implementation = with_host(vm, |host| {
        host.document_implementation.as_ref().map(Rc::clone)
    })
    .flatten();
    Ok(implementation
        .map(JSValue::from_object)
        .unwrap_or(JSValue::undefined()))
}

fn install_document_implementation(engine: &mut pixi_byte::JSEngine) {
    let mut implementation = JSObject::new();
    implementation.set(
        "createDocumentType".to_string(),
        JSValue::from_native_function(dom_implementation_create_document_type),
    );
    implementation.set(
        "createDocument".to_string(),
        JSValue::from_native_function(dom_implementation_create_document),
    );
    implementation.set(
        "hasFeature".to_string(),
        JSValue::from_native_function(dom_implementation_has_feature),
    );
    implementation.set(
        "createHTMLDocument".to_string(),
        JSValue::from_native_function(dom_implementation_create_html_document),
    );
    let implementation = Rc::new(RefCell::new(implementation));
    let _ = with_host_mut(engine.vm(), |host| {
        host.document_implementation = Some(Rc::clone(&implementation));
    });
}

fn dom_implementation_has_feature(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::from_bool(true))
}

fn dom_implementation_create_document_type(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(qualified_name) = args.get(1).and_then(JSValue::as_string) else {
        return Err(throw_dom_exception(
            "Must provide a qualified name",
            "SyntaxError",
        ));
    };
    let qualified_name = qualified_name.trim();
    if validate_qualified_name(qualified_name).is_err() {
        return Err(throw_dom_exception(
            "The qualified name provided is not a valid name",
            "NamespaceError",
        ));
    }
    if qualified_name.contains(':') {
        // A qualified name for a DocumentType must not contain a prefix.
        return Err(throw_dom_exception(
            "A DocumentType qualified name must not have a prefix",
            "NamespaceError",
        ));
    }
    let public_id = args
        .get(2)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    let system_id = args
        .get(3)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    let node = TreeNode::new(HtmlNodeType::Doctype {
        name: Some(qualified_name.to_string()),
        public_id: Some(public_id),
        system_id: Some(system_id),
    });
    Ok(expose_detached_node(vm, node).unwrap_or(JSValue::null()))
}

fn dom_implementation_create_document(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let namespace = args
        .get(1)
        .filter(|value| value.is_string())
        .map(|value| value.as_string().unwrap_or("").to_string())
        .unwrap_or_default();
    let qualified_name = args
        .get(2)
        .and_then(JSValue::as_string)
        .map(|name| name.trim().to_string())
        .unwrap_or_default();

    let node = TreeNode::new(HtmlNodeType::Document);
    let doc = expose_detached_node(vm, node).unwrap_or(JSValue::null());

    if !qualified_name.is_empty() {
        let (prefix, local_name) = validate_qualified_name(&qualified_name).map_err(|code| {
            throw_dom_exception(
                "The qualified name provided is not a valid name",
                exception_name_for_code(code),
            )
        })?;
        validate_namespace(&prefix, &local_name, &namespace).map_err(|code| {
            throw_dom_exception(
                "The qualified name and namespace provided are not valid",
                exception_name_for_code(code),
            )
        })?;
        let element = TreeNode::new(HtmlNodeType::Element {
            tag_name: qualified_name,
            attributes: Vec::new(),
        });
        let value = expose_detached_node(vm, element).unwrap_or(JSValue::null());
        if let Some(dom_id) = node_dom_id(&value) {
            let _ = with_host_mut(vm, |host| {
                host.namespaces.insert(dom_id, namespace);
            });
        }
        if let Some(doc_dom_id) = node_dom_id(&doc)
            && let Some(Some(Some(root))) =
                with_host(vm, |host| host.refs.get(&doc_dom_id).map(|w| w.upgrade()))
            && let Some(child) = dom_node(vm, &value)
        {
            TreeNode::append_child(&root, child);
        }
    }
    Ok(doc)
}

/// `document.implementation.createHTMLDocument(title)`:
/// creates a new, detached HTML document with `<head><title>…</title></head>`
/// and `<body>` children under `<html>`.
fn dom_implementation_create_html_document(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let title_text = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();

    let doc_node = TreeNode::new(HtmlNodeType::Document);
    let html_node = TreeNode::new(HtmlNodeType::Element {
        tag_name: "html".to_string(),
        attributes: Vec::new(),
    });
    let head_node = TreeNode::new(HtmlNodeType::Element {
        tag_name: "head".to_string(),
        attributes: Vec::new(),
    });
    let body_node = TreeNode::new(HtmlNodeType::Element {
        tag_name: "body".to_string(),
        attributes: Vec::new(),
    });
    let title_node = TreeNode::new(HtmlNodeType::Element {
        tag_name: "title".to_string(),
        attributes: Vec::new(),
    });
    let title_text_node = TreeNode::new(HtmlNodeType::Text(title_text.clone()));

    TreeNode::add_child(&doc_node, Rc::clone(&html_node));
    TreeNode::add_child(&html_node, Rc::clone(&head_node));
    TreeNode::add_child(&html_node, Rc::clone(&body_node));
    TreeNode::add_child(&head_node, Rc::clone(&title_node));
    TreeNode::add_child(&title_node, Rc::clone(&title_text_node));

    let html_value = expose_detached_node(vm, html_node).unwrap_or(JSValue::null());
    let head_value = expose_detached_node(vm, head_node).unwrap_or(JSValue::null());
    let body_value = expose_detached_node(vm, body_node).unwrap_or(JSValue::null());

    let mut document = JSObject::new();
    document.define_property(
        "nodeType".to_string(),
        Property::read_only(JSValue::from_number(9.0)),
    );
    document.define_property(
        "nodeName".to_string(),
        Property::read_only(JSValue::from_string("#document".to_string())),
    );
    define_node_constants(&mut document);
    document.define_property(
        "namespaceURI".to_string(),
        Property::read_only(JSValue::from_string(
            "http://www.w3.org/1999/xhtml".to_string(),
        )),
    );
    document.set("documentElement".to_string(), html_value);
    document.set("head".to_string(), head_value);
    document.set("body".to_string(), body_value);
    document.set("title".to_string(), JSValue::from_string(title_text));
    document.set(
        "createElement".to_string(),
        JSValue::from_native_function(create_element),
    );
    document.set(
        "createElementNS".to_string(),
        JSValue::from_native_function(create_element_ns),
    );
    document.set(
        "createTextNode".to_string(),
        JSValue::from_native_function(create_text_node),
    );
    document.set(
        "createComment".to_string(),
        JSValue::from_native_function(create_comment),
    );
    document.set(
        "createDocumentFragment".to_string(),
        JSValue::from_native_function(create_document_fragment),
    );
    document.set(
        "createEvent".to_string(),
        JSValue::from_native_function(super::events::make_create_event),
    );
    document.set(
        "createNodeIterator".to_string(),
        JSValue::from_native_function(create_node_iterator),
    );
    document.set(
        "createTreeWalker".to_string(),
        JSValue::from_native_function(create_tree_walker),
    );
    document.set(
        "querySelector".to_string(),
        JSValue::from_native_function(document_query_selector),
    );
    document.set(
        "querySelectorAll".to_string(),
        JSValue::from_native_function(document_query_selector_all),
    );
    document.set(
        "getElementById".to_string(),
        JSValue::from_native_function(get_element_by_id),
    );
    document.set(
        "getElementsByTagName".to_string(),
        JSValue::from_native_function(document_get_elements_by_tag_name),
    );
    document.set(
        "getElementsByClassName".to_string(),
        JSValue::from_native_function(document_get_elements_by_class_name),
    );
    document.set(
        "dispatchEvent".to_string(),
        JSValue::from_native_function(window_dispatch_event),
    );
    let detached_doc = Rc::new(RefCell::new(document));
    let _ = with_host_mut(vm, |host| {
        host.detached_documents.push(Rc::clone(&detached_doc));
    });
    Ok(JSValue::from_object(detached_doc))
}

fn get_document_cookie(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let cookies = with_host(vm, |host| {
        host.document_cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    })
    .unwrap_or_default();
    Ok(JSValue::from_string(cookies))
}

fn set_document_cookie(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let cookie = args
        .get(1)
        .unwrap_or(&JSValue::undefined())
        .to_console_string();
    let Some(pair) = cookie.split(';').next() else {
        return Ok(JSValue::undefined());
    };
    let Some((name, value)) = pair.split_once('=') else {
        return Ok(JSValue::undefined());
    };
    let name = name.trim();
    if name.is_empty() {
        return Ok(JSValue::undefined());
    }

    let should_remove = cookie.split(';').skip(1).any(|attribute| {
        let attribute = attribute.trim();
        attribute.eq_ignore_ascii_case("max-age=0")
            || attribute
                .strip_prefix("Max-Age=")
                .and_then(|value| value.trim().parse::<i64>().ok())
                .is_some_and(|max_age| max_age <= 0)
    });
    let _ = with_host_mut(vm, |host| {
        if should_remove {
            host.document_cookies.remove(name);
        } else {
            host.document_cookies
                .insert(name.to_string(), value.trim().to_string());
        }
    });
    Ok(JSValue::undefined())
}

fn document_has_focus(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::from_bool(true))
}

// ---------------------------------------------------------------------------
// Hit-testing, selection, editing and presentation-state accessors
// ---------------------------------------------------------------------------

/// Resolves the exposed element whose layout rect contains (`x`, `y`),
/// preferring the innermost (last in document order) match like browsers do.
fn element_at_layout_point(vm: &VM, x: f64, y: f64) -> Option<NodeRef<HtmlNodeType>> {
    let dom_id = with_host(vm, |host| {
        let mut found: Option<u64> = None;
        for (&candidate_id, metrics) in &host.layout_metrics_by_dom_id {
            let contains = x >= metrics.rect_left
                && x < metrics.rect_left + metrics.rect_width
                && y >= metrics.rect_top
                && y < metrics.rect_top + metrics.rect_height;
            if contains && candidate_id > found.unwrap_or(0) {
                found = Some(candidate_id);
            }
        }
        found
    })
    .flatten()?;
    with_host(vm, |host| {
        host.refs.get(&dom_id).and_then(|weak| weak.upgrade())
    })
    .flatten()
}

/// `document.elementFromPoint(x, y)` — the topmost element whose layout rect
/// contains the viewport point, or `null`. Layout metrics are keyed by DOM id,
/// so this is only meaningful after a committed layout.
fn document_element_from_point(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let x = args.get(1).map(JSValue::to_number).unwrap_or(f64::NAN);
    let y = args.get(2).map(JSValue::to_number).unwrap_or(f64::NAN);
    if !x.is_finite() || !y.is_finite() {
        return Err(throw_dom_exception(
            "elementFromPoint: coordinates must be finite numbers",
            "TypeError",
        ));
    }
    let found = element_at_layout_point(vm, x, y)
        .and_then(|node| expose_node(vm, node));
    Ok(found.unwrap_or(JSValue::null()))
}

/// `document.elementsFromPoint(x, y)` — every element whose layout rect
/// contains the point, innermost first. The engine keeps one rect per element,
/// so the stack is the containing elements in reverse registration order.
fn document_elements_from_point(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let x = args.get(1).map(JSValue::to_number).unwrap_or(f64::NAN);
    let y = args.get(2).map(JSValue::to_number).unwrap_or(f64::NAN);
    if !x.is_finite() || !y.is_finite() {
        return Err(throw_dom_exception(
            "elementsFromPoint: coordinates must be finite numbers",
            "TypeError",
        ));
    }
    let ids = with_host(vm, |host| {
        let mut hits: Vec<u64> = host
            .layout_metrics_by_dom_id
            .iter()
            .filter(|(_, metrics)| {
                x >= metrics.rect_left
                    && x < metrics.rect_left + metrics.rect_width
                    && y >= metrics.rect_top
                    && y < metrics.rect_top + metrics.rect_height
            })
            .map(|(&id, _)| id)
            .collect();
        hits.sort_unstable_by(|a, b| b.cmp(a));
        hits
    })
    .unwrap_or_default();
    let values = ids
        .into_iter()
        .filter_map(|id| {
            with_host(vm, |host| {
                host.refs.get(&id).and_then(|weak| weak.upgrade())
            })
            .flatten()
            .and_then(|node| expose_node(vm, node))
        })
        .collect();
    Ok(vm.array_from_values(values))
}

/// `document.execCommand(command, showDefaultUI, value)` — the engine has no
/// editing session, so commands are accepted and reported as no-ops. Only
/// unknown commands report failure, matching the "unsupported command"
/// contract scripts rely on.
fn document_exec_command(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let command = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    Ok(JSValue::from_bool(!command.is_empty()))
}

/// `document.getSelection()` — a singleton `Selection` with no ranges (the
/// engine has no text-selection state). The object is stable across calls.
fn document_get_selection(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let selection = with_host(vm, |host| {
        host.document_selection.as_ref().map(Rc::clone)
    })
    .flatten();
    if let Some(selection) = selection {
        return Ok(JSValue::from_object(selection));
    }
    let selection = Rc::new(RefCell::new(make_selection()));
    let _ = with_host_mut(vm, |host| {
        host.document_selection = Some(Rc::clone(&selection));
    });
    Ok(JSValue::from_object(selection))
}

fn make_selection() -> JSObject {
    let mut selection = JSObject::new();
    selection.define_property(
        "rangeCount".to_string(),
        Property::read_only(JSValue::from_number(0.0)),
    );
    selection.define_property(
        "isCollapsed".to_string(),
        Property::read_only(JSValue::from_bool(true)),
    );
    selection.define_property("anchorNode".to_string(), Property::read_only(JSValue::null()));
    selection.define_property("focusNode".to_string(), Property::read_only(JSValue::null()));
    selection.define_property("type".to_string(), Property::read_only(JSValue::from_string("None".to_string())));
    for name in ["getRangeAt", "removeAllRanges", "addRange", "collapse", "selectAllChildren"] {
        selection.set(name.to_string(), JSValue::from_native_function(selection_noop));
    }
    selection
}

fn selection_noop(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::undefined())
}

/// `document.pictureInPictureElement` — `null` unless a PiP request is active,
/// which this engine does not initiate.
fn get_picture_in_picture_element(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::null())
}

/// `document.fullscreenElement` — the element that most recently entered
/// fullscreen via `requestFullscreen()`, or `null`.
fn get_fullscreen_element(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let element = with_host(vm, |host| {
        host.fullscreen_element
            .and_then(|dom_id| host.objects.get(&dom_id).cloned())
    })
    .flatten();
    Ok(element
        .map(JSValue::from_object)
        .unwrap_or(JSValue::null()))
}

pub(crate) fn expose_detached_node(vm: &mut VM, node: NodeRef<HtmlNodeType>) -> Option<JSValue> {
    let value = expose_node(vm, Rc::clone(&node))?;
    let dom_id = node_dom_id(&value)?;
    with_host_mut(vm, |host| {
        host.detached_nodes.insert(dom_id, node);
    })?;
    Some(value)
}

// Expose a DOM node as a JavaScript object.
pub(crate) fn expose_node(vm: &VM, node: NodeRef<HtmlNodeType>) -> Option<JSValue> {
    let node_kind = {
        let borrowed = node.borrow();
        match &borrowed.value {
            HtmlNodeType::Element { tag_name, .. } => NodeKind::Element {
                tag_name: tag_name.clone(),
                id: borrowed.value.get_attr("id").unwrap_or("").to_string(),
            },
            HtmlNodeType::Text(_) => NodeKind::Text,
            HtmlNodeType::Comment(_) => NodeKind::Comment,
            HtmlNodeType::ProcessingInstruction { .. } => NodeKind::ProcessingInstruction,
            HtmlNodeType::DocumentFragment => NodeKind::Fragment,
            HtmlNodeType::Doctype {
                name,
                public_id: _,
                system_id: _,
            } => NodeKind::Doctype { name: name.clone() },
            HtmlNodeType::Document => {
                return with_host(vm, |host| host.document.as_ref().cloned())
                    .flatten()
                    .map(JSValue::from_object);
            }
            _ => return None,
        }
    };

    expose_node_inner(vm, node, node_kind)
}

enum NodeKind {
    Element { tag_name: String, id: String },
    Text,
    Comment,
    ProcessingInstruction,
    Fragment,
    Doctype { name: Option<String> },
}

fn expose_node_inner(vm: &VM, node: NodeRef<HtmlNodeType>, kind: NodeKind) -> Option<JSValue> {
    let dom_id = with_host_mut(vm, |host| {
        if let Some(dom_id) = host.dom_id_for_node(&node) {
            return dom_id;
        }
        host.next_id += 1;
        let dom_id = host.next_id;
        host.refs.insert(dom_id, Rc::downgrade(&node));
        dom_id
    })?;

    let obj = with_host_mut(vm, |host| {
        if let Some(existing) = host.objects.get(&dom_id) {
            return Rc::clone(existing);
        }

        let is_element = matches!(kind, NodeKind::Element { .. });

        let obj = match &kind {
            NodeKind::Element { tag_name, id } => make_element(
                tag_name.clone(),
                id.clone(),
                dom_id,
                Rc::clone(&host.element_prototype),
                Rc::clone(&host.element_constructor),
            ),
            NodeKind::Text => make_text_node(dom_id),
            NodeKind::Comment => make_comment_node(dom_id),
            NodeKind::ProcessingInstruction => make_processing_instruction_node(dom_id),
            NodeKind::Fragment => make_document_fragment(dom_id),
            NodeKind::Doctype { name } => make_doctype_node(dom_id, name.clone()),
        };

        // Custom elements inherit the class prototype so their methods are
        // reachable from `this` inside lifecycle callbacks and event handlers.
        if let NodeKind::Element { tag_name, .. } = &kind
            && let Some(definition) = host.custom_elements.get(tag_name)
            && let Some(class_proto) = link_custom_element_prototype(&obj, definition)
        {
            obj.borrow_mut().set_prototype(Some(class_proto));
        }

        // Non-element nodes are created without a prototype; wire them to
        // their interface prototype so they inherit node methods (getRootNode,
        // contains, dispatchEvent, …). Fragments chain the dedicated
        // DocumentFragment prototype; other nodes chain Node.prototype.
        if !is_element {
            let proto = match &kind {
                NodeKind::Fragment => host.fragment_prototype.clone(),
                _ => host.node_prototype.clone(),
            };
            if let Some(proto) = proto {
                obj.borrow_mut().set_prototype(Some(proto));
            }
        }

        host.objects.insert(dom_id, Rc::clone(&obj));
        obj
    })?;

    Some(JSValue::from_object(obj))
}

pub(crate) fn expose_node_list(vm: &mut VM, nodes: Vec<NodeRef<HtmlNodeType>>) -> JSValue {
    let values = nodes
        .into_iter()
        .filter_map(|node| expose_node(vm, node))
        .collect();
    vm.array_from_values(values)
}

// ---------------------------------------------------------------------------
// ShadowRoot exposure
// ---------------------------------------------------------------------------

/// Creates or retrieves the JS object wrapping a shadow root DOM node.
pub(crate) fn expose_shadow_root(
    vm: &VM,
    node: &NodeRef<HtmlNodeType>,
    dom_id: u64,
) -> Option<Rc<RefCell<JSObject>>> {
    let mode_str = match &node.borrow().value {
        HtmlNodeType::ShadowRoot { mode } => match mode {
            crate::engine::html::ShadowRootMode::Open => "open",
            crate::engine::html::ShadowRootMode::Closed => "closed",
        },
        _ => return None,
    };

    with_host_mut(vm, |host| {
        if let Some(existing) = host.objects.get(&dom_id) {
            return Rc::clone(existing);
        }
        let mut obj = JSObject::new();
        // __orinium_dom_id for DOM tree lookups
        obj.define_property(
            "__orinium_dom_id".to_string(),
            Property {
                value: JSValue::from_number(dom_id as f64),
                enumerable: false,
                configurable: false,
                writable: false,
                getter: None,
                setter: None,
            },
        );
        // nodeType = 11 (DOCUMENT_FRAGMENT_NODE)
        obj.define_property(
            "nodeType".to_string(),
            Property::read_only(JSValue::from_number(11.0)),
        );
        obj.define_property(
            "nodeName".to_string(),
            Property::read_only(JSValue::from_string("#shadow-root".to_string())),
        );
        obj.define_property(
            "mode".to_string(),
            Property::read_only(JSValue::from_string(mode_str.to_string())),
        );
        obj.define_property(
            "textContent".to_string(),
            accessor_property(get_shadow_text_content, set_shadow_text_content),
        );
        // querySelector / querySelectorAll
        obj.define_property(
            "querySelector".to_string(),
            Property::read_only(JSValue::from_native_function(shadow_query_selector)),
        );
        obj.define_property(
            "querySelectorAll".to_string(),
            Property::read_only(JSValue::from_native_function(shadow_query_selector_all)),
        );
        // DOM mutation methods (delegate to element functions).
        obj.set(
            "appendChild".to_string(),
            JSValue::from_native_function(super::element::append_child),
        );
        obj.set(
            "removeChild".to_string(),
            JSValue::from_native_function(super::element::remove_child),
        );
        obj.set(
            "insertBefore".to_string(),
            JSValue::from_native_function(super::element::insert_before),
        );
        obj.set(
            "replaceChild".to_string(),
            JSValue::from_native_function(super::element::replace_child),
        );
        obj.set(
            "cloneNode".to_string(),
            JSValue::from_native_function(super::element::clone_node),
        );
        obj.define_property(
            "children".to_string(),
            read_only_accessor_property(super::element::get_element_children),
        );
        let host_obj = Rc::new(RefCell::new(obj));
        // Shadow roots chain the `ShadowRoot` interface prototype (falling
        // back to Node.prototype) so `instanceof ShadowRoot` holds.
        let proto = host
            .shadow_root_prototype
            .clone()
            .or_else(|| host.node_prototype.clone());
        if let Some(proto) = proto {
            host_obj.borrow_mut().set_prototype(Some(proto));
        }
        host.objects.insert(dom_id, Rc::clone(&host_obj));
        host_obj
    })
}

// ShadowRoot native functions

fn get_shadow_text_content(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let this = args.first().cloned().unwrap_or(JSValue::undefined());
    let dom_id = node_dom_id(&this)
        .ok_or_else(|| JSError::TypeError("textContent: not a node".to_string()))?;
    let node = with_host(vm, |host| host.refs.get(&dom_id).cloned())
        .flatten()
        .and_then(|w| w.upgrade())
        .ok_or_else(|| JSError::TypeError("textContent: node not found".to_string()))?;
    Ok(JSValue::from_string(DomTree::inner_text(&node)))
}

fn set_shadow_text_content(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let this = args.first().cloned().unwrap_or(JSValue::undefined());
    let new_text = args.get(1).and_then(JSValue::as_string).unwrap_or("");
    let dom_id = node_dom_id(&this)
        .ok_or_else(|| JSError::TypeError("textContent: not a node".to_string()))?;
    let node = with_host(vm, |host| host.refs.get(&dom_id).cloned())
        .flatten()
        .and_then(|w| w.upgrade())
        .ok_or_else(|| JSError::TypeError("textContent: node not found".to_string()))?;
    DomTree::set_text_content(&node, new_text);
    Ok(JSValue::undefined())
}

fn shadow_query_selector(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let this = args.first().cloned().unwrap_or(JSValue::undefined());
    let dom_id = node_dom_id(&this)
        .ok_or_else(|| JSError::TypeError("querySelector: not a node".to_string()))?;
    let selector = args.get(1).and_then(JSValue::as_string).unwrap_or("");
    let node = with_host(vm, |host| host.refs.get(&dom_id).cloned())
        .flatten()
        .and_then(|w| w.upgrade())
        .ok_or_else(|| JSError::TypeError("querySelector: node not found".to_string()))?;
    let result = DomTree::query_selector_all_within(&node, selector)
        .into_iter()
        .next();
    match result {
        Some(found) => Ok(super::document::expose_node(vm, found).unwrap_or(JSValue::null())),
        None => Ok(JSValue::null()),
    }
}

fn shadow_query_selector_all(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let this = args.first().cloned().unwrap_or(JSValue::undefined());
    let dom_id = node_dom_id(&this)
        .ok_or_else(|| JSError::TypeError("querySelectorAll: not a node".to_string()))?;
    let selector = args.get(1).and_then(JSValue::as_string).unwrap_or("");
    let node = with_host(vm, |host| host.refs.get(&dom_id).cloned())
        .flatten()
        .and_then(|w| w.upgrade())
        .ok_or_else(|| JSError::TypeError("querySelectorAll: node not found".to_string()))?;
    let results = DomTree::query_selector_all_within(&node, selector);
    Ok(super::document::expose_node_list(vm, results))
}

// ---------------------------------------------------------------------------
// document.write / document.writeln
// ---------------------------------------------------------------------------

fn document_write(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let text = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    if text.is_empty() {
        return Ok(JSValue::undefined());
    }

    // Reuse existing HTML parser
    let parsed = HtmlParser::new(&text).parse();

    // Find insertion target: prefer existing <body>
    let target = with_host(vm, |host| host.dom.query_selector("body")).flatten();

    let Some(target) = target else {
        // No <body> yet — nothing to insert into
        return Ok(JSValue::undefined());
    };

    // Extract children from the parsed body, not the root.
    // Parser::new(text).parse() produces Document -> html -> body -> content,
    // so we must drill into the parsed body to avoid inserting a nested <html>.
    let source = parsed
        .query_selector("body")
        .unwrap_or_else(|| Rc::clone(&parsed.root));

    let children: Vec<_> = source.borrow().children().to_vec();
    for child in children {
        TreeNode::append_child(&target, Rc::clone(&child));
        mark_dom_dirty(vm);
    }

    Ok(JSValue::undefined())
}

fn document_writeln(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let text = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    let mut new_args = args;
    new_args.push(JSValue::from_string(format!("{text}\n")));
    document_write(vm, new_args)
}

/// `document.close()` — signals the end of a write() sequence.
///
/// In this engine there is no async write buffering, so close() is a no-op
/// that simply returns undefined, matching the spec's observable behavior for
/// non-rendered documents.
fn document_close(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::undefined())
}

// ---------------------------------------------------------------------------
// Iframe contentDocument support
// ---------------------------------------------------------------------------

/// A lightweight per-iframe document. Holds the iframe's own DOM tree and the
/// JS `document` object exposed as `iframe.contentDocument`.
pub(crate) struct IframeDocument {
    pub(crate) tree: Rc<DomTree>,
    pub(crate) document: Rc<RefCell<JSObject>>,
    /// The exposed `<html>` element node of this iframe document.
    pub(crate) document_element: NodeRef<HtmlNodeType>,
}

/// Builds an `iframe.contentDocument`-compatible object: an empty document
/// with its own DOM tree whose `documentElement` is `<html>`.
pub(crate) fn make_iframe_document(host_dom_id: u64) -> Rc<RefCell<IframeDocument>> {
    let document = TreeNode::new(HtmlNodeType::Document);
    let html = TreeNode::new(HtmlNodeType::Element {
        tag_name: "html".to_string(),
        attributes: Vec::new(),
    });
    let head = TreeNode::new(HtmlNodeType::Element {
        tag_name: "head".to_string(),
        attributes: Vec::new(),
    });
    let body = TreeNode::new(HtmlNodeType::Element {
        tag_name: "body".to_string(),
        attributes: Vec::new(),
    });
    TreeNode::add_child(&document, Rc::clone(&html));
    TreeNode::add_child(&html, Rc::clone(&head));
    TreeNode::add_child(&html, Rc::clone(&body));
    let tree = Rc::new(DomTree::from_root(document));

    let document_obj = build_iframe_document_object(host_dom_id, &tree);

    Rc::new(RefCell::new(IframeDocument {
        tree,
        document: document_obj,
        document_element: html,
    }))
}

/// Parses fetched HTML and installs it as the content document of the iframe
/// with the given DOM id. Returns `true` if a document was installed.
pub(crate) fn install_parsed_iframe_document(
    host: &mut JsHost,
    iframe_dom_id: u64,
    html: &str,
) -> bool {
    let tree = Rc::new(HtmlParser::new(html).parse());
    let document_element = tree
        .query_selector("html")
        .unwrap_or_else(|| Rc::clone(&tree.root));
    let document_obj = build_iframe_document_object(iframe_dom_id, &tree);
    let iframe_doc = Rc::new(RefCell::new(IframeDocument {
        tree,
        document: document_obj,
        document_element,
    }));
    host.iframe_documents.insert(iframe_dom_id, iframe_doc);
    true
}

fn build_iframe_document_object(host_dom_id: u64, _tree: &Rc<DomTree>) -> Rc<RefCell<JSObject>> {
    let mut document = JSObject::new();
    document.define_property(
        "__orinium_iframe_id".to_string(),
        Property {
            value: JSValue::from_number(host_dom_id as f64),
            enumerable: false,
            writable: false,
            configurable: false,
            getter: None,
            setter: None,
        },
    );
    document.define_property(
        "nodeType".to_string(),
        Property::read_only(JSValue::from_number(9.0)),
    );
    define_node_constants(&mut document);
    document.define_property(
        "documentElement".to_string(),
        read_only_accessor_property(iframe_document_element),
    );
    document.define_property(
        "body".to_string(),
        read_only_accessor_property(iframe_document_body),
    );
    document.define_property(
        "head".to_string(),
        read_only_accessor_property(iframe_document_head),
    );
    document.set(
        "getElementById".to_string(),
        JSValue::from_native_function(iframe_get_element_by_id),
    );
    document.set(
        "querySelector".to_string(),
        JSValue::from_native_function(iframe_query_selector),
    );
    document.set(
        "querySelectorAll".to_string(),
        JSValue::from_native_function(iframe_query_selector_all),
    );
    document.set(
        "getElementsByTagName".to_string(),
        JSValue::from_native_function(iframe_get_elements_by_tag_name),
    );
    document.set(
        "getElementsByClassName".to_string(),
        JSValue::from_native_function(iframe_get_elements_by_class_name),
    );
    document.set(
        "createElement".to_string(),
        JSValue::from_native_function(create_element),
    );
    document.set(
        "createElementNS".to_string(),
        JSValue::from_native_function(create_element_ns),
    );
    document.set(
        "createNodeIterator".to_string(),
        JSValue::from_native_function(create_node_iterator),
    );
    document.set(
        "createTreeWalker".to_string(),
        JSValue::from_native_function(create_tree_walker),
    );
    document.set(
        "appendChild".to_string(),
        JSValue::from_native_function(iframe_append_child),
    );
    document.set(
        "removeChild".to_string(),
        JSValue::from_native_function(iframe_remove_child),
    );
    document.set(
        "createTextNode".to_string(),
        JSValue::from_native_function(create_text_node),
    );
    document.set(
        "createComment".to_string(),
        JSValue::from_native_function(create_comment),
    );
    document.set(
        "createDocumentFragment".to_string(),
        JSValue::from_native_function(create_document_fragment),
    );
    document.set(
        "createEvent".to_string(),
        JSValue::from_native_function(super::events::make_create_event),
    );
    document.set(
        "createRange".to_string(),
        JSValue::from_native_function(create_range_stub),
    );
    document.set(
        "dispatchEvent".to_string(),
        JSValue::from_native_function(window_dispatch_event),
    );
    document.set(
        "write".to_string(),
        JSValue::from_native_function(iframe_document_write),
    );
    document.set(
        "close".to_string(),
        JSValue::from_native_function(iframe_document_close),
    );
    Rc::new(RefCell::new(document))
}

fn iframe_get_iframe_doc(vm: &VM, this: &JSValue) -> Option<Rc<RefCell<IframeDocument>>> {
    let iframe_id = this.as_object().and_then(|o| {
        o.borrow()
            .get("__orinium_iframe_id")
            .as_number()
            .map(|n| n as u64)
    })?;
    with_host(vm, |host| {
        host.iframe_documents.get(&iframe_id).map(Rc::clone)
    })
    .flatten()
}

fn iframe_document_element(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let this = args.first().cloned().unwrap_or(JSValue::undefined());
    let Some(doc) = iframe_get_iframe_doc(vm, &this) else {
        return Ok(JSValue::null());
    };
    Ok(expose_node(vm, Rc::clone(&doc.borrow().document_element)).unwrap_or(JSValue::null()))
}

fn iframe_document_body(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let this = args.first().cloned().unwrap_or(JSValue::undefined());
    let Some(doc) = iframe_get_iframe_doc(vm, &this) else {
        return Ok(JSValue::null());
    };
    Ok(doc
        .borrow()
        .tree
        .query_selector("body")
        .map(|node| expose_node(vm, node).unwrap_or(JSValue::null()))
        .unwrap_or(JSValue::null()))
}

fn iframe_document_head(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let this = args.first().cloned().unwrap_or(JSValue::undefined());
    let Some(doc) = iframe_get_iframe_doc(vm, &this) else {
        return Ok(JSValue::null());
    };
    Ok(doc
        .borrow()
        .tree
        .query_selector("head")
        .map(|node| expose_node(vm, node).unwrap_or(JSValue::null()))
        .unwrap_or(JSValue::null()))
}

fn iframe_receiver_tree(vm: &VM, args: &[JSValue]) -> Option<Rc<DomTree>> {
    let this = args.first().cloned().unwrap_or(JSValue::undefined());
    iframe_get_iframe_doc(vm, &this).map(|doc| Rc::clone(&doc.borrow().tree))
}

fn iframe_get_element_by_id(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(id) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(JSValue::null());
    };
    let Some(tree) = iframe_receiver_tree(vm, &args) else {
        return Ok(JSValue::null());
    };
    let Some(node) = tree.get_element_by_id(id) else {
        return Ok(JSValue::null());
    };
    Ok(expose_node(vm, node).unwrap_or(JSValue::null()))
}

fn iframe_query_selector(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(selector) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(JSValue::null());
    };
    let Some(tree) = iframe_receiver_tree(vm, &args) else {
        return Ok(JSValue::null());
    };
    let Some(node) = tree.query_selector(selector) else {
        return Ok(JSValue::null());
    };
    Ok(expose_node(vm, node).unwrap_or(JSValue::null()))
}

fn iframe_query_selector_all(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(selector) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(vm.array_from_values(Vec::new()));
    };
    let Some(tree) = iframe_receiver_tree(vm, &args) else {
        return Ok(vm.array_from_values(Vec::new()));
    };
    let nodes = tree.query_selector_all(selector);
    Ok(expose_node_list(vm, nodes))
}

fn iframe_get_elements_by_tag_name(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let tag_name = args.get(1).unwrap_or(&JSValue::undefined()).to_string();
    let Some(tree) = iframe_receiver_tree(vm, &args) else {
        return Ok(vm.array_from_values(Vec::new()));
    };
    let nodes = if tag_name == "*" {
        tree.find_all(|node| node.tag_name().is_some())
    } else {
        tree.get_elements_by_tag_name(&tag_name.to_ascii_lowercase())
    };
    Ok(expose_node_list(vm, nodes))
}

fn iframe_get_elements_by_class_name(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let selector = class_selector(args.get(1).unwrap_or(&JSValue::undefined()));
    let Some(tree) = iframe_receiver_tree(vm, &args) else {
        return Ok(vm.array_from_values(Vec::new()));
    };
    let nodes = if selector.is_empty() {
        Vec::new()
    } else {
        tree.query_selector_all(&selector)
    };
    Ok(expose_node_list(vm, nodes))
}

fn iframe_document_write(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(tree) = iframe_receiver_tree(vm, &args) else {
        return Ok(JSValue::undefined());
    };
    let text = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    let parsed = HtmlParser::new(&text).parse();
    let source = parsed
        .query_selector("body")
        .unwrap_or_else(|| Rc::clone(&parsed.root));
    let children: Vec<_> = source.borrow().children().to_vec();
    let target = tree.query_selector("body");
    if let Some(target) = target {
        for child in children {
            TreeNode::append_child(&target, Rc::clone(&child));
        }
    }
    mark_dom_dirty(vm);
    Ok(JSValue::undefined())
}

fn iframe_document_close(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::undefined())
}

fn iframe_document_root(vm: &mut VM, args: &[JSValue]) -> Option<NodeRef<HtmlNodeType>> {
    let this = args.first().cloned().unwrap_or(JSValue::undefined());
    let doc = iframe_get_iframe_doc(vm, &this)?;
    let root = Rc::clone(&doc.borrow().tree.root);
    Some(root)
}

fn iframe_append_child(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(parent) = iframe_document_root(vm, &args) else {
        return Ok(JSValue::null());
    };
    let Some(child_value) = args.get(1).cloned() else {
        return Ok(JSValue::null());
    };
    let Some(child) = dom_node(vm, &child_value) else {
        return Ok(JSValue::null());
    };
    TreeNode::append_child(&parent, Rc::clone(&child));
    let _ = with_host_mut(vm, |host| {
        if let Some(dom_id) = node_dom_id(&child_value) {
            host.detached_nodes.remove(&dom_id);
        }
    });
    mark_dom_dirty(vm);
    Ok(child_value)
}

fn iframe_remove_child(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(parent) = iframe_document_root(vm, &args) else {
        return Ok(JSValue::null());
    };
    let Some(child_value) = args.get(1).cloned() else {
        return Ok(JSValue::null());
    };
    let Some(child) = dom_node(vm, &child_value) else {
        return Ok(JSValue::null());
    };
    TreeNode::remove_child(&parent, &child);
    mark_dom_dirty(vm);
    Ok(child_value)
}

fn create_range_stub(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::undefined())
}

/// Validates a qualified element name per a simplified ASCII subset of the
/// XML `Name` production.
///
/// - Must not be empty.
/// - Local part: starts with `[a-zA-Z]`, followed by `[a-zA-Z0-9\-_\.]`.
/// - If a colon is present, there must be exactly one, and the prefix must
///   follow the same rules as the local part.
fn is_valid_element_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut parts = name.split(':');
    let local = parts.next().unwrap_or("");
    // At most one colon (prefix:local)
    if parts.next().is_some() && parts.next().is_some() {
        return false;
    }
    // Validate the local part
    local_part_valid(local)
}

fn local_part_valid(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// Validates a qualified name per the XML `QName` production and extracts its
/// prefix (if any) and local name. A single `prefix:local` split is allowed.
///
/// Returns `Err(code)` where `code` is the legacy DOMException code to throw:
/// `5` (INVALID_CHARACTER_ERR) for a malformed name, `14` (NAMESPACE_ERR) when
/// the colon is present but the prefix is empty.
fn validate_qualified_name(name: &str) -> Result<(Option<String>, String), f64> {
    if name.is_empty() {
        return Err(5.0);
    }
    let mut parts = name.split(':');
    let first = parts.next().unwrap_or("");
    let second = parts.next();
    let third = parts.next();

    match (first, second, third) {
        // No colon: plain local name.
        (first, None, None) => {
            if local_part_valid(first) {
                Ok((None, first.to_string()))
            } else {
                Err(5.0)
            }
        }
        // One colon: prefix:local. An empty prefix is a namespace error.
        (prefix, Some(local), None) => {
            if prefix.is_empty() {
                return Err(14.0);
            }
            if !local_part_valid(prefix) || !local_part_valid(local) {
                return Err(5.0);
            }
            Ok((Some(prefix.to_string()), local.to_string()))
        }
        // More than one colon.
        _ => Err(5.0),
    }
}

/// Applies the DOM namespace rules for `createElementNS`/`createAttributeNS`.
///
/// Returns `Err(code)` with `14` (NAMESPACE_ERR) on violation.
fn validate_namespace(
    prefix: &Option<String>,
    _local_name: &str,
    namespace: &str,
) -> Result<(), f64> {
    let namespace_is_null_or_empty = namespace.is_empty();
    match prefix {
        Some(prefix) if namespace_is_null_or_empty => Err(14.0),
        Some(prefix) if prefix == "xml" && namespace != XML_NAMESPACE => Err(14.0),
        Some(prefix) if prefix == "xmlns" && namespace != XMLNS_NAMESPACE => Err(14.0),
        Some(prefix) if namespace == XMLNS_NAMESPACE && prefix != "xmlns" => Err(14.0),
        Some(_) => Ok(()),
        None if namespace == XMLNS_NAMESPACE => Err(14.0),
        None => Ok(()),
    }
}

const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";

/// Maps a legacy DOMException numeric code back to its error name.
fn exception_name_for_code(code: f64) -> &'static str {
    match code {
        5.0 => "InvalidCharacterError",
        14.0 => "NamespaceError",
        _ => "DOMException",
    }
}
