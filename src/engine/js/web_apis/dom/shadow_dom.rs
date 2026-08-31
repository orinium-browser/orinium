//! Shadow DOM API — `attachShadow`, `shadowRoot`, and shadow boundary enforcement.
//!
//! Shadow roots are stored as `HtmlNodeType::ShadowRoot` nodes in the DOM tree.
//! The `JsHost::shadow_roots` map records the association host_dom_id →
//! shadow_root_dom_id.

use crate::engine::html::{HtmlNodeType, ShadowRootMode};
use crate::engine::js::common::{node_dom_id, with_host, with_host_mut};
use crate::engine::tree::{NodeRef, TreeNode};
use pixi_byte::vm::VM;
use pixi_byte::{JSError, JSResult, JSValue};
use std::rc::Rc;

/// Returns `true` if the given node is a shadow root.
#[allow(dead_code)]
pub(crate) fn is_shadow_root(node: &NodeRef<HtmlNodeType>) -> bool {
    matches!(node.borrow().value, HtmlNodeType::ShadowRoot { .. })
}

/// Given a host element's DOM id, return the shadow root's DOM id, if any.
#[allow(dead_code)]
pub(crate) fn shadow_root_of_host(vm: &mut VM, host_dom_id: u64) -> Option<u64> {
    with_host(vm, |host| host.shadow_roots.get(&host_dom_id).copied()).flatten()
}

/// Checks whether a node is inside a shadow tree.
#[allow(dead_code)]
pub(crate) fn is_in_shadow_tree(node: &NodeRef<HtmlNodeType>) -> bool {
    let mut current = Some(Rc::clone(node));
    while let Some(n) = current {
        if is_shadow_root(&n) {
            return true;
        }
        current = n.borrow().parent();
    }
    false
}

/// Given a node inside a shadow tree, find the enclosing shadow root.
#[allow(dead_code)]
pub(crate) fn enclosing_shadow_root(node: &NodeRef<HtmlNodeType>) -> Option<NodeRef<HtmlNodeType>> {
    let mut current = Some(Rc::clone(node));
    while let Some(n) = current {
        if is_shadow_root(&n) {
            return Some(n);
        }
        current = n.borrow().parent();
    }
    None
}

// ---------------------------------------------------------------------------
// `element.attachShadow(options)`
// ---------------------------------------------------------------------------

pub(crate) fn element_attach_shadow(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let this = args.first().cloned().unwrap_or(JSValue::undefined());
    let host_dom_id = node_dom_id(&this)
        .ok_or_else(|| JSError::TypeError("attachShadow: not an element".to_string()))?;

    // Parse options.
    let options = args.get(1).cloned().unwrap_or(JSValue::undefined());
    let mode = if let Some(obj) = options.as_object() {
        match obj.borrow().get("mode").as_string().unwrap_or("") {
            "closed" => ShadowRootMode::Closed,
            _ => ShadowRootMode::Open,
        }
    } else {
        ShadowRootMode::Open
    };

    // Check that this element doesn't already have a shadow root.
    let already_has =
        with_host(vm, |host| host.shadow_roots.contains_key(&host_dom_id)).unwrap_or(false);
    if already_has {
        return Err(JSError::TypeError(
            "attachShadow: The element already has a shadow root attached.".to_string(),
        ));
    }

    // Find the host node in the DOM tree.
    let host_node = with_host(vm, |host| host.refs.get(&host_dom_id).cloned())
        .flatten()
        .and_then(|w| w.upgrade())
        .ok_or_else(|| JSError::TypeError("attachShadow: host element not found".to_string()))?;

    // Create the shadow root DOM node.
    let shadow_node = TreeNode::new(HtmlNodeType::ShadowRoot { mode });

    // Attach shadow root as a child of the host element.
    TreeNode::add_child(&host_node, Rc::clone(&shadow_node));

    // Register the shadow root with a DOM id.
    let shadow_dom_id = with_host_mut(vm, |host| {
        host.next_id += 1;
        let id = host.next_id;
        host.refs.insert(id, Rc::downgrade(&shadow_node));
        host.shadow_roots.insert(host_dom_id, id);
        host.detached_nodes.insert(id, Rc::clone(&shadow_node));
        id
    })
    .unwrap_or(0);

    // Expose the shadow root as a JS object.
    let shadow_obj = super::document::expose_shadow_root(vm, &shadow_node, shadow_dom_id)
        .ok_or_else(|| {
            JSError::TypeError("attachShadow: failed to create shadow root object".to_string())
        })?;

    Ok(JSValue::from_object(shadow_obj))
}

// ---------------------------------------------------------------------------
// `element.shadowRoot` (getter)
// ---------------------------------------------------------------------------

pub(crate) fn get_shadow_root(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let this = args.first().cloned().unwrap_or(JSValue::undefined());
    let host_dom_id = node_dom_id(&this)
        .ok_or_else(|| JSError::TypeError("shadowRoot: not an element".to_string()))?;

    let shadow_dom_id_opt =
        with_host(vm, |host| host.shadow_roots.get(&host_dom_id).copied()).flatten();

    let Some(shadow_dom_id) = shadow_dom_id_opt else {
        return Ok(JSValue::null());
    };

    let shadow_ref = with_host(vm, |host| host.refs.get(&shadow_dom_id).cloned())
        .flatten()
        .and_then(|w| w.upgrade());

    if let Some(ref sr) = shadow_ref {
        // Closed shadow roots return null for shadowRoot access.
        if matches!(
            sr.borrow().value,
            HtmlNodeType::ShadowRoot {
                mode: ShadowRootMode::Closed
            }
        ) {
            return Ok(JSValue::null());
        }
    }

    // Return cached JS object or create one.
    let cached = with_host(vm, |host| host.objects.get(&shadow_dom_id).cloned()).flatten();
    if let Some(obj) = cached {
        return Ok(JSValue::from_object(obj));
    }

    let shadow_ref = shadow_ref
        .ok_or_else(|| JSError::TypeError("shadowRoot: shadow root node not found".to_string()))?;
    let obj =
        super::document::expose_shadow_root(vm, &shadow_ref, shadow_dom_id).ok_or_else(|| {
            JSError::TypeError("shadowRoot: failed to create shadow root object".to_string())
        })?;
    Ok(JSValue::from_object(obj))
}
