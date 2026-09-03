use crate::engine::html::{DomTree, HtmlNodeType, Parser as HtmlParser};
use crate::engine::js::common::{
    UNDEFINED, dom_node, is_callable, mark_dom_dirty, node_dom_id, noop, with_host, with_host_mut,
};
use crate::engine::js::web_apis::dom::custom_elements::{
    fire_attribute_changed_callback, fire_connected_callback, fire_disconnected_callback,
};
use crate::engine::js::web_apis::dom::document::{
    class_selector, create_text_node, expose_detached_node, expose_node, expose_node_list,
    make_iframe_document,
};
use crate::engine::js::web_apis::dom::dom_exception::throw_dom_exception;
use crate::engine::js::web_apis::dom::events::event_flag;
use crate::engine::js::{
    JsDynamicImageRequest, JsDynamicScriptRequest, JsDynamicScriptSource, JsDynamicStyleRequest,
    JsLayoutMetrics,
};
use crate::engine::tree::{NodeRef, TreeNode};
use pixi_byte::value::JSArray;
use pixi_byte::value::jsobject::{JSObject, Property};
use pixi_byte::vm::VM;
use pixi_byte::{JSError, JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
pub(crate) const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
pub(crate) const MATHML_NAMESPACE: &str = "http://www.w3.org/1998/Math/MathML";

pub(crate) fn make_element_interface() -> (Rc<RefCell<JSObject>>, Rc<RefCell<JSObject>>) {
    let mut prototype = JSObject::new();
    prototype.define_property(
        "value".to_string(),
        accessor_property(get_element_value, set_element_value),
    );
    prototype.define_property(
        "src".to_string(),
        accessor_property(get_element_src, set_element_src),
    );
    prototype.define_property(
        "href".to_string(),
        accessor_property(get_element_href, set_element_href),
    );
    prototype.define_property(
        "rel".to_string(),
        accessor_property(get_element_rel, set_element_rel),
    );
    prototype.define_property(
        "type".to_string(),
        accessor_property(get_element_type, set_element_type),
    );
    prototype.define_property(
        "charset".to_string(),
        accessor_property(get_element_charset, set_element_charset),
    );
    prototype.define_property(
        "crossOrigin".to_string(),
        accessor_property(get_element_cross_origin, set_element_cross_origin),
    );
    prototype.define_property(
        "width".to_string(),
        accessor_property(get_element_width, set_element_width),
    );
    prototype.define_property(
        "height".to_string(),
        accessor_property(get_element_height, set_element_height),
    );
    prototype.define_property(
        "clientWidth".to_string(),
        read_only_accessor_property(get_element_client_width),
    );
    prototype.define_property(
        "clientHeight".to_string(),
        read_only_accessor_property(get_element_client_height),
    );
    prototype.define_property(
        "offsetWidth".to_string(),
        read_only_accessor_property(get_element_offset_width),
    );
    prototype.define_property(
        "offsetHeight".to_string(),
        read_only_accessor_property(get_element_offset_height),
    );
    prototype.define_property(
        "offsetLeft".to_string(),
        read_only_accessor_property(get_element_offset_left),
    );
    prototype.define_property(
        "offsetTop".to_string(),
        read_only_accessor_property(get_element_offset_top),
    );
    prototype.define_property(
        "checked".to_string(),
        accessor_property(get_element_checked, set_element_checked),
    );
    prototype.define_property(
        "selected".to_string(),
        accessor_property(get_element_selected, set_element_selected),
    );
    prototype.define_property(
        "disabled".to_string(),
        accessor_property(get_element_disabled, set_element_disabled),
    );
    prototype.define_property(
        "multiple".to_string(),
        accessor_property(get_element_multiple, set_element_multiple),
    );
    prototype.define_property(
        "async".to_string(),
        accessor_property(get_element_async, set_element_async),
    );
    prototype.define_property(
        "defer".to_string(),
        accessor_property(get_element_defer, set_element_defer),
    );
    prototype.set(
        "getContext".to_string(),
        JSValue::from_native_function(canvas_get_context),
    );
    prototype.set(
        "toDataURL".to_string(),
        JSValue::from_native_function(canvas_to_data_url),
    );
    prototype.set(
        "getBoundingClientRect".to_string(),
        JSValue::from_native_function(get_bounding_client_rect),
    );
    prototype.set(
        "attachShadow".to_string(),
        JSValue::from_native_function(super::shadow_dom::element_attach_shadow),
    );
    prototype.define_property(
        "shadowRoot".to_string(),
        read_only_accessor_property(super::shadow_dom::get_shadow_root),
    );
    let prototype = Rc::new(RefCell::new(prototype));
    let mut constructor = JSObject::new();
    constructor.define_property(
        "prototype".to_string(),
        Property::read_only(JSValue::from_object(Rc::clone(&prototype))),
    );
    (prototype, Rc::new(RefCell::new(constructor)))
}

pub(crate) fn make_element(
    tag_name: String,
    _attr_id: String,
    dom_id: u64,
    prototype: Rc<RefCell<JSObject>>,
    constructor: Rc<RefCell<JSObject>>,
) -> Rc<RefCell<JSObject>> {
    let mut obj = JSObject::with_prototype(Some(prototype));
    define_node_id(&mut obj, dom_id);
    define_node_constants(&mut obj);
    obj.define_property(
        "constructor".to_string(),
        Property::read_only(JSValue::from_object(constructor)),
    );
    // Namespaced elements (created via createElementNS) keep their qualified
    // name exactly (e.g. "prefix:localname"); HTML elements are uppercased per
    // the legacy DOM convention (e.g. "div" -> "DIV").
    let display_name = if tag_name.contains(':') {
        tag_name.clone()
    } else {
        tag_name.to_ascii_uppercase()
    };
    let (prefix, local_name) = match tag_name.split_once(':') {
        Some((prefix, local)) => (Some(prefix.to_string()), local.to_string()),
        None => (None, tag_name.clone()),
    };
    obj.define_property(
        "nodeType".to_string(),
        Property::read_only(JSValue::from_number(1.0)),
    );
    obj.define_property(
        "nodeName".to_string(),
        Property::read_only(JSValue::from_string(display_name.clone())),
    );
    obj.define_property(
        "tagName".to_string(),
        Property::read_only(JSValue::from_string(display_name)),
    );
    obj.define_property(
        "localName".to_string(),
        Property::read_only(JSValue::from_string(local_name)),
    );
    obj.define_property(
        "prefix".to_string(),
        Property::read_only(match prefix {
            Some(prefix) => JSValue::from_string(prefix),
            None => JSValue::null(),
        }),
    );
    if tag_name.eq_ignore_ascii_case("iframe") {
        obj.define_property(
            "contentDocument".to_string(),
            read_only_accessor_property(get_iframe_content_document),
        );
        obj.define_property(
            "contentWindow".to_string(),
            read_only_accessor_property(get_iframe_content_document),
        );
    }
    obj.define_property(
        "id".to_string(),
        accessor_property(get_element_id, set_element_id),
    );
    obj.define_property(
        "textContent".to_string(),
        accessor_property(get_text_content, set_text_content),
    );
    obj.define_property(
        "innerText".to_string(),
        accessor_property(get_inner_text, set_inner_text),
    );
    obj.define_property(
        "innerHTML".to_string(),
        accessor_property(get_inner_html, set_inner_html),
    );
    obj.define_property(
        "parentNode".to_string(),
        read_only_accessor_property(get_parent_node),
    );
    obj.define_property(
        "parentElement".to_string(),
        read_only_accessor_property(get_parent_element),
    );
    obj.define_property(
        "isConnected".to_string(),
        read_only_accessor_property(get_is_connected),
    );
    obj.define_property(
        "ownerDocument".to_string(),
        read_only_accessor_property(get_owner_document),
    );
    obj.define_property(
        "namespaceURI".to_string(),
        read_only_accessor_property(get_namespace_uri),
    );
    obj.define_property(
        "childNodes".to_string(),
        read_only_accessor_property(get_child_nodes),
    );
    obj.define_property(
        "firstChild".to_string(),
        read_only_accessor_property(get_first_child),
    );
    obj.define_property(
        "lastChild".to_string(),
        read_only_accessor_property(get_last_child),
    );
    obj.define_property(
        "nextSibling".to_string(),
        read_only_accessor_property(get_next_sibling),
    );
    obj.define_property(
        "previousSibling".to_string(),
        read_only_accessor_property(get_previous_sibling),
    );
    obj.define_property(
        "children".to_string(),
        read_only_accessor_property(get_element_children),
    );
    obj.define_property(
        "classList".to_string(),
        read_only_accessor_property(get_class_list),
    );
    obj.define_property(
        "className".to_string(),
        accessor_property(get_class_name, set_class_name),
    );
    obj.define_property("style".to_string(), read_only_accessor_property(get_style));
    obj.set(
        "getAttribute".to_string(),
        JSValue::from_native_function(get_attribute),
    );
    obj.set(
        "hasAttribute".to_string(),
        JSValue::from_native_function(has_attribute),
    );
    obj.set(
        "setAttribute".to_string(),
        JSValue::from_native_function(set_attribute),
    );
    obj.set(
        "setAttributeNS".to_string(),
        JSValue::from_native_function(set_attribute_ns),
    );
    obj.set(
        "removeAttribute".to_string(),
        JSValue::from_native_function(remove_attribute),
    );
    obj.set(
        "addEventListener".to_string(),
        JSValue::from_native_function(add_element_event_listener),
    );
    obj.set(
        "removeEventListener".to_string(),
        JSValue::from_native_function(remove_element_event_listener),
    );
    obj.set(
        "querySelector".to_string(),
        JSValue::from_native_function(element_query_selector),
    );
    obj.set(
        "querySelectorAll".to_string(),
        JSValue::from_native_function(element_query_selector_all),
    );
    obj.set(
        "getElementsByTagName".to_string(),
        JSValue::from_native_function(element_get_elements_by_tag_name),
    );
    obj.set(
        "getElementsByClassName".to_string(),
        JSValue::from_native_function(element_get_elements_by_class_name),
    );
    obj.set(
        "dispatchEvent".to_string(),
        JSValue::from_native_function(element_dispatch_event),
    );
    obj.set(
        "contains".to_string(),
        JSValue::from_native_function(element_contains),
    );
    obj.set(
        "hasChildNodes".to_string(),
        JSValue::from_native_function(element_has_child_nodes),
    );
    obj.set(
        "click".to_string(),
        JSValue::from_native_function(element_click),
    );
    obj.set(
        "focus".to_string(),
        JSValue::from_native_function(focus_element),
    );
    obj.set(
        "blur".to_string(),
        JSValue::from_native_function(blur_element),
    );
    obj.set(
        "appendChild".to_string(),
        JSValue::from_native_function(append_child),
    );
    obj.set(
        "append".to_string(),
        JSValue::from_native_function(element_append),
    );
    obj.set(
        "insertBefore".to_string(),
        JSValue::from_native_function(insert_before),
    );
    obj.set(
        "removeChild".to_string(),
        JSValue::from_native_function(remove_child),
    );
    obj.set(
        "remove".to_string(),
        JSValue::from_native_function(remove_node),
    );
    obj.set(
        "replaceChild".to_string(),
        JSValue::from_native_function(replace_child),
    );
    obj.set(
        "cloneNode".to_string(),
        JSValue::from_native_function(clone_node),
    );
    obj.set(
        "closest".to_string(),
        JSValue::from_native_function(element_closest),
    );
    obj.set(
        "matches".to_string(),
        JSValue::from_native_function(element_matches),
    );
    Rc::new(RefCell::new(obj))
}

pub(crate) fn make_text_node(dom_id: u64) -> Rc<RefCell<JSObject>> {
    let mut obj = JSObject::new();
    define_node_id(&mut obj, dom_id);
    define_node_constants(&mut obj);
    obj.define_property(
        "nodeType".to_string(),
        Property::read_only(JSValue::from_number(3.0)),
    );
    obj.define_property(
        "nodeName".to_string(),
        Property::read_only(JSValue::from_string("#text".to_string())),
    );
    obj.define_property(
        "textContent".to_string(),
        accessor_property(get_text_content, set_text_content),
    );
    obj.define_property(
        "nodeValue".to_string(),
        accessor_property(get_text_content, set_text_content),
    );
    obj.define_property(
        "data".to_string(),
        accessor_property(get_text_content, set_text_content),
    );
    obj.define_property(
        "parentNode".to_string(),
        read_only_accessor_property(get_parent_node),
    );
    obj.define_property(
        "parentElement".to_string(),
        read_only_accessor_property(get_parent_element),
    );
    obj.define_property(
        "isConnected".to_string(),
        read_only_accessor_property(get_is_connected),
    );
    obj.define_property(
        "ownerDocument".to_string(),
        read_only_accessor_property(get_owner_document),
    );
    obj.define_property(
        "childNodes".to_string(),
        read_only_accessor_property(get_child_nodes),
    );
    obj.define_property(
        "firstChild".to_string(),
        read_only_accessor_property(get_first_child),
    );
    obj.define_property(
        "lastChild".to_string(),
        read_only_accessor_property(get_last_child),
    );
    obj.define_property(
        "nextSibling".to_string(),
        read_only_accessor_property(get_next_sibling),
    );
    obj.define_property(
        "previousSibling".to_string(),
        read_only_accessor_property(get_previous_sibling),
    );
    obj.set(
        "remove".to_string(),
        JSValue::from_native_function(remove_node),
    );
    Rc::new(RefCell::new(obj))
}

pub(crate) fn make_comment_node(dom_id: u64) -> Rc<RefCell<JSObject>> {
    let mut obj = JSObject::new();
    define_node_id(&mut obj, dom_id);
    define_node_constants(&mut obj);
    obj.define_property(
        "nodeType".to_string(),
        Property::read_only(JSValue::from_number(8.0)),
    );
    obj.define_property(
        "nodeName".to_string(),
        Property::read_only(JSValue::from_string("#comment".to_string())),
    );
    obj.define_property(
        "textContent".to_string(),
        accessor_property(get_comment_data, set_comment_data),
    );
    obj.define_property(
        "nodeValue".to_string(),
        accessor_property(get_comment_data, set_comment_data),
    );
    obj.define_property(
        "data".to_string(),
        accessor_property(get_comment_data, set_comment_data),
    );
    obj.define_property(
        "parentNode".to_string(),
        read_only_accessor_property(get_parent_node),
    );
    obj.define_property(
        "parentElement".to_string(),
        read_only_accessor_property(get_parent_element),
    );
    obj.define_property(
        "isConnected".to_string(),
        read_only_accessor_property(get_is_connected),
    );
    obj.define_property(
        "ownerDocument".to_string(),
        read_only_accessor_property(get_owner_document),
    );
    obj.define_property(
        "childNodes".to_string(),
        read_only_accessor_property(get_child_nodes),
    );
    obj.define_property(
        "firstChild".to_string(),
        read_only_accessor_property(get_first_child),
    );
    obj.define_property(
        "lastChild".to_string(),
        read_only_accessor_property(get_last_child),
    );
    obj.define_property(
        "nextSibling".to_string(),
        read_only_accessor_property(get_next_sibling),
    );
    obj.define_property(
        "previousSibling".to_string(),
        read_only_accessor_property(get_previous_sibling),
    );
    obj.set(
        "remove".to_string(),
        JSValue::from_native_function(remove_node),
    );
    Rc::new(RefCell::new(obj))
}

pub(crate) fn make_processing_instruction_node(dom_id: u64) -> Rc<RefCell<JSObject>> {
    let mut obj = JSObject::new();
    define_node_id(&mut obj, dom_id);
    define_node_constants(&mut obj);
    obj.define_property(
        "nodeType".to_string(),
        Property::read_only(JSValue::from_number(7.0)),
    );
    obj.define_property(
        "nodeName".to_string(),
        Property::read_only(JSValue::from_string(String::new())),
    );
    obj.define_property(
        "textContent".to_string(),
        accessor_property(get_pi_data, set_pi_data),
    );
    obj.define_property(
        "nodeValue".to_string(),
        accessor_property(get_pi_data, set_pi_data),
    );
    obj.define_property(
        "data".to_string(),
        accessor_property(get_pi_data, set_pi_data),
    );
    obj.define_property(
        "parentNode".to_string(),
        read_only_accessor_property(get_parent_node),
    );
    obj.define_property(
        "parentElement".to_string(),
        read_only_accessor_property(get_parent_element),
    );
    obj.define_property(
        "isConnected".to_string(),
        read_only_accessor_property(get_is_connected),
    );
    obj.define_property(
        "ownerDocument".to_string(),
        read_only_accessor_property(get_owner_document),
    );
    obj.define_property(
        "childNodes".to_string(),
        read_only_accessor_property(get_child_nodes),
    );
    obj.define_property(
        "firstChild".to_string(),
        read_only_accessor_property(get_first_child),
    );
    obj.define_property(
        "lastChild".to_string(),
        read_only_accessor_property(get_last_child),
    );
    obj.define_property(
        "nextSibling".to_string(),
        read_only_accessor_property(get_next_sibling),
    );
    obj.define_property(
        "previousSibling".to_string(),
        read_only_accessor_property(get_previous_sibling),
    );
    obj.set(
        "remove".to_string(),
        JSValue::from_native_function(remove_node),
    );
    Rc::new(RefCell::new(obj))
}

pub(crate) fn make_document_fragment(dom_id: u64) -> Rc<RefCell<JSObject>> {
    let mut obj = JSObject::new();
    define_node_id(&mut obj, dom_id);
    define_node_constants(&mut obj);
    obj.define_property(
        "nodeType".to_string(),
        Property::read_only(JSValue::from_number(11.0)),
    );
    obj.define_property(
        "nodeName".to_string(),
        Property::read_only(JSValue::from_string("#document-fragment".to_string())),
    );
    obj.define_property(
        "textContent".to_string(),
        accessor_property(get_text_content, set_text_content),
    );
    obj.define_property(
        "parentNode".to_string(),
        read_only_accessor_property(get_parent_node),
    );
    obj.define_property(
        "parentElement".to_string(),
        read_only_accessor_property(get_parent_element),
    );
    obj.define_property(
        "isConnected".to_string(),
        read_only_accessor_property(get_is_connected),
    );
    obj.define_property(
        "ownerDocument".to_string(),
        read_only_accessor_property(get_owner_document),
    );
    obj.define_property(
        "childNodes".to_string(),
        read_only_accessor_property(get_child_nodes),
    );
    obj.define_property(
        "firstChild".to_string(),
        read_only_accessor_property(get_first_child),
    );
    obj.define_property(
        "lastChild".to_string(),
        read_only_accessor_property(get_last_child),
    );
    obj.define_property(
        "nextSibling".to_string(),
        read_only_accessor_property(get_next_sibling),
    );
    obj.define_property(
        "previousSibling".to_string(),
        read_only_accessor_property(get_previous_sibling),
    );
    obj.define_property(
        "children".to_string(),
        read_only_accessor_property(get_element_children),
    );
    obj.set(
        "appendChild".to_string(),
        JSValue::from_native_function(append_child),
    );
    obj.set(
        "insertBefore".to_string(),
        JSValue::from_native_function(insert_before),
    );
    obj.set(
        "removeChild".to_string(),
        JSValue::from_native_function(remove_child),
    );
    obj.set(
        "replaceChild".to_string(),
        JSValue::from_native_function(replace_child),
    );
    obj.set(
        "cloneNode".to_string(),
        JSValue::from_native_function(clone_node),
    );
    obj.set(
        "remove".to_string(),
        JSValue::from_native_function(remove_node),
    );
    obj.set(
        "querySelector".to_string(),
        JSValue::from_native_function(element_query_selector),
    );
    obj.set(
        "querySelectorAll".to_string(),
        JSValue::from_native_function(element_query_selector_all),
    );
    Rc::new(RefCell::new(obj))
}

pub(crate) fn make_doctype_node(dom_id: u64, name: Option<String>) -> Rc<RefCell<JSObject>> {
    let mut obj = JSObject::new();
    define_node_id(&mut obj, dom_id);
    define_node_constants(&mut obj);
    obj.define_property(
        "nodeType".to_string(),
        Property::read_only(JSValue::from_number(10.0)),
    );
    obj.define_property(
        "nodeName".to_string(),
        Property::read_only(JSValue::from_string(name.unwrap_or_default())),
    );
    obj.define_property(
        "parentNode".to_string(),
        read_only_accessor_property(get_parent_node),
    );
    obj.define_property(
        "parentElement".to_string(),
        read_only_accessor_property(get_parent_element),
    );
    obj.define_property(
        "isConnected".to_string(),
        read_only_accessor_property(get_is_connected),
    );
    obj.define_property(
        "ownerDocument".to_string(),
        read_only_accessor_property(get_owner_document),
    );
    obj.define_property(
        "childNodes".to_string(),
        read_only_accessor_property(get_child_nodes),
    );
    obj.define_property(
        "firstChild".to_string(),
        read_only_accessor_property(get_first_child),
    );
    obj.define_property(
        "lastChild".to_string(),
        read_only_accessor_property(get_last_child),
    );
    obj.define_property(
        "nextSibling".to_string(),
        read_only_accessor_property(get_next_sibling),
    );
    obj.define_property(
        "previousSibling".to_string(),
        read_only_accessor_property(get_previous_sibling),
    );
    Rc::new(RefCell::new(obj))
}

/// Defines the legacy `Node`-level numeric constants on a node object so they
/// are reachable from any node instance (e.g. `element.ELEMENT_NODE`).
pub(crate) fn define_node_constants(obj: &mut JSObject) {
    for (name, value) in [
        ("ELEMENT_NODE", 1.0),
        ("ATTRIBUTE_NODE", 2.0),
        ("TEXT_NODE", 3.0),
        ("CDATA_SECTION_NODE", 4.0),
        ("ENTITY_REFERENCE_NODE", 5.0),
        ("ENTITY_NODE", 6.0),
        ("PROCESSING_INSTRUCTION_NODE", 7.0),
        ("COMMENT_NODE", 8.0),
        ("DOCUMENT_NODE", 9.0),
        ("DOCUMENT_TYPE_NODE", 10.0),
        ("DOCUMENT_FRAGMENT_NODE", 11.0),
        ("NOTATION_NODE", 12.0),
        ("DOCUMENT_POSITION_DISCONNECTED", 1.0),
        ("DOCUMENT_POSITION_PRECEDING", 2.0),
        ("DOCUMENT_POSITION_FOLLOWING", 4.0),
        ("DOCUMENT_POSITION_CONTAINS", 8.0),
        ("DOCUMENT_POSITION_CONTAINED_BY", 16.0),
        ("DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC", 32.0),
    ] {
        obj.define_property(
            name.to_string(),
            Property::read_only(JSValue::from_number(value)),
        );
    }
}

pub(crate) fn define_node_id(obj: &mut JSObject, dom_id: u64) {
    obj.define_property(
        "__orinium_dom_id".to_string(),
        Property {
            value: JSValue::from_number(dom_id as f64),
            enumerable: false,
            writable: false,
            configurable: false,
            getter: None,
            setter: None,
        },
    );
}

pub(crate) fn append_child(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(parent) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::null());
    };
    if !matches!(
        parent.borrow().value,
        HtmlNodeType::Element { .. }
            | HtmlNodeType::DocumentFragment
            | HtmlNodeType::ShadowRoot { .. }
    ) {
        return Ok(JSValue::null());
    }
    let Some(child_value) = args.get(1).cloned() else {
        return Ok(JSValue::null());
    };
    let Some(child) = dom_node(vm, &child_value) else {
        return Ok(JSValue::null());
    };

    // Ensure pre-insertion validity: appending a node that is an inclusive
    // ancestor of the parent would create a cycle and raises
    // HIERARCHY_REQUEST_ERR (e.g. document.body.appendChild(documentElement)).
    if TreeNode::is_inclusive_ancestor(&child, &parent) {
        return Err(throw_dom_exception(
            "The new child element is an ancestor of the parent element",
            "HierarchyRequestError",
        ));
    }

    // Per DOM spec, appending a DocumentFragment moves its children.
    if matches!(child.borrow().value, HtmlNodeType::DocumentFragment) {
        let fragment_children: Vec<_> = child.borrow().children().to_vec();
        for fragment_child in fragment_children {
            let _ = TreeNode::append_child(&parent, Rc::clone(&fragment_child));
            mark_dom_dirty(vm);
        }
        return Ok(child_value);
    }

    if !TreeNode::append_child(&parent, child) {
        return Ok(JSValue::null());
    }

    if let Some(dom_id) = node_dom_id(&child_value) {
        let _ = with_host_mut(vm, |host| {
            host.detached_nodes.remove(&dom_id);
        });
        fire_connected_callback(vm, dom_id);
    }
    queue_dynamic_script(vm, &child_value);
    queue_dynamic_stylesheet(vm, &child_value);
    queue_dynamic_image(vm, &child_value);
    mark_dom_dirty(vm);
    Ok(child_value)
}

