//! DOM Standard `ParentNode` mixin.
//!
//! Collects the `append()`, `prepend()`, and `replaceChildren()` API shared by
//! `Element`, `Document`, `DocumentFragment` and (through `DocumentFragment`
//! inheritance) `ShadowRoot`.
//!
//! `Node` (`appendChild`, `removeChild`, `firstChild`, …) is a separate
//! responsibility and intentionally lives elsewhere.

use crate::engine::html::HtmlNodeType;
use crate::engine::js::common::{UNDEFINED, dom_node, mark_dom_dirty, with_host, with_host_mut};
use crate::engine::js::web_apis::dom::custom_elements::fire_disconnected_callback;
use crate::engine::js::web_apis::dom::document::{create_text_node, expose_node};
use crate::engine::js::web_apis::dom::element::{
    append_child, insert_before, read_only_accessor_property,
};
use crate::engine::tree::TreeNode;
use pixi_byte::value::jsobject::JSObject;
use pixi_byte::vm::VM;
use pixi_byte::{JSResult, JSValue};
use std::rc::Rc;

/// `parentNode.append(...nodes)` — appends nodes or strings at the end.
pub(crate) fn parent_node_append(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let receiver = args.first().cloned().unwrap_or(JSValue::undefined());
    for value in args.into_iter().skip(1) {
        let child = if dom_node(vm, &value).is_some() {
            value
        } else {
            create_text_node(vm, vec![JSValue::undefined(), value])?
        };
        let _ = append_child(vm, vec![receiver.clone(), child]);
    }
    Ok(JSValue::undefined())
}

/// `parentNode.prepend(...nodes)` — inserts nodes or strings before the first
/// child, in argument order.
pub(crate) fn parent_node_prepend(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let receiver = args.first().cloned().unwrap_or(JSValue::undefined());
    let Some(parent) = dom_node(vm, &receiver) else {
        return Ok(JSValue::undefined());
    };
    let first_child = parent.borrow().children().first().cloned();
    let reference = first_child
        .as_ref()
        .and_then(|fc| expose_node(vm, Rc::clone(fc)));
    for value in args.into_iter().skip(1) {
        let child = if dom_node(vm, &value).is_some() {
            value
        } else {
            create_text_node(vm, vec![JSValue::undefined(), value])?
        };
        let mut call_args = vec![receiver.clone(), child];
        if let Some(ref_js) = &reference {
            call_args.push(ref_js.clone());
        }
        let _ = insert_before(vm, call_args);
    }
    Ok(JSValue::undefined())
}

/// `parentNode.replaceChildren(...nodes)` — replaces all children with the
/// given nodes or strings, in argument order.
pub(crate) fn parent_node_replace_children(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let receiver = args.first().cloned().unwrap_or(JSValue::undefined());
    let Some(parent) = dom_node(vm, &receiver) else {
        return Ok(JSValue::undefined());
    };
    // Remove all existing children, firing disconnected callbacks.
    let children: Vec<_> = parent.borrow().children().to_vec();
    for child in children {
        if TreeNode::remove_child(&parent, &child).is_some() {
            mark_dom_dirty(vm);
            if let Some(dom_id) = with_host(vm, |host| host.dom_id_for_node(&child)).flatten() {
                fire_disconnected_callback(vm, dom_id);
                let _ = with_host_mut(vm, |host| {
                    host.detached_nodes.insert(dom_id, Rc::clone(&child));
                });
            }
        }
    }
    // Append the new nodes (strings become text nodes), in argument order.
    for value in args.into_iter().skip(1) {
        let child = if dom_node(vm, &value).is_some() {
            value
        } else {
            create_text_node(vm, vec![JSValue::undefined(), value])?
        };
        let _ = append_child(vm, vec![receiver.clone(), child]);
    }
    Ok(JSValue::undefined())
}

/// `parentNode.firstElementChild` — first Element child, or null.
pub(crate) fn get_first_element_child(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::null());
    };
    let child = node
        .borrow()
        .children()
        .iter()
        .find(|child| matches!(child.borrow().value, HtmlNodeType::Element { .. }))
        .cloned();
    Ok(child
        .and_then(|child| expose_node(vm, child))
        .unwrap_or(JSValue::null()))
}

/// `parentNode.lastElementChild` — last Element child, or null.
pub(crate) fn get_last_element_child(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::null());
    };
    let child = node
        .borrow()
        .children()
        .iter()
        .rev()
        .find(|child| matches!(child.borrow().value, HtmlNodeType::Element { .. }))
        .cloned();
    Ok(child
        .and_then(|child| expose_node(vm, child))
        .unwrap_or(JSValue::null()))
}

/// `parentNode.childElementCount` — number of Element children.
pub(crate) fn get_child_element_count(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node) = dom_node(vm, args.first().unwrap_or(&UNDEFINED)) else {
        return Ok(JSValue::from_number(0.0));
    };
    let count = node
        .borrow()
        .children()
        .iter()
        .filter(|child| matches!(child.borrow().value, HtmlNodeType::Element { .. }))
        .count();
    Ok(JSValue::from_number(count as f64))
}

/// Registers the `ParentNode` mixin methods on a given interface prototype.
///
/// `children` is intentionally not installed here because it is already wired
/// directly on `Element`, `DocumentFragment`, and `ShadowRoot` via
/// `get_element_children`; installing it again from the mixin would create
/// duplicate accessor definitions.
pub(crate) fn install_parent_node_methods(prototype: &mut JSObject) {
    prototype.set(
        "append".to_string(),
        JSValue::from_native_function(parent_node_append),
    );
    prototype.set(
        "prepend".to_string(),
        JSValue::from_native_function(parent_node_prepend),
    );
    prototype.set(
        "replaceChildren".to_string(),
        JSValue::from_native_function(parent_node_replace_children),
    );
    prototype.define_property(
        "firstElementChild".to_string(),
        read_only_accessor_property(get_first_element_child),
    );
    prototype.define_property(
        "lastElementChild".to_string(),
        read_only_accessor_property(get_last_element_child),
    );
    prototype.define_property(
        "childElementCount".to_string(),
        read_only_accessor_property(get_child_element_count),
    );
}
