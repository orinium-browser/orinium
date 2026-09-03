use crate::engine::html::{DomTree, HtmlNodeType, Parser as HtmlParser};
use crate::engine::js::common::{
    dom_node, is_callable, mark_dom_dirty, node_dom_id, with_host, with_host_mut,
};
use crate::engine::js::web_apis::dom::dom_exception::throw_dom_exception;
use crate::engine::js::web_apis::dom::element::{
    accessor_property, define_node_constants, make_comment_node, make_doctype_node,
    make_document_fragment, make_element, make_processing_instruction_node, make_text_node,
    read_only_accessor_property,
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
            "addEventListener".to_string(),
            JSValue::from_native_function(add_document_event_listener),
        );
        document.set(
            "removeEventListener".to_string(),
            JSValue::from_native_function(remove_document_event_listener),
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

    // DOMException constructor
    engine.global_mut().borrow_mut().set(
        "DOMException".to_string(),
        JSValue::from_object(super::dom_exception::make_dom_exception_constructor()),
    );
    // Link DOMException.prototype -> Error.prototype so instanceof checks work.
    // We use Object.setPrototypeOf via eval since pixi_byte doesn't expose
    // the prototype chain through JSObject APIs.
    let _ = engine.eval("Object.setPrototypeOf(DOMException.prototype, Error.prototype)");

    // Node constants
    let mut node_obj = JSObject::new();
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
    engine.global_mut().borrow_mut().set(
        "Node".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(node_obj))),
    );
}

fn html_iframe_element_has_instance(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = args.get(1).and_then(|value| dom_node(vm, value)) else {
        return Ok(JSValue::from_bool(false));
    };
    let is_iframe = node.borrow().value.tag_name() == Some("iframe");
    Ok(JSValue::from_bool(is_iframe))
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

fn create_element(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
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
        let obj = match kind {
            NodeKind::Element { tag_name, id } => make_element(
                tag_name,
                id,
                dom_id,
                Rc::clone(&host.element_prototype),
                Rc::clone(&host.element_constructor),
            ),
            NodeKind::Text => make_text_node(dom_id),
            NodeKind::Comment => make_comment_node(dom_id),
            NodeKind::ProcessingInstruction => make_processing_instruction_node(dom_id),
            NodeKind::Fragment => make_document_fragment(dom_id),
            NodeKind::Doctype { name } => make_doctype_node(dom_id, name),
        };
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
pub(crate) fn make_iframe_document(
    host_dom_id: u64,
    _src: &str,
    body_has_p: bool,
) -> Rc<RefCell<IframeDocument>> {
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
    if body_has_p {
        let p = TreeNode::new(HtmlNodeType::Element {
            tag_name: "p".to_string(),
            attributes: Vec::new(),
        });
        TreeNode::add_child(&body, p);
    }
    let tree = Rc::new(DomTree::from_root(document));

    let document_obj = build_iframe_document_object(host_dom_id, &tree);

    Rc::new(RefCell::new(IframeDocument {
        tree,
        document: document_obj,
        document_element: html,
    }))
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
        "createRange".to_string(),
        JSValue::from_native_function(create_range_stub),
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