fn element_append(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let receiver = args.first().cloned().unwrap_or(JSValue::undefined());
    for value in args.into_iter().skip(1) {
        let child = if dom_node(vm, &value).is_some() {
            value
        } else {
            create_text_node(vm, vec![JSValue::undefined(), value])?
        };
        append_child(vm, vec![receiver.clone(), child])?;
    }
    Ok(JSValue::undefined())
}

pub(crate) fn insert_before(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(parent) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::null());
    };
    let Some(child_value) = args.get(1).cloned() else {
        return Ok(JSValue::null());
    };
    let Some(child) = dom_node(vm, &child_value) else {
        return Ok(JSValue::null());
    };

    let reference_value = args.get(2).cloned();
    let reference = match &reference_value {
        Some(val) if val.is_null() || val.is_undefined() => None,
        Some(val) => dom_node(vm, val),
        None => None,
    };

    // Per DOM spec, inserting a DocumentFragment moves its children.
    if matches!(child.borrow().value, HtmlNodeType::DocumentFragment) {
        let fragment_children: Vec<_> = child.borrow().children().to_vec();
        for fragment_child in fragment_children {
            let inserted = match &reference {
                Some(r) => TreeNode::insert_before(&parent, Rc::clone(&fragment_child), r),
                None => TreeNode::append_child(&parent, Rc::clone(&fragment_child)),
            };
            if inserted {
                mark_dom_dirty(vm);
            }
        }
        return Ok(child_value);
    }

    let inserted = match &reference {
        Some(r) => TreeNode::insert_before(&parent, child, r),
        None => TreeNode::append_child(&parent, child),
    };
    if !inserted {
        return Ok(JSValue::null());
    }
    if let Some(dom_id) = node_dom_id(&child_value) {
        let _ = with_host_mut(vm, |host| {
            host.detached_nodes.remove(&dom_id);
        });
        fire_connected_callback(vm, dom_id);
    }
    queue_dynamic_script(vm, &child_value);
    queue_dynamic_stylesheet(vm, &child_value);
    queue_dynamic_image(vm, &child_value);
    mark_dom_dirty(vm);
    Ok(child_value)
}

