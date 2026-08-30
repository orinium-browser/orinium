use crate::engine::html::HtmlNodeType;
use crate::engine::js::common::{dom_node, is_callable, node_dom_id, with_host, with_host_mut};
use crate::engine::js::web_apis::dom::element::{
    accessor_property, make_document_fragment, make_element, make_text_node,
    read_only_accessor_property,
};
use crate::engine::tree::{NodeRef, TreeNode};
use pixi_byte::value::jsobject::{JSObject, Property};
use pixi_byte::vm::VM;
use pixi_byte::{JSResult, JSValue};
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
        document.define_property(
            "documentElement".to_string(),
            read_only_accessor_property(get_document_element),
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
            "createTextNode".to_string(),
            JSValue::from_native_function(create_text_node),
        );
        document.set(
            "createDocumentFragment".to_string(),
            JSValue::from_native_function(create_document_fragment),
        );
        document.set(
            "addEventListener".to_string(),
            JSValue::from_native_function(add_document_event_listener),
        );
        document.set(
            "removeEventListener".to_string(),
            JSValue::from_native_function(remove_document_event_listener),
        );
    }
    let _ = with_host_mut(engine.vm(), |host| {
        host.document = Some(Rc::clone(&document_obj));
    });
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
        return Ok(JSValue::null());
    };
    let tag_name = tag_name.trim().to_ascii_lowercase();
    if tag_name.is_empty() {
        return Ok(JSValue::null());
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
        return Ok(JSValue::null());
    };
    let tag_name = qualified_name.trim().to_ascii_lowercase();
    if tag_name.is_empty() {
        return Ok(JSValue::null());
    }

    let node = TreeNode::new(HtmlNodeType::Element {
        tag_name,
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
            HtmlNodeType::DocumentFragment => NodeKind::Fragment,
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
    Fragment,
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
            NodeKind::Fragment => make_document_fragment(dom_id),
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