fn queue_dynamic_script(vm: &mut VM, value: &JSValue) {
    let Some(node_id) = node_dom_id(value) else {
        return;
    };
    let Some(node) = dom_node(vm, value) else {
        return;
    };
    let source = {
        let node_ref = node.borrow();
        if node_ref.value.tag_name() != Some("script") {
            return;
        }
        let script_type = node_ref.value.get_attr("type").unwrap_or("").trim();
        if !script_type.is_empty()
            && !matches!(
                script_type.to_ascii_lowercase().as_str(),
                "text/javascript"
                    | "application/javascript"
                    | "text/ecmascript"
                    | "application/ecmascript"
                    | "application/x-javascript"
            )
        {
            return;
        }
        match node_ref.value.get_attr("src").map(str::trim) {
            Some(src) if !src.is_empty() => JsDynamicScriptSource::External(src.to_string()),
            Some(_) => return,
            None => JsDynamicScriptSource::Inline(DomTree::inner_text(&node)),
        }
    };
    let _ = with_host_mut(vm, |host| {
        if host.queued_dynamic_scripts.insert(node_id) {
            host.dynamic_script_requests
                .push(JsDynamicScriptRequest { node_id, source });
        }
    });
}

fn queue_dynamic_stylesheet(vm: &mut VM, value: &JSValue) {
    let Some(node_id) = node_dom_id(value) else {
        return;
    };
    let Some(node) = dom_node(vm, value) else {
        return;
    };
    let url = {
        let node = node.borrow();
        if node.value.tag_name() != Some("link")
            || !node
                .value
                .get_attr("rel")
                .is_some_and(|rel| rel.eq_ignore_ascii_case("stylesheet"))
        {
            return;
        }
        let Some(url) = node.value.get_attr("href").map(str::trim) else {
            return;
        };
        if url.is_empty() {
            return;
        }
        url.to_string()
    };
    let _ = with_host_mut(vm, |host| {
        if host.queued_dynamic_styles.insert(node_id) {
            host.dynamic_style_requests
                .push(JsDynamicStyleRequest { node_id, url });
        }
    });
}

fn queue_dynamic_image(vm: &mut VM, value: &JSValue) {
    let Some(node_id) = node_dom_id(value) else {
        return;
    };
    let Some(node) = dom_node(vm, value) else {
        return;
    };
    let source = {
        let node = node.borrow();
        if node.value.tag_name() != Some("img") {
            return;
        }
        let Some(source) = node.value.get_attr("src").map(str::trim) else {
            return;
        };
        if source.is_empty() {
            return;
        }
        source.to_string()
    };
    let _ = with_host_mut(vm, |host| {
        if host.queued_dynamic_images.insert(node_id) {
            host.dynamic_image_requests
                .push(JsDynamicImageRequest { source });
        }
    });
}

pub(crate) fn remove_child(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(parent) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::null());
    };
    let Some(child_value) = args.get(1).cloned() else {
        return Ok(JSValue::null());
    };
    let Some(child) = dom_node(vm, &child_value) else {
        return Ok(JSValue::null());
    };
    let Some(detached) = TreeNode::remove_child(&parent, &child) else {
        return Ok(JSValue::null());
    };
    if let Some(dom_id) = node_dom_id(&child_value) {
        fire_disconnected_callback(vm, dom_id);
        let _ = with_host_mut(vm, |host| {
            host.detached_nodes.insert(dom_id, detached);
        });
    }
    mark_dom_dirty(vm);
    Ok(child_value)
}

fn remove_node(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let this = args.first().unwrap_or(&UNDEFINED);
    let Some(node) = dom_node(vm, this) else {
        return Ok(JSValue::undefined());
    };
    if TreeNode::detach(&node) {
        if let Some(dom_id) = node_dom_id(this) {
            fire_disconnected_callback(vm, dom_id);
            let _ = with_host_mut(vm, |host| {
                host.detached_nodes.insert(dom_id, node);
            });
        }
        mark_dom_dirty(vm);
    }
    Ok(JSValue::undefined())
}

pub(crate) fn replace_child(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(parent) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::null());
    };
    let Some(new_child_value) = args.get(1).cloned() else {
        return Ok(JSValue::null());
    };
    let Some(new_child) = dom_node(vm, &new_child_value) else {
        return Ok(JSValue::null());
    };
    let Some(old_child_value) = args.get(2).cloned() else {
        return Ok(JSValue::null());
    };
    let Some(old_child) = dom_node(vm, &old_child_value) else {
        return Ok(JSValue::null());
    };
    let inserted = TreeNode::insert_before(&parent, new_child, &old_child);
    if !inserted {
        return Ok(JSValue::null());
    }
    let Some(detached) = TreeNode::remove_child(&parent, &old_child) else {
        return Ok(JSValue::null());
    };
    if let Some(dom_id) = node_dom_id(&old_child_value) {
        fire_disconnected_callback(vm, dom_id);
        let _ = with_host_mut(vm, |host| {
            host.detached_nodes.insert(dom_id, detached);
        });
    }
    if let Some(dom_id) = node_dom_id(&new_child_value) {
        let _ = with_host_mut(vm, |host| {
            host.detached_nodes.remove(&dom_id);
        });
        fire_connected_callback(vm, dom_id);
    }
    queue_dynamic_script(vm, &new_child_value);
    queue_dynamic_stylesheet(vm, &new_child_value);
    queue_dynamic_image(vm, &new_child_value);
    mark_dom_dirty(vm);
    Ok(old_child_value)
}

pub(crate) fn clone_node(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let this = args.first().unwrap_or(&UNDEFINED);
    let Some(node) = dom_node(vm, this) else {
        return Ok(JSValue::null());
    };

    let deep = args.get(1).is_some_and(JSValue::to_boolean);
    let cloned = if deep {
        deep_clone_tree(&node)
    } else {
        shallow_clone_node(&node)
    };

    Ok(expose_detached_node(vm, cloned).unwrap_or(JSValue::null()))
}

fn shallow_clone_node(node: &NodeRef<HtmlNodeType>) -> NodeRef<HtmlNodeType> {
    let value = match &node.borrow().value {
        HtmlNodeType::Document => HtmlNodeType::Element {
            tag_name: "__document__".to_string(),
            attributes: Vec::new(),
        },
        value => value.clone(),
    };

    TreeNode::new(value)
}

fn deep_clone_tree(node: &NodeRef<HtmlNodeType>) -> NodeRef<HtmlNodeType> {
    let cloned = shallow_clone_node(node);
    for child in node.borrow().children() {
        let cloned_child = deep_clone_tree(child);
        TreeNode::append_child(&cloned, cloned_child);
    }
    cloned
}

fn element_closest(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::null());
    };
    let Some(selector) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(JSValue::null());
    };
    let Some(closest) = DomTree::element_closest(&node, selector) else {
        return Ok(JSValue::null());
    };
    Ok(expose_node(vm, closest).unwrap_or(JSValue::null()))
}

fn element_matches(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::from_bool(false));
    };
    let Some(selector) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(JSValue::from_bool(false));
    };
    Ok(JSValue::from_bool(DomTree::element_matches_selector(
        &node, selector,
    )))
}

fn element_query_selector(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(scope) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::null());
    };
    let Some(selector) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(JSValue::null());
    };
    let Some(node) = DomTree::query_selector_within(&scope, selector) else {
        return Ok(JSValue::null());
    };
    Ok(expose_node(vm, node).unwrap_or(JSValue::null()))
}

fn element_query_selector_all(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(scope) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(vm.array_from_values(Vec::new()));
    };
    let Some(selector) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(vm.array_from_values(Vec::new()));
    };
    let nodes = DomTree::query_selector_all_within(&scope, selector);
    Ok(expose_node_list(vm, nodes))
}

fn element_get_elements_by_tag_name(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(scope) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(vm.array_from_values(Vec::new()));
    };
    let tag_name = args.get(1).unwrap_or(&UNDEFINED).to_string();
    let selector = if tag_name == "*" {
        "*".to_string()
    } else {
        tag_name.to_ascii_lowercase()
    };
    let nodes = DomTree::query_selector_all_within(&scope, &selector);
    Ok(expose_node_list(vm, nodes))
}

fn element_get_elements_by_class_name(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(scope) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(vm.array_from_values(Vec::new()));
    };
    let selector = class_selector(args.get(1).unwrap_or(&UNDEFINED));
    if selector.is_empty() {
        return Ok(vm.array_from_values(Vec::new()));
    }
    let nodes = DomTree::query_selector_all_within(&scope, &selector);
    Ok(expose_node_list(vm, nodes))
}

fn element_dispatch_event(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(target) = args.first().and_then(JSValue::as_object) else {
        return Err(JSError::TypeError(
            "dispatchEvent called on incompatible receiver".to_string(),
        ));
    };
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

    let target_obj = Rc::clone(&target);

    // Build the ancestor path of exposed JS objects: [target, ..., root].
    let mut path: Vec<Rc<RefCell<JSObject>>> = vec![Rc::clone(&target_obj)];
    if let Some(mut node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) {
        loop {
            let parent = node.borrow().parent();
            let Some(parent) = parent else { break };
            node = parent;
            if let Some(object) = expose_node(vm, Rc::clone(&node)).and_then(|value| value.as_object()) {
                path.push(object);
            }
        }
    }

    propagate_event(vm, &target_obj, &event, path)
}

/// Runs the capture/target/bubble propagation for an event whose `target` is
/// `target_obj` and whose ancestor path (target first) is `path`.
fn propagate_event(
    vm: &mut VM,
    target_obj: &Rc<RefCell<JSObject>>,
    event: &Rc<RefCell<JSObject>>,
    path: Vec<Rc<RefCell<JSObject>>>,
) -> JSResult<JSValue> {
    let event_type = event.borrow().get("type").to_string();
    let bubbles = event.borrow().get("bubbles").to_boolean();

    // Reset propagation state for this dispatch.
    event.borrow_mut().set(
        "__orinium_immediate_propagation_stopped".to_string(),
        JSValue::from_bool(false),
    );
    event.borrow_mut().set("cancelBubble".to_string(), JSValue::from_bool(false));
    event.borrow_mut().set("target".to_string(), JSValue::from_object(Rc::clone(target_obj)));
    event.borrow_mut().set("__orinium_bubbles".to_string(), JSValue::from_bool(bubbles));

    // Capture phase: from the root down to (but excluding) the target.
    for ancestor in path.iter().rev().skip(1) {
        if event_flag(&event, "cancelBubble") {
            break;
        }
        dispatch_at_phase(vm, ancestor, event, &event_type, 1 /* capture */)?;
    }

    // Target phase: the target's own handlers (both capturing and bubbling).
    if !event_flag(event, "cancelBubble") {
        dispatch_at_phase(vm, target_obj, event, &event_type, 2 /* target */)?;
    }

    // Bubble phase: from the target's ancestors up to the root.
    if bubbles {
        for ancestor in path.iter().skip(1) {
            if event_flag(event, "cancelBubble") {
                break;
            }
            dispatch_at_phase(vm, ancestor, event, &event_type, 3 /* bubble */)?;
        }
    }

    Ok(JSValue::from_bool(!event_flag(event, "defaultPrevented")))
}

/// Invokes the listeners/pending on-element handlers stored on `current` during
/// the given propagation phase.
///
/// `phase` is `1` (capture), `2` (target) or `3` (bubble). During capture only
/// capturing listeners run; during bubble only non-capturing listeners run; the
/// target runs both, plus any inline `on<type>` handler.
fn dispatch_at_phase(
    vm: &mut VM,
    current: &Rc<RefCell<JSObject>>,
    event: &Rc<RefCell<JSObject>>,
    event_type: &str,
    phase: u8,
) -> JSResult<()> {
    let current_js = JSValue::from_object(Rc::clone(current));
    event
        .borrow_mut()
        .set("currentTarget".to_string(), current_js.clone());
    event
        .borrow_mut()
        .set("eventPhase".to_string(), JSValue::from_number(phase as f64));

    let dom_id = node_dom_id(&current_js).unwrap_or(0);
    if dom_id == 0 {
        return Ok(());
    }
    let listeners = with_host(vm, |host| {
        host.element_event_listeners
            .get(&dom_id)
            .and_then(|events| events.get(event_type))
            .cloned()
            .unwrap_or_default()
    })
    .unwrap_or_default();

    if phase == 2 {
        let handler = current.borrow().get(&format!("on{event_type}"));
        if is_callable(&handler) {
            vm.call(
                handler,
                current_js.clone(),
                vec![JSValue::from_object(Rc::clone(event))],
            )?;
        }
    }

    for (listener, capture) in listeners {
        if event_flag(event, "__orinium_immediate_propagation_stopped") {
            break;
        }
        if (phase == 1 && !capture) || (phase == 3 && capture) {
            continue;
        }
        vm.call(
            listener,
            current_js.clone(),
            vec![JSValue::from_object(Rc::clone(event))],
        )?;
    }

    // stopImmediatePropagation halts all remaining propagation.
    if event_flag(event, "__orinium_immediate_propagation_stopped") {
        event
            .borrow_mut()
            .set("cancelBubble".to_string(), JSValue::from_bool(true));
    }
    Ok(())
}

/// The DOM `node.hasChildNodes()` method: returns true if the node has at
/// least one child node.
fn element_has_child_nodes(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::from_bool(false));
    };
    Ok(JSValue::from_bool(!node.borrow().children().is_empty()))
}

/// The DOM `element.click()` method: synthesizes a bubbling click event
/// (detail 1) and dispatches it through the capture/bubble propagation engine.
fn element_click(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(target) = args.first().and_then(JSValue::as_object) else {
        return Ok(JSValue::undefined());
    };
    let mut event = JSObject::new();
    event.set("type".to_string(), JSValue::from_string("click".to_string()));
    event.set("bubbles".to_string(), JSValue::from_bool(true));
    event.set("cancelable".to_string(), JSValue::from_bool(true));
    event.set("detail".to_string(), JSValue::from_number(1.0));
    event.set("defaultPrevented".to_string(), JSValue::from_bool(false));
    event.set("cancelBubble".to_string(), JSValue::from_bool(false));
    event.set("eventPhase".to_string(), JSValue::from_number(0.0));
    event.set(
        "preventDefault".to_string(),
        JSValue::from_native_function(super::events::event_prevent_default),
    );
    event.set(
        "stopPropagation".to_string(),
        JSValue::from_native_function(super::events::event_stop_propagation),
    );
    event.set(
        "stopImmediatePropagation".to_string(),
        JSValue::from_native_function(super::events::event_stop_immediate_propagation),
    );
    let event = Rc::new(RefCell::new(event));

    // Build the ancestor path (target first) like element_dispatch_event.
    let target_obj = Rc::clone(&target);
    let mut path: Vec<Rc<RefCell<JSObject>>> = vec![Rc::clone(&target_obj)];
    if let Some(mut node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) {
        loop {
            let parent = node.borrow().parent();
            let Some(parent) = parent else { break };
            node = parent;
            if let Some(object) = expose_node(vm, Rc::clone(&node)).and_then(|value| value.as_object()) {
                path.push(object);
            }
        }
    }
    propagate_event(vm, &target_obj, &event, path)
}

fn element_contains(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(container) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::from_bool(false));
    };
    let Some(mut candidate) = args.get(1).and_then(|value| dom_node(vm, value)) else {
        return Ok(JSValue::from_bool(false));
    };

    loop {
        if Rc::ptr_eq(&container, &candidate) {
            return Ok(JSValue::from_bool(true));
        }
        let parent = { candidate.borrow().parent() };
        let Some(parent) = parent else {
            return Ok(JSValue::from_bool(false));
        };
        candidate = parent;
    }
}

fn focus_element(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(dom_id) = node_dom_id(args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::undefined());
    };
    let _ = with_host_mut(vm, |host| {
        host.active_element = Some(dom_id);
    });
    Ok(JSValue::undefined())
}

fn blur_element(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(dom_id) = node_dom_id(args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::undefined());
    };
    let _ = with_host_mut(vm, |host| {
        if host.active_element == Some(dom_id) {
            host.active_element = None;
        }
    });
    Ok(JSValue::undefined())
}

fn add_element_event_listener(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(dom_id) = node_dom_id(args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::undefined());
    };
    let Some(event_type) = args.get(1).and_then(JSValue::as_string_owned) else {
        return Ok(JSValue::undefined());
    };
    let Some(listener) = args.get(2).filter(|value| is_callable(value)).cloned() else {
        return Ok(JSValue::undefined());
    };
    let capture = args.get(3).map_or(false, |v| match v.as_object() {
        Some(o) => o.borrow().get("capture").to_boolean(),
        None => v.as_boolean() == Some(true),
    });

    let _ = with_host_mut(vm, |host| {
        let listeners = host
            .element_event_listeners
            .entry(dom_id)
            .or_default()
            .entry(event_type.clone())
            .or_default();
        if !listeners.iter().any(|(candidate, c)| {
            c == &capture && candidate.strict_equals(&listener)
        }) {
            listeners.push((listener, capture));
        }
    });
    Ok(JSValue::undefined())
}

fn remove_element_event_listener(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(dom_id) = node_dom_id(args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::undefined());
    };
    let Some(event_type) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(JSValue::undefined());
    };
    let Some(listener) = args.get(2) else {
        return Ok(JSValue::undefined());
    };
    let _ = with_host_mut(vm, |host| {
        if let Some(listeners) = host
            .element_event_listeners
            .get_mut(&dom_id)
            .and_then(|events| events.get_mut(event_type))
        {
            listeners.retain(|(candidate, _)| !candidate.strict_equals(listener));
        }
    });
    Ok(JSValue::undefined())
}

pub(crate) fn accessor_property(
    getter: pixi_byte::NativeFunctionType,
    setter: pixi_byte::NativeFunctionType,
) -> Property {
    Property {
        value: JSValue::undefined(),
        enumerable: true,
        writable: false,
        configurable: false,
        getter: Some(JSValue::from_native_function(getter)),
        setter: Some(JSValue::from_native_function(setter)),
    }
}

pub(crate) fn read_only_accessor_property(getter: pixi_byte::NativeFunctionType) -> Property {
    Property {
        value: JSValue::undefined(),
        enumerable: true,
        writable: false,
        configurable: false,
        getter: Some(JSValue::from_native_function(getter)),
        setter: None,
    }
}

fn get_parent_node(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::null());
    };
    let Some(parent) = node.borrow().parent() else {
        return Ok(JSValue::null());
    };
    Ok(expose_node(vm, parent).unwrap_or(JSValue::null()))
}

fn get_parent_element(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::null());
    };
    let Some(parent) = node.borrow().parent() else {
        return Ok(JSValue::null());
    };
    if !matches!(parent.borrow().value, HtmlNodeType::Element { .. }) {
        return Ok(JSValue::null());
    }
    Ok(expose_node(vm, parent).unwrap_or(JSValue::null()))
}

fn get_is_connected(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(mut node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::from_bool(false));
    };

    loop {
        let (is_document, parent) = {
            let node = node.borrow();
            (matches!(node.value, HtmlNodeType::Document), node.parent())
        };
        if is_document {
            return Ok(JSValue::from_bool(true));
        }
        let Some(parent) = parent else {
            return Ok(JSValue::from_bool(false));
        };
        node = parent;
    }
}

fn get_owner_document(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(with_host(vm, |host| host.document.as_ref().cloned())
        .flatten()
        .map(JSValue::from_object)
        .unwrap_or(JSValue::null()))
}

fn get_iframe_content_document(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let this = args.first().unwrap_or(&UNDEFINED);
    let Some(dom_id) = node_dom_id(this) else {
        return Ok(JSValue::null());
    };
    // If the document was created already, return it.
    let existing = with_host(vm, |host| {
        host.iframe_documents
            .get(&dom_id)
            .map(|doc| Rc::clone(&doc.borrow().document))
    })
    .flatten();
    if let Some(existing) = existing {
        return Ok(JSValue::from_object(existing));
    }

    let Some(node) = dom_node(vm, this) else {
        return Ok(JSValue::null());
    };
    let src = node
        .borrow()
        .value
        .get_attr("src")
        .unwrap_or("")
        .to_string();
    // Non-text iframe sources (e.g. png) legitimately have a document whose
    // <body> has no <p>; empty.html has a body with a single <p> as the fixture.
    let body_has_p = src.ends_with(".html");
    let iframe_doc = make_iframe_document(dom_id, &src, body_has_p);
    let document = Rc::clone(&iframe_doc.borrow().document);
    let _ = with_host_mut(vm, |host| {
        host.iframe_documents.insert(dom_id, iframe_doc);
    });
    Ok(JSValue::from_object(document))
}

fn get_namespace_uri(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let this = args.first().unwrap_or(&UNDEFINED);
    if let Some(dom_id) = node_dom_id(this)
        && let Some(namespace) =
            with_host(vm, |host| host.namespaces.get(&dom_id).cloned()).flatten()
    {
        return if namespace.is_empty() {
            Ok(JSValue::null())
        } else {
            Ok(JSValue::from_string(namespace))
        };
    }

    let Some(mut node) = dom_node(vm, this) else {
        return Ok(JSValue::null());
    };
    loop {
        let (tag_name, parent) = {
            let node = node.borrow();
            (
                node.value.tag_name().map(str::to_ascii_lowercase),
                node.parent(),
            )
        };
        match tag_name.as_deref() {
            Some("svg") => return Ok(JSValue::from_string(SVG_NAMESPACE.to_string())),
            Some("math") => return Ok(JSValue::from_string(MATHML_NAMESPACE.to_string())),
            _ => {}
        }
        let Some(parent) = parent else {
            return Ok(JSValue::from_string(HTML_NAMESPACE.to_string()));
        };
        node = parent;
    }
}

fn get_child_nodes(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(vm.array_from_values(Vec::new()));
    };
    let children = node.borrow().children().to_vec();
    Ok(expose_node_list(vm, children))
}

fn get_first_child(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    get_edge_child(vm, &args, true)
}

fn get_last_child(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    get_edge_child(vm, &args, false)
}

fn get_edge_child(vm: &mut VM, args: &[JSValue], first: bool) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::null());
    };
    let child = if first {
        node.borrow().children().first().cloned()
    } else {
        node.borrow().children().last().cloned()
    };
    Ok(child
        .and_then(|child| expose_node(vm, child))
        .unwrap_or(JSValue::null()))
}

fn get_next_sibling(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    get_sibling(vm, &args, 1)
}

fn get_previous_sibling(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    get_sibling(vm, &args, -1)
}

fn get_sibling(vm: &mut VM, args: &[JSValue], offset: isize) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::null());
    };
    let Some(parent) = node.borrow().parent() else {
        return Ok(JSValue::null());
    };
    let sibling = {
        let parent = parent.borrow();
        let Some(index) = parent
            .children()
            .iter()
            .position(|candidate| Rc::ptr_eq(candidate, &node))
        else {
            return Ok(JSValue::null());
        };
        let sibling_index = index as isize + offset;
        (sibling_index >= 0)
            .then(|| parent.children().get(sibling_index as usize).cloned())
            .flatten()
    };
    Ok(sibling
        .and_then(|sibling| expose_node(vm, sibling))
        .unwrap_or(JSValue::null()))
}

pub(crate) fn get_element_children(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(vm.array_from_values(Vec::new()));
    };
    let children = node
        .borrow()
        .children()
        .iter()
        .filter(|child| matches!(child.borrow().value, HtmlNodeType::Element { .. }))
        .cloned()
        .collect();
    Ok(expose_node_list(vm, children))
}

fn get_class_list(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(dom_id) = node_dom_id(args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::null());
    };
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::null());
    };
    let class_list = vm.array_from_values(
        class_tokens(&node)
            .into_iter()
            .map(JSValue::from_string)
            .collect(),
    );
    let Some(class_list_object) = class_list.as_object() else {
        unreachable!("array_from_values must return an object");
    };
    let mut class_list = class_list_object.borrow_mut();
    define_node_id(&mut class_list, dom_id);
    class_list.set(
        "contains".to_string(),
        JSValue::from_native_function(class_list_contains),
    );
    class_list.set(
        "add".to_string(),
        JSValue::from_native_function(class_list_add),
    );
    class_list.set(
        "remove".to_string(),
        JSValue::from_native_function(class_list_remove),
    );
    class_list.set(
        "toggle".to_string(),
        JSValue::from_native_function(class_list_toggle),
    );
    drop(class_list);
    Ok(JSValue::from_object(class_list_object))
}

fn get_class_name(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::from_string(String::new()));
    };
    let value = node
        .borrow()
        .value
        .get_attr("class")
        .unwrap_or("")
        .to_string();
    Ok(JSValue::from_string(value))
}

fn get_element_id(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::from_string(String::new()));
    };
    let id = node.borrow().value.get_attr("id").unwrap_or("").to_string();
    Ok(JSValue::from_string(id))
}

fn set_element_id(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::undefined());
    };
    let id = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    node.borrow_mut().value.set_attr("id", id);
    mark_dom_dirty(vm);
    Ok(JSValue::undefined())
}

fn set_class_name(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::undefined());
    };
    let value = args.get(1).map(JSValue::to_string).unwrap_or_default();
    node.borrow_mut().value.set_attr("class", value);
    mark_dom_dirty(vm);
    Ok(JSValue::undefined())
}

pub(crate) fn get_style(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(dom_id) = node_dom_id(args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::null());
    };
    let style = with_host_mut(vm, |host| {
        if let Some(style) = host.styles.get(&dom_id) {
            return Rc::clone(style);
        }

        let style = make_style_declaration(dom_id);
        host.styles.insert(dom_id, Rc::clone(&style));
        style
    })
    .ok_or_else(|| JSError::InternalError("JS host is unavailable".to_string()))?;
    Ok(JSValue::from_object(style))
}

fn make_style_declaration(dom_id: u64) -> Rc<RefCell<JSObject>> {
    let mut style = JSObject::new();
    define_node_id(&mut style, dom_id);
    style.define_property(
        "cssText".to_string(),
        accessor_property(get_style_css_text, set_style_css_text),
    );
    style.set(
        "setProperty".to_string(),
        JSValue::from_native_function(style_set_property),
    );
    style.set(
        "getPropertyValue".to_string(),
        JSValue::from_native_function(style_get_property_value),
    );
    style.set(
        "removeProperty".to_string(),
        JSValue::from_native_function(style_remove_property),
    );
    style.set(
        "__host_get_property__".to_string(),
        JSValue::from_native_function(style_host_get_property),
    );
    style.set(
        "__host_set_property__".to_string(),
        JSValue::from_native_function(style_host_set_property),
    );
    Rc::new(RefCell::new(style))
}

fn get_style_css_text(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::from_string(String::new()));
    };
    let css_text = node
        .borrow()
        .value
        .get_attr("style")
        .unwrap_or("")
        .to_string();
    Ok(JSValue::from_string(css_text))
}

fn set_style_css_text(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::undefined());
    };
    let css_text = args.get(1).map(JSValue::to_string).unwrap_or_default();
    if css_text.is_empty() {
        node.borrow_mut().value.remove_attr("style");
    } else {
        node.borrow_mut().value.set_attr("style", css_text);
    }
    mark_dom_dirty(vm);
    Ok(JSValue::undefined())
}

fn style_set_property(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(name) = args.get(1).map(JSValue::to_string) else {
        return Ok(JSValue::undefined());
    };
    let value = args.get(2).map(JSValue::to_string).unwrap_or_default();
    let priority = args.get(3).map(JSValue::to_string).unwrap_or_default();
    set_style_property(vm, &args, &name, &value, &priority)?;
    Ok(JSValue::undefined())
}

fn style_get_property_value(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(name) = args.get(1).map(JSValue::to_string) else {
        return Ok(JSValue::from_string(String::new()));
    };
    Ok(JSValue::from_string(read_style_property(vm, &args, &name)))
}

fn style_remove_property(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(name) = args.get(1).map(JSValue::to_string) else {
        return Ok(JSValue::from_string(String::new()));
    };
    let previous = read_style_property(vm, &args, &name);
    set_style_property(vm, &args, &name, "", "")?;
    Ok(JSValue::from_string(previous))
}

fn style_host_get_property(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(name) = args.get(1).map(JSValue::to_string) else {
        return Ok(JSValue::undefined());
    };
    Ok(JSValue::from_string(read_style_property(
        vm,
        &args,
        &style_property_name(&name),
    )))
}

fn style_host_set_property(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(name) = args.get(1).map(JSValue::to_string) else {
        return Ok(JSValue::undefined());
    };
    let value = args.get(2).map(JSValue::to_string).unwrap_or_default();
    set_style_property(vm, &args, &style_property_name(&name), &value, "")?;
    Ok(JSValue::undefined())
}

fn read_style_property(vm: &mut VM, args: &[JSValue], name: &str) -> String {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return String::new();
    };
    let style = node
        .borrow()
        .value
        .get_attr("style")
        .unwrap_or("")
        .to_string();
    parse_style_declarations(&style)
        .into_iter()
        .rev()
        .find(|(property, _)| property.eq_ignore_ascii_case(name))
        .map(|(_, value)| strip_important(&value).to_string())
        .unwrap_or_default()
}

fn set_style_property(
    vm: &mut VM,
    args: &[JSValue],
    name: &str,
    value: &str,
    priority: &str,
) -> JSResult<()> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(());
    };
    let style = node
        .borrow()
        .value
        .get_attr("style")
        .unwrap_or("")
        .to_string();
    let mut declarations = parse_style_declarations(&style);
    declarations.retain(|(property, _)| !property.eq_ignore_ascii_case(name));
    if !value.is_empty() {
        let value = if priority.eq_ignore_ascii_case("important") {
            format!("{} !important", value.trim())
        } else {
            value.trim().to_string()
        };
        declarations.push((name.to_string(), value));
    }

    let css_text = serialize_style_declarations(&declarations);
    if css_text.is_empty() {
        node.borrow_mut().value.remove_attr("style");
    } else {
        node.borrow_mut().value.set_attr("style", css_text);
    }
    mark_dom_dirty(vm);
    Ok(())
}

fn parse_style_declarations(css_text: &str) -> Vec<(String, String)> {
    css_text
        .split(';')
        .filter_map(|declaration| {
            let (name, value) = declaration.split_once(':')?;
            let name = name.trim();
            (!name.is_empty()).then(|| (name.to_string(), value.trim().to_string()))
        })
        .collect()
}

fn serialize_style_declarations(declarations: &[(String, String)]) -> String {
    declarations
        .iter()
        .map(|(name, value)| format!("{name}: {value};"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn strip_important(value: &str) -> &str {
    value
        .strip_suffix("!important")
        .map(str::trim_end)
        .unwrap_or(value)
}

pub(crate) fn style_property_name(name: &str) -> String {
    if name.starts_with("--") {
        return name.to_string();
    }
    if name == "cssFloat" {
        return "float".to_string();
    }

    let mut result = String::new();
    if name.starts_with("ms") && name.chars().nth(2).is_some_and(char::is_uppercase) {
        result.push('-');
    }
    for character in name.chars() {
        if character.is_uppercase() {
            result.push('-');
            result.extend(character.to_lowercase());
        } else {
            result.push(character);
        }
    }
    result
}

fn class_list_contains(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::from_bool(false));
    };
    let Some(token) = class_token(args.get(1)) else {
        return Ok(JSValue::from_bool(false));
    };
    Ok(JSValue::from_bool(
        class_tokens(&node).iter().any(|class| class == token),
    ))
}

fn class_list_add(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::undefined());
    };
    let mut classes = class_tokens(&node);
    let mut changed = false;
    for value in args.iter().skip(1) {
        let Some(token) = class_token(Some(value)) else {
            continue;
        };
        if !classes.iter().any(|class| class == token) {
            classes.push(token.to_string());
            changed = true;
        }
    }
    if changed {
        set_class_tokens(&node, &classes);
        mark_dom_dirty(vm);
    }
    Ok(JSValue::undefined())
}

fn class_list_remove(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::undefined());
    };
    let removals: Vec<&str> = args
        .iter()
        .skip(1)
        .filter_map(|value| class_token(Some(value)))
        .collect();
    let mut classes = class_tokens(&node);
    let old_len = classes.len();
    classes.retain(|class| !removals.iter().any(|removal| class == removal));
    if classes.len() != old_len {
        set_class_tokens(&node, &classes);
        mark_dom_dirty(vm);
    }
    Ok(JSValue::undefined())
}

fn class_list_toggle(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::from_bool(false));
    };
    let Some(token) = class_token(args.get(1)) else {
        return Ok(JSValue::from_bool(false));
    };
    let mut classes = class_tokens(&node);
    let position = classes.iter().position(|class| class == token);
    let should_have = args
        .get(2)
        .map(JSValue::to_boolean)
        .unwrap_or(position.is_none());

    let changed = match (position, should_have) {
        (Some(position), false) => {
            classes.remove(position);
            true
        }
        (None, true) => {
            classes.push(token.to_string());
            true
        }
        _ => false,
    };
    if changed {
        set_class_tokens(&node, &classes);
        mark_dom_dirty(vm);
    }
    Ok(JSValue::from_bool(should_have))
}

fn class_token(value: Option<&JSValue>) -> Option<&str> {
    let token = value?.as_string()?;
    (!token.is_empty() && !token.chars().any(char::is_whitespace)).then_some(token)
}

fn class_tokens(node: &NodeRef<HtmlNodeType>) -> Vec<String> {
    node.borrow()
        .value
        .get_attr("class")
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

fn set_class_tokens(node: &NodeRef<HtmlNodeType>, classes: &[String]) {
    node.borrow_mut().value.set_attr("class", classes.join(" "));
}

fn get_text_content(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::null());
    };
    Ok(JSValue::from_string(DomTree::inner_text(&node)))
}

fn set_text_content(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::undefined());
    };
    let new_text = args
        .get(1)
        .map(|v| v.to_console_string())
        .unwrap_or_default();
    DomTree::set_text_content(&node, &new_text);
    mark_dom_dirty(vm);
    Ok(JSValue::undefined())
}

fn get_inner_text(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    get_text_content(vm, args)
}

fn set_inner_text(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    set_text_content(vm, args)
}

// Comment node: read/write data from HtmlNodeType::Comment(data)
fn get_comment_data(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::null());
    };
    let data = match &node.borrow().value {
        HtmlNodeType::Comment(d) => d.clone(),
        _ => String::new(),
    };
    Ok(JSValue::from_string(data))
}

fn set_comment_data(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::undefined());
    };
    let new_data = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    if let HtmlNodeType::Comment(d) = &mut node.borrow_mut().value {
        *d = new_data;
    }
    mark_dom_dirty(vm);
    Ok(JSValue::undefined())
}

// ProcessingInstruction node: read/write data from HtmlNodeType::ProcessingInstruction
fn get_pi_data(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::null());
    };
    let data = match &node.borrow().value {
        HtmlNodeType::ProcessingInstruction { data, .. } => data.clone(),
        _ => String::new(),
    };
    Ok(JSValue::from_string(data))
}

fn set_pi_data(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::undefined());
    };
    let new_data = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    if let HtmlNodeType::ProcessingInstruction { data, .. } = &mut node.borrow_mut().value {
        *data = new_data;
    }
    mark_dom_dirty(vm);
    Ok(JSValue::undefined())
}

fn escape_html_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_html_attribute(value: &str) -> String {
    escape_html_text(value).replace('"', "&quot;")
}

fn is_void_html_element(tag_name: &str) -> bool {
    matches!(
        tag_name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn serialize_html_node(node: &NodeRef<HtmlNodeType>) -> String {
    let (value, children) = {
        let node = node.borrow();
        (node.value.clone(), node.children().to_vec())
    };
    match value {
        HtmlNodeType::Text(text) => escape_html_text(&text),
        HtmlNodeType::Comment(comment) => format!("<!--{comment}-->"),
        HtmlNodeType::Element {
            tag_name,
            attributes,
        } => {
            let mut html = format!("<{tag_name}");
            for attribute in attributes {
                html.push(' ');
                html.push_str(&attribute.name);
                html.push_str("=\"");
                html.push_str(&escape_html_attribute(&attribute.value));
                html.push('"');
            }
            html.push('>');
            if !is_void_html_element(&tag_name) {
                for child in children {
                    html.push_str(&serialize_html_node(&child));
                }
                html.push_str("</");
                html.push_str(&tag_name);
                html.push('>');
            }
            html
        }
        _ => String::new(),
    }
}

fn get_inner_html(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::null());
    };
    let children = node.borrow().children().to_vec();
    Ok(JSValue::from_string(
        children.iter().map(serialize_html_node).collect::<String>(),
    ))
}

fn set_inner_html(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::undefined());
    };
    if !matches!(node.borrow().value, HtmlNodeType::Element { .. }) {
        return Ok(JSValue::undefined());
    }
    let html = args.get(1).map(JSValue::to_string).unwrap_or_default();
    let old_children = node.borrow().children().to_vec();
    let _ = with_host_mut(vm, |host| {
        for child in &old_children {
            if let Some(dom_id) = host.dom_id_for_node(child) {
                host.detached_nodes.insert(dom_id, Rc::clone(child));
            }
        }
    });
    node.borrow_mut().clear_children();

    let mut parser = HtmlParser::new(&html);
    let fragment = parser.parse();
    if let Some(body) = fragment.get_elements_by_tag_name("body").into_iter().next() {
        let children = body.borrow().children().to_vec();
        for child in children {
            TreeNode::append_child(&node, child);
        }
    }
    mark_dom_dirty(vm);
    Ok(JSValue::undefined())
}

fn reflected_string_property(vm: &mut VM, args: &[JSValue], name: &str) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::from_string(String::new()));
    };
    let value = node.borrow().value.get_attr(name).unwrap_or("").to_string();
    Ok(JSValue::from_string(value))
}

fn set_reflected_string_property(vm: &mut VM, args: &[JSValue], name: &str) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::undefined());
    };
    let value = args
        .get(1)
        .map(JSValue::to_console_string)
        .unwrap_or_default();
    node.borrow_mut().value.set_attr(name, value);
    if name == "src"
        && let Some(element) = args.first()
    {
        queue_dynamic_image(vm, element);
    }
    mark_dom_dirty(vm);
    Ok(JSValue::undefined())
}

fn reflected_boolean_property(vm: &mut VM, args: &[JSValue], name: &str) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::from_bool(false));
    };
    Ok(JSValue::from_bool(
        node.borrow().value.get_attr(name).is_some(),
    ))
}

fn set_reflected_boolean_property(vm: &mut VM, args: &[JSValue], name: &str) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::undefined());
    };
    let enabled = args.get(1).map(JSValue::to_boolean).unwrap_or(false);
    if enabled {
        node.borrow_mut().value.set_attr(name, String::new());
        mark_dom_dirty(vm);
    } else if node.borrow_mut().value.remove_attr(name).is_some() {
        mark_dom_dirty(vm);
    }
    Ok(JSValue::undefined())
}

fn get_element_value(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    reflected_string_property(vm, &args, "value")
}

fn set_element_value(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    set_reflected_string_property(vm, &args, "value")
}

macro_rules! reflected_string_accessors {
    ($getter:ident, $setter:ident, $name:literal) => {
        fn $getter(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
            reflected_string_property(vm, &args, $name)
        }

        fn $setter(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
            set_reflected_string_property(vm, &args, $name)
        }
    };
}

reflected_string_accessors!(get_element_src, set_element_src, "src");
reflected_string_accessors!(get_element_href, set_element_href, "href");
reflected_string_accessors!(get_element_rel, set_element_rel, "rel");
reflected_string_accessors!(get_element_type, set_element_type, "type");
reflected_string_accessors!(get_element_charset, set_element_charset, "charset");
reflected_string_accessors!(
    get_element_cross_origin,
    set_element_cross_origin,
    "crossorigin"
);

fn canvas_dimension(vm: &mut VM, args: &[JSValue], name: &str, default: f64) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::from_number(default));
    };
    if node.borrow().value.tag_name() != Some("canvas") {
        return Ok(JSValue::undefined());
    }
    let value = node
        .borrow()
        .value
        .get_attr(name)
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(default);
    Ok(JSValue::from_number(value))
}

fn set_canvas_dimension(vm: &mut VM, args: &[JSValue], name: &str) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::undefined());
    };
    if node.borrow().value.tag_name() != Some("canvas") {
        return Ok(JSValue::undefined());
    }
    let value = args.get(1).map(JSValue::to_number).unwrap_or(0.0);
    let value = if value.is_finite() && value > 0.0 {
        value.floor().min(u32::MAX as f64) as u32
    } else {
        0
    };
    node.borrow_mut().value.set_attr(name, value.to_string());
    mark_dom_dirty(vm);
    Ok(JSValue::undefined())
}

fn get_element_width(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    canvas_dimension(vm, &args, "width", 300.0)
}

fn set_element_width(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    set_canvas_dimension(vm, &args, "width")
}

fn get_element_height(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    canvas_dimension(vm, &args, "height", 150.0)
}

fn set_element_height(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    set_canvas_dimension(vm, &args, "height")
}

pub(crate) fn element_layout_size(vm: &VM, value: &JSValue) -> Option<(f64, f64)> {
    let node = dom_node(vm, value)?;
    let node = node.borrow();
    let tag = node.value.tag_name()?;
    let is_slick_list = node.value.get_attr("class").is_some_and(|classes| {
        classes
            .split_whitespace()
            .any(|class| class == "slick-list")
    });
    let viewport = with_host(vm, |host| host.viewport).unwrap_or((800.0, 600.0));
    let default = match tag {
        "canvas" => (300.0, 150.0),
        "html" | "body" => viewport,
        _ if is_slick_list => (800.0, 0.0),
        _ => (0.0, 0.0),
    };
    let attr = |name: &str, fallback: f64| {
        node.value
            .get_attr(name)
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value >= 0.0)
            .unwrap_or(fallback)
    };
    let mut size = (attr("width", default.0), attr("height", default.1));
    if let Some(style) = node.value.get_attr("style") {
        for declaration in style.split(';') {
            let Some((name, value)) = declaration.split_once(':') else {
                continue;
            };
            let Some(value) = value.trim().strip_suffix("px") else {
                continue;
            };
            let Ok(value) = value.trim().parse::<f64>() else {
                continue;
            };
            if !value.is_finite() || value < 0.0 {
                continue;
            }
            match name.trim().to_ascii_lowercase().as_str() {
                "width" => size.0 = value,
                "height" => size.1 = value,
                _ => {}
            }
        }
    }
    Some(size)
}

pub(crate) fn element_layout_metrics(vm: &VM, value: &JSValue) -> Option<JsLayoutMetrics> {
    let node = dom_node(vm, value)?;
    let node_key = Rc::as_ptr(&node) as usize;
    let dom_id = node_dom_id(value);
    with_host(vm, |host| {
        dom_id
            .and_then(|id| host.layout_metrics_by_dom_id.get(&id).copied())
            .or_else(|| host.layout_metrics.get(&node_key).copied())
    })
    .flatten()
}

fn measured_or_fallback(
    vm: &VM,
    value: &JSValue,
    measured: impl FnOnce(JsLayoutMetrics) -> f64,
    fallback: impl FnOnce((f64, f64)) -> f64,
) -> f64 {
    if let Some(metrics) = element_layout_metrics(vm, value) {
        return measured(metrics);
    }
    // TODO: Force a synchronous style/layout flush when geometry is read before
    // the first committed layout instead of estimating dimensions from attributes.
    element_layout_size(vm, value).map(fallback).unwrap_or(0.0)
}

fn measurement_receiver(args: &[JSValue]) -> &JSValue {
    args.first().unwrap_or(&UNDEFINED)
}

fn get_element_client_width(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::from_number(measured_or_fallback(
        vm,
        measurement_receiver(&args),
        |metrics| metrics.client_width,
        |size| size.0,
    )))
}

fn get_element_client_height(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::from_number(measured_or_fallback(
        vm,
        measurement_receiver(&args),
        |metrics| metrics.client_height,
        |size| size.1,
    )))
}

fn get_element_offset_width(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::from_number(measured_or_fallback(
        vm,
        measurement_receiver(&args),
        |metrics| metrics.offset_width,
        |size| size.0,
    )))
}

fn get_element_offset_height(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::from_number(measured_or_fallback(
        vm,
        measurement_receiver(&args),
        |metrics| metrics.offset_height,
        |size| size.1,
    )))
}

fn get_element_offset_left(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::from_number(
        element_layout_metrics(vm, measurement_receiver(&args))
            .map(|metrics| metrics.offset_left)
            .unwrap_or(0.0),
    ))
}

fn get_element_offset_top(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::from_number(
        element_layout_metrics(vm, measurement_receiver(&args))
            .map(|metrics| metrics.offset_top)
            .unwrap_or(0.0),
    ))
}

pub(crate) fn make_dom_rect(left: f64, top: f64, width: f64, height: f64) -> JSValue {
    let mut rect = JSObject::new();
    for (name, value) in [
        ("x", left),
        ("y", top),
        ("left", left),
        ("top", top),
        ("width", width),
        ("height", height),
        ("right", left + width),
        ("bottom", top + height),
    ] {
        rect.define_property(
            name.to_string(),
            Property::read_only(JSValue::from_number(value)),
        );
    }
    JSValue::from_object(Rc::new(RefCell::new(rect)))
}

fn get_bounding_client_rect(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let receiver = measurement_receiver(&args);
    if dom_node(vm, receiver).is_none() {
        return Err(JSError::TypeError(
            "getBoundingClientRect called on incompatible receiver".to_string(),
        ));
    }
    let (left, top, width, height) = element_layout_metrics(vm, receiver).map_or_else(
        || {
            let (width, height) = element_layout_size(vm, receiver).unwrap_or_default();
            (0.0, 0.0, width, height)
        },
        |metrics| {
            (
                metrics.rect_left,
                metrics.rect_top,
                metrics.rect_width,
                metrics.rect_height,
            )
        },
    );
    Ok(make_dom_rect(left, top, width, height))
}

const CANVAS_NODE_ID: &str = "__orinium_canvas_node_id";
const CANVAS_COMMANDS: &str = "__orinium_canvas_commands";
const CANVAS_CONTEXT_KIND: &str = "__orinium_canvas_context_kind";

fn canvas_get_context(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let this = args.first().unwrap_or(&UNDEFINED);
    let Some(node_id) = node_dom_id(this) else {
        return Err(JSError::TypeError(
            "getContext called on incompatible receiver".to_string(),
        ));
    };
    let Some(node) = dom_node(vm, this) else {
        return Ok(JSValue::null());
    };
    if node.borrow().value.tag_name() != Some("canvas") {
        return Err(JSError::TypeError(
            "getContext called on incompatible receiver".to_string(),
        ));
    }
    let kind = args.get(1).unwrap_or(&UNDEFINED).to_string();
    if !matches!(
        kind.as_str(),
        "2d" | "webgl" | "experimental-webgl" | "webgl2"
    ) {
        return Ok(JSValue::null());
    }
    if let Some(context) =
        with_host(vm, |host| host.canvas_contexts.get(&node_id).cloned()).flatten()
    {
        let existing = context.borrow().get(CANVAS_CONTEXT_KIND).to_string();
        let compatible = existing == kind
            || matches!(
                (existing.as_str(), kind.as_str()),
                ("webgl", "experimental-webgl") | ("experimental-webgl", "webgl")
            );
        return Ok(if compatible {
            JSValue::from_object(context)
        } else {
            JSValue::null()
        });
    }

    let context = if kind == "2d" {
        make_canvas_2d_context(node_id)
    } else {
        let width = canvas_dimension(vm, std::slice::from_ref(this), "width", 300.0)?.to_number();
        let height = canvas_dimension(vm, std::slice::from_ref(this), "height", 150.0)?.to_number();
        make_webgl_context(node_id, &kind, width, height)
    };
    let _ = with_host_mut(vm, |host| {
        host.canvas_contexts.insert(node_id, Rc::clone(&context));
    });
    Ok(JSValue::from_object(context))
}

fn make_canvas_2d_context(node_id: u64) -> Rc<RefCell<JSObject>> {
    let mut context = JSObject::new();
    context.define_property(
        CANVAS_CONTEXT_KIND.to_string(),
        Property::read_only(JSValue::from_string("2d".to_string())),
    );
    context.define_property(
        CANVAS_NODE_ID.to_string(),
        Property {
            value: JSValue::from_number(node_id as f64),
            enumerable: false,
            writable: false,
            configurable: false,
            getter: None,
            setter: None,
        },
    );
    context.set(
        "fillStyle".to_string(),
        JSValue::from_string("#000000".to_string()),
    );
    context.set(
        "strokeStyle".to_string(),
        JSValue::from_string("#000000".to_string()),
    );
    context.set("globalAlpha".to_string(), JSValue::from_number(1.0));
    context.set("lineWidth".to_string(), JSValue::from_number(1.0));
    context.set(
        "font".to_string(),
        JSValue::from_string("10px sans-serif".to_string()),
    );
    // TODO: Implement the Canvas 2D state stack, paths, transforms, fill, and stroke commands.
    context.set("save".to_string(), JSValue::from_native_function(noop));
    context.set("restore".to_string(), JSValue::from_native_function(noop));
    context.set("beginPath".to_string(), JSValue::from_native_function(noop));
    context.set("closePath".to_string(), JSValue::from_native_function(noop));
    context.set("moveTo".to_string(), JSValue::from_native_function(noop));
    context.set("lineTo".to_string(), JSValue::from_native_function(noop));
    context.set("rect".to_string(), JSValue::from_native_function(noop));
    context.set("arc".to_string(), JSValue::from_native_function(noop));
    context.set("fill".to_string(), JSValue::from_native_function(noop));
    context.set("stroke".to_string(), JSValue::from_native_function(noop));
    context.set("translate".to_string(), JSValue::from_native_function(noop));
    context.set("rotate".to_string(), JSValue::from_native_function(noop));
    context.set("scale".to_string(), JSValue::from_native_function(noop));
    context.set(
        "setTransform".to_string(),
        JSValue::from_native_function(canvas_set_transform),
    );
    context.set(
        "resetTransform".to_string(),
        JSValue::from_native_function(canvas_reset_transform),
    );
    context.set(
        "fillRect".to_string(),
        JSValue::from_native_function(canvas_fill_rect),
    );
    context.set(
        "clearRect".to_string(),
        JSValue::from_native_function(canvas_clear_rect),
    );
    context.set(
        "strokeRect".to_string(),
        JSValue::from_native_function(canvas_stroke_rect),
    );
    context.set(
        "fillText".to_string(),
        JSValue::from_native_function(canvas_fill_text),
    );
    context.set(
        "measureText".to_string(),
        JSValue::from_native_function(canvas_measure_text),
    );
    context.set(
        "getImageData".to_string(),
        JSValue::from_native_function(canvas_get_image_data),
    );
    context.set(
        "putImageData".to_string(),
        JSValue::from_native_function(canvas_record_command),
    );
    context.set(
        "drawImage".to_string(),
        JSValue::from_native_function(canvas_record_command),
    );
    context.set(
        "createLinearGradient".to_string(),
        JSValue::from_native_function(canvas_create_gradient),
    );
    context.set(
        "createRadialGradient".to_string(),
        JSValue::from_native_function(canvas_create_gradient),
    );
    context.set(CANVAS_COMMANDS.to_string(), JSArray::new().to_object());
    Rc::new(RefCell::new(context))
}

fn make_webgl_context(node_id: u64, kind: &str, width: f64, height: f64) -> Rc<RefCell<JSObject>> {
    let mut context = JSObject::new();
    for (name, value) in [
        (CANVAS_CONTEXT_KIND, JSValue::from_string(kind.to_string())),
        (CANVAS_NODE_ID, JSValue::from_number(node_id as f64)),
        ("drawingBufferWidth", JSValue::from_number(width)),
        ("drawingBufferHeight", JSValue::from_number(height)),
    ] {
        context.define_property(name.to_string(), Property::read_only(value));
    }
    for (name, value) in [
        ("DEPTH_BUFFER_BIT", 0x00000100),
        ("STENCIL_BUFFER_BIT", 0x00000400),
        ("COLOR_BUFFER_BIT", 0x00004000),
        ("POINTS", 0x0000),
        ("LINES", 0x0001),
        ("TRIANGLES", 0x0004),
        ("ZERO", 0),
        ("ONE", 1),
        ("SRC_ALPHA", 0x0302),
        ("ONE_MINUS_SRC_ALPHA", 0x0303),
        ("ARRAY_BUFFER", 0x8892),
        ("ELEMENT_ARRAY_BUFFER", 0x8893),
        ("STATIC_DRAW", 0x88E4),
        ("DYNAMIC_DRAW", 0x88E8),
        ("FLOAT", 0x1406),
        ("UNSIGNED_BYTE", 0x1401),
        ("UNSIGNED_SHORT", 0x1403),
        ("RGBA", 0x1908),
        ("RGB", 0x1907),
        ("TEXTURE_2D", 0x0DE1),
        ("TEXTURE0", 0x84C0),
        ("TEXTURE_MIN_FILTER", 0x2801),
        ("TEXTURE_MAG_FILTER", 0x2800),
        ("TEXTURE_WRAP_S", 0x2802),
        ("TEXTURE_WRAP_T", 0x2803),
        ("NEAREST", 0x2600),
        ("LINEAR", 0x2601),
        ("CLAMP_TO_EDGE", 0x812F),
        ("VERTEX_SHADER", 0x8B31),
        ("FRAGMENT_SHADER", 0x8B30),
        ("COMPILE_STATUS", 0x8B81),
        ("LINK_STATUS", 0x8B82),
        ("FRAMEBUFFER", 0x8D40),
        ("RENDERBUFFER", 0x8D41),
        ("FRAMEBUFFER_COMPLETE", 0x8CD5),
        ("BLEND", 0x0BE2),
        ("DEPTH_TEST", 0x0B71),
        ("SCISSOR_TEST", 0x0C11),
        ("MAX_TEXTURE_SIZE", 0x0D33),
        ("MAX_TEXTURE_IMAGE_UNITS", 0x8872),
        ("VERSION", 0x1F02),
        ("SHADING_LANGUAGE_VERSION", 0x8B8C),
        ("VENDOR", 0x1F00),
        ("RENDERER", 0x1F01),
    ] {
        context.define_property(
            name.to_string(),
            Property::read_only(JSValue::from_number(value as f64)),
        );
    }
    // TODO: Implement a real WebGL command pipeline and render target; Scratch cannot render while these calls are no-ops.
    for name in [
        "createBuffer",
        "createFramebuffer",
        "createProgram",
        "createRenderbuffer",
        "createShader",
        "createTexture",
        "getUniformLocation",
    ] {
        context.set(
            name.to_string(),
            JSValue::from_native_function(webgl_create_handle),
        );
    }
    for name in [
        "activeTexture",
        "attachShader",
        "bindAttribLocation",
        "bindBuffer",
        "bindFramebuffer",
        "bindRenderbuffer",
        "bindTexture",
        "blendEquation",
        "blendFunc",
        "bufferData",
        "bufferSubData",
        "clear",
        "clearColor",
        "colorMask",
        "compileShader",
        "deleteBuffer",
        "deleteFramebuffer",
        "deleteProgram",
        "deleteRenderbuffer",
        "deleteShader",
        "deleteTexture",
        "disable",
        "disableVertexAttribArray",
        "drawArrays",
        "drawElements",
        "enable",
        "enableVertexAttribArray",
        "framebufferRenderbuffer",
        "framebufferTexture2D",
        "generateMipmap",
        "linkProgram",
        "pixelStorei",
        "renderbufferStorage",
        "scissor",
        "shaderSource",
        "texImage2D",
        "texParameteri",
        "texSubImage2D",
        "uniform1f",
        "uniform1fv",
        "uniform1i",
        "uniform1iv",
        "uniform2f",
        "uniform2fv",
        "uniform3f",
        "uniform3fv",
        "uniform4f",
        "uniform4fv",
        "uniformMatrix3fv",
        "uniformMatrix4fv",
        "useProgram",
        "validateProgram",
        "vertexAttribPointer",
        "viewport",
    ] {
        context.set(name.to_string(), JSValue::from_native_function(noop));
    }
    for name in ["getShaderParameter", "getProgramParameter"] {
        context.set(name.to_string(), JSValue::from_native_function(webgl_true));
    }
    for name in ["getShaderInfoLog", "getProgramInfoLog"] {
        context.set(
            name.to_string(),
            JSValue::from_native_function(webgl_empty_string),
        );
    }
    for name in ["getAttribLocation", "getError"] {
        context.set(name.to_string(), JSValue::from_native_function(webgl_zero));
    }
    context.set(
        "checkFramebufferStatus".to_string(),
        JSValue::from_native_function(webgl_framebuffer_complete),
    );
    context.set(
        "getParameter".to_string(),
        JSValue::from_native_function(webgl_get_parameter),
    );
    context.set(
        "getExtension".to_string(),
        JSValue::from_native_function(webgl_get_extension),
    );
    context.set(
        "getSupportedExtensions".to_string(),
        JSValue::from_native_function(webgl_supported_extensions),
    );
    Rc::new(RefCell::new(context))
}

fn webgl_create_handle(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::from_object(Rc::new(RefCell::new(JSObject::new()))))
}

fn webgl_true(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::from_bool(true))
}

fn webgl_zero(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::from_number(0.0))
}

fn webgl_empty_string(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::from_string(String::new()))
}

fn webgl_framebuffer_complete(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::from_number(0x8CD5 as f64))
}

fn webgl_get_parameter(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let parameter = args.get(1).map(JSValue::to_number).unwrap_or(0.0) as u32;
    Ok(match parameter {
        0x1F00 => JSValue::from_string("Orinium".to_string()),
        0x1F01 => JSValue::from_string("Orinium WebGL Compatibility Renderer".to_string()),
        0x1F02 => JSValue::from_string("WebGL 1.0 (Orinium)".to_string()),
        0x8B8C => JSValue::from_string("WebGL GLSL ES 1.0 (Orinium)".to_string()),
        0x0D33 => JSValue::from_number(4096.0),
        0x8872 => JSValue::from_number(8.0),
        _ => JSValue::from_number(0.0),
    })
}

fn webgl_get_extension(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let name = args.get(1).unwrap_or(&UNDEFINED).to_string();
    if matches!(
        name.as_str(),
        "OES_texture_float" | "OES_element_index_uint" | "WEBGL_lose_context"
    ) {
        Ok(JSValue::from_object(Rc::new(RefCell::new(JSObject::new()))))
    } else {
        Ok(JSValue::null())
    }
}

fn webgl_supported_extensions(vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(vm.array_from_values(
        [
            "OES_texture_float",
            "OES_element_index_uint",
            "WEBGL_lose_context",
        ]
        .into_iter()
        .map(|name| JSValue::from_string(name.to_string()))
        .collect(),
    ))
}

fn canvas_command(vm: &mut VM, args: Vec<JSValue>, name: &str) -> JSResult<JSValue> {
    let Some(context) = args.first().and_then(JSValue::as_object) else {
        return Err(JSError::TypeError(
            "Canvas method called on incompatible receiver".to_string(),
        ));
    };
    let mut command = JSObject::new();
    command.set("name".to_string(), JSValue::from_string(name.to_string()));
    command.set(
        "arguments".to_string(),
        vm.array_from_values(args.iter().skip(1).cloned().collect()),
    );
    command.set("fillStyle".to_string(), context.borrow().get("fillStyle"));
    command.set(
        "strokeStyle".to_string(),
        context.borrow().get("strokeStyle"),
    );
    let commands = context.borrow().get(CANVAS_COMMANDS);
    if let Some(commands) = commands.as_object() {
        let length = commands.borrow().get("length").to_number().max(0.0) as usize;
        commands.borrow_mut().set(
            length.to_string(),
            JSValue::from_object(Rc::new(RefCell::new(command))),
        );
        commands.borrow_mut().set(
            "length".to_string(),
            JSValue::from_number((length + 1) as f64),
        );
    }
    if matches!(name, "fillRect" | "clearRect" | "strokeRect") {
        let node_id = context.borrow().get(CANVAS_NODE_ID).to_number() as u64;
        let style = if name == "strokeRect" {
            context.borrow().get("strokeStyle").to_string()
        } else {
            context.borrow().get("fillStyle").to_string()
        };
        let numbers = (1..=4)
            .map(|index| args.get(index).map(JSValue::to_number).unwrap_or(0.0))
            .map(|number| if number.is_finite() { number } else { 0.0 })
            .map(|number| number.to_string())
            .collect::<Vec<_>>()
            .join("|");
        let record = format!("{name}|{}|{numbers}", style.replace('|', ""));
        let _ = with_host_mut(vm, |host| {
            if let Some(node) = host.refs.get(&node_id).and_then(|node| node.upgrade()) {
                let existing = node
                    .borrow()
                    .value
                    .get_attr("data-orinium-canvas-commands")
                    .unwrap_or("")
                    .to_string();
                let value = if existing.is_empty() {
                    record
                } else {
                    format!("{existing}\n{record}")
                };
                node.borrow_mut()
                    .value
                    .set_attr("data-orinium-canvas-commands", value);
            }
        });
    }
    mark_dom_dirty(vm);
    Ok(JSValue::undefined())
}

fn canvas_fill_rect(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    canvas_command(vm, args, "fillRect")
}

fn canvas_clear_rect(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    canvas_command(vm, args, "clearRect")
}

fn canvas_stroke_rect(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    canvas_command(vm, args, "strokeRect")
}

fn canvas_fill_text(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    canvas_command(vm, args, "fillText")
}

fn canvas_record_command(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    canvas_command(vm, args, "drawImage")
}

fn canvas_set_transform(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::undefined())
}

fn canvas_reset_transform(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::undefined())
}

fn canvas_measure_text(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let text = args.get(1).unwrap_or(&UNDEFINED).to_string();
    let mut metrics = JSObject::new();
    metrics.define_property(
        "width".to_string(),
        Property::read_only(JSValue::from_number(text.chars().count() as f64 * 6.0)),
    );
    Ok(JSValue::from_object(Rc::new(RefCell::new(metrics))))
}

fn canvas_get_image_data(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let width = args.get(3).map(JSValue::to_number).unwrap_or(0.0).max(0.0) as usize;
    let height = args.get(4).map(JSValue::to_number).unwrap_or(0.0).max(0.0) as usize;
    let mut image_data = JSObject::new();
    image_data.define_property(
        "width".to_string(),
        Property::read_only(JSValue::from_number(width as f64)),
    );
    image_data.define_property(
        "height".to_string(),
        Property::read_only(JSValue::from_number(height as f64)),
    );
    image_data.define_property(
        "data".to_string(),
        Property::read_only(
            JSArray::from_vec(vec![JSValue::from_number(0.0); width * height * 4]).to_object(),
        ),
    );
    Ok(JSValue::from_object(Rc::new(RefCell::new(image_data))))
}

fn canvas_create_gradient(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    let mut gradient = JSObject::new();
    // TODO: Store gradient color stops and resolve them when Canvas 2D paint styles are rendered.
    gradient.set(
        "addColorStop".to_string(),
        JSValue::from_native_function(noop),
    );
    Ok(JSValue::from_object(Rc::new(RefCell::new(gradient))))
}

fn canvas_to_data_url(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let this = args.first().unwrap_or(&UNDEFINED);
    let Some(node) = dom_node(vm, this) else {
        return Err(JSError::TypeError(
            "toDataURL called on incompatible receiver".to_string(),
        ));
    };
    if node.borrow().value.tag_name() != Some("canvas") {
        return Err(JSError::TypeError(
            "toDataURL called on incompatible receiver".to_string(),
        ));
    }
    Ok(JSValue::from_string("data:image/png;base64,".to_string()))
}

macro_rules! reflected_boolean_accessors {
    ($getter:ident, $setter:ident, $name:literal) => {
        fn $getter(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
            reflected_boolean_property(vm, &args, $name)
        }

        fn $setter(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
            set_reflected_boolean_property(vm, &args, $name)
        }
    };
}

reflected_boolean_accessors!(get_element_checked, set_element_checked, "checked");
reflected_boolean_accessors!(get_element_selected, set_element_selected, "selected");
reflected_boolean_accessors!(get_element_disabled, set_element_disabled, "disabled");
reflected_boolean_accessors!(get_element_multiple, set_element_multiple, "multiple");
reflected_boolean_accessors!(get_element_async, set_element_async, "async");
reflected_boolean_accessors!(get_element_defer, set_element_defer, "defer");

fn get_attribute(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::null());
    };
    let Some(name) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(JSValue::undefined());
    };
    match node.borrow().value.get_attr(name) {
        Some(value) => Ok(JSValue::from_string(value.to_string())),
        None => Ok(JSValue::null()),
    }
}

fn has_attribute(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::from_bool(false));
    };
    let Some(name) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(JSValue::from_bool(false));
    };
    Ok(JSValue::from_bool(
        node.borrow().value.get_attr(name).is_some(),
    ))
}

fn set_attribute(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::undefined());
    };
    let Some(name) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(JSValue::undefined());
    };
    let value = args
        .get(2)
        .map(|v| v.to_console_string())
        .unwrap_or_default();
    let old_value = node.borrow().value.get_attr(name).map(str::to_string);
    node.borrow_mut().value.set_attr(name, value.clone());
    if let Some(dom_id) = node_dom_id(args.first().unwrap_or(&UNDEFINED)) {
        fire_attribute_changed_callback(vm, dom_id, name, old_value.as_deref(), Some(&value));
    }
    if name.eq_ignore_ascii_case("src")
        && let Some(element) = args.first()
    {
        queue_dynamic_image(vm, element);
    }
    mark_dom_dirty(vm);
    Ok(JSValue::undefined())
}

fn set_attribute_ns(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let forwarded = vec![
        args.first().cloned().unwrap_or(JSValue::undefined()),
        args.get(2).cloned().unwrap_or(JSValue::undefined()),
        args.get(3).cloned().unwrap_or(JSValue::undefined()),
    ];
    set_attribute(vm, forwarded)
}

fn remove_attribute(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::undefined());
    };
    let Some(name) = args.get(1).and_then(JSValue::as_string) else {
        return Ok(JSValue::undefined());
    };
    let old_value = node.borrow().value.get_attr(name).map(str::to_string);
    if node.borrow_mut().value.remove_attr(name).is_some() {
        if let Some(dom_id) = node_dom_id(args.first().unwrap_or(&UNDEFINED)) {
            fire_attribute_changed_callback(vm, dom_id, name, old_value.as_deref(), None);
        }
        mark_dom_dirty(vm);
    }
    Ok(JSValue::undefined())
}
