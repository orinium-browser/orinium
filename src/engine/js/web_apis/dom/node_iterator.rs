//! `NodeIterator` and `TreeWalker` DOM traversal objects.
//!
//! Both are created via `document.createNodeIterator` / `createTreeWalker` and
//! walk a subtree in document order, optionally filtering and restricting by a
//! `whatToShow` bitmask. Each keeps a mutable position (`referenceNode` /
//! `currentNode`) so filters may mutate the tree between steps.

use crate::engine::html::HtmlNodeType;
use crate::engine::js::common::dom_node;
use crate::engine::js::web_apis::dom::document::expose_node;
use crate::engine::js::web_apis::dom::element::accessor_property;
use crate::engine::tree::{NodeRef, TreeNode};
use pixi_byte::value::jsobject::{JSObject, Property};
use pixi_byte::vm::VM;
use pixi_byte::{JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) const FILTER_ACCEPT: f64 = 1.0;
#[allow(dead_code)]
pub(crate) const FILTER_REJECT: f64 = 2.0;
#[allow(dead_code)]
pub(crate) const FILTER_SKIP: f64 = 3.0;

pub(crate) const SHOW_ALL: u32 = 0xFFFFFFFF;
pub(crate) const SHOW_ELEMENT: u32 = 0x1;
pub(crate) const SHOW_TEXT: u32 = 0x4;
pub(crate) const SHOW_PROCESSING_INSTRUCTION: u32 = 0x40;
pub(crate) const SHOW_COMMENT: u32 = 0x80;
pub(crate) const SHOW_DOCUMENT: u32 = 0x100;
pub(crate) const SHOW_DOCUMENT_TYPE: u32 = 0x200;
pub(crate) const SHOW_DOCUMENT_FRAGMENT: u32 = 0x400;

pub(crate) const NODE_ELEMENT: u8 = 1;
pub(crate) const NODE_TEXT: u8 = 3;
pub(crate) const NODE_PROCESSING_INSTRUCTION: u8 = 7;
pub(crate) const NODE_COMMENT: u8 = 8;
pub(crate) const NODE_DOCUMENT: u8 = 9;
pub(crate) const NODE_DOCUMENT_TYPE: u8 = 10;
pub(crate) const NODE_DOCUMENT_FRAGMENT: u8 = 11;

fn node_type_of(node: &NodeRef<HtmlNodeType>) -> u8 {
    match &node.borrow().value {
        HtmlNodeType::Element { .. } => NODE_ELEMENT,
        HtmlNodeType::Text(_) => NODE_TEXT,
        HtmlNodeType::Comment(_) => NODE_COMMENT,
        HtmlNodeType::Document => NODE_DOCUMENT,
        HtmlNodeType::Doctype { .. } => NODE_DOCUMENT_TYPE,
        HtmlNodeType::DocumentFragment => NODE_DOCUMENT_FRAGMENT,
        HtmlNodeType::ProcessingInstruction { .. } => NODE_PROCESSING_INSTRUCTION,
        HtmlNodeType::ShadowRoot { .. } => NODE_DOCUMENT_FRAGMENT,
        HtmlNodeType::InvalidNode(_, _) => 0,
    }
}

fn show_bit_for_type(node_type: u8) -> Option<u32> {
    match node_type {
        NODE_ELEMENT => Some(SHOW_ELEMENT),
        NODE_TEXT => Some(SHOW_TEXT),
        NODE_COMMENT => Some(SHOW_COMMENT),
        NODE_DOCUMENT => Some(SHOW_DOCUMENT),
        NODE_DOCUMENT_TYPE => Some(SHOW_DOCUMENT_TYPE),
        NODE_DOCUMENT_FRAGMENT => Some(SHOW_DOCUMENT_FRAGMENT),
        NODE_PROCESSING_INSTRUCTION => Some(SHOW_PROCESSING_INSTRUCTION),
        _ => None,
    }
}

#[allow(dead_code)]
fn expose_opt(vm: &mut VM, node: Option<NodeRef<HtmlNodeType>>) -> JSResult<JSValue> {
    Ok(node
        .map(|n| expose_node(vm, n).unwrap_or(JSValue::null()))
        .unwrap_or(JSValue::null()))
}

/// The outcome of filtering a node during TreeWalker/NodeIterator traversal, as
/// defined by the DOM "filter a node" algorithm. `Reject` skips the node's whole
/// subtree; `Skip` skips only the node itself but still visits its children.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FilterResult {
    Accept,
    Reject,
    Skip,
}

/// The shared filtering request: `whatToShow` + a `NodeFilter` value.
#[derive(Clone)]
struct FilterSpec {
    what_to_show: u32,
    filter: Option<JSValue>,
    filter_is_function: bool,
}

impl FilterSpec {
    /// `Ok(None)` means the node fails `whatToShow` (must not pass to filter).
    /// `Ok(Some(true/false))` is the filter decision. `Err` forwards a filter
    /// exception.
    fn passes(&self, vm: &mut VM, node: &NodeRef<HtmlNodeType>) -> JSResult<Option<bool>> {
        let bit = match show_bit_for_type(node_type_of(node)) {
            Some(bit) => bit,
            None => return Ok(None),
        };
        if self.what_to_show & bit == 0 {
            return Ok(None);
        }
        let Some(filter) = &self.filter else {
            return Ok(Some(true));
        };
        if self.filter_is_function {
            let arg = expose_node(vm, Rc::clone(node)).unwrap_or(JSValue::null());
            let result = vm.call(filter.clone(), JSValue::undefined(), vec![arg]);
            return match result {
                Ok(v) => Ok(Some(v.to_number() as i64 == FILTER_ACCEPT as i64)),
                Err(e) => Err(e),
            };
        }
        // Numeric filter constant: 1=accept, 2=reject, 3=skip.
        let n = filter.clone().to_number() as i64;
        Ok(Some(n == FILTER_ACCEPT as i64))
    }

    /// Full "filter a node" with distinct Reject vs Skip (spec DOM 6.3).
    fn filter_result(&self, vm: &mut VM, node: &NodeRef<HtmlNodeType>) -> JSResult<FilterResult> {
        let bit = show_bit_for_type(node_type_of(node)).unwrap_or(0);
        if self.what_to_show & bit == 0 {
            return Ok(FilterResult::Skip);
        }
        let Some(filter) = &self.filter else {
            return Ok(FilterResult::Accept);
        };
        let result = if self.filter_is_function {
            let arg = expose_node(vm, Rc::clone(node)).unwrap_or(JSValue::null());
            let result = vm.call(filter.clone(), JSValue::undefined(), vec![arg])?;
            result.to_number() as i64
        } else {
            filter.clone().to_number() as i64
        };
        Ok(match result {
            r if r == FILTER_ACCEPT as i64 => FilterResult::Accept,
            r if r == FILTER_REJECT as i64 => FilterResult::Reject,
            _ => FilterResult::Skip,
        })
    }
}

// ---------------------------------------------------------------------------
// Document-order navigation helpers
// ---------------------------------------------------------------------------

/// The node immediately following `node` in tree order within `root`'s subtree
/// (never returning a node outside `root`). `root` itself is only returned if
/// it is the starting node.
fn following(
    node: &NodeRef<HtmlNodeType>,
    root: &NodeRef<HtmlNodeType>,
) -> Option<NodeRef<HtmlNodeType>> {
    if let Some(first) = node.borrow().children().first().cloned() {
        return Some(first);
    }
    let mut cur = Rc::clone(node);
    loop {
        if Rc::ptr_eq(&cur, root) {
            return None;
        }
        let parent = cur.borrow().parent()?;
        let siblings = parent.borrow().children().to_vec();
        if let Some(nxt) = siblings
            .iter()
            .position(|c| Rc::ptr_eq(c, &cur))
            .and_then(|idx| siblings.get(idx + 1).cloned())
        {
            return Some(nxt);
        }
        cur = parent;
    }
}

/// The node immediately preceding `node` in tree order within `root`'s subtree.
fn preceding(
    node: &NodeRef<HtmlNodeType>,
    root: &NodeRef<HtmlNodeType>,
) -> Option<NodeRef<HtmlNodeType>> {
    let parent = node.borrow().parent()?;
    let siblings = parent.borrow().children().to_vec();
    if let Some(idx) = siblings
        .iter()
        .position(|c| Rc::ptr_eq(c, node))
        .filter(|&i| i > 0)
    {
        let mut cur = Rc::clone(&siblings[idx - 1]);
        loop {
            let children = cur.borrow().children().to_vec();
            match children.last() {
                Some(last) => cur = Rc::clone(last),
                None => break,
            }
        }
        if is_within(&cur, root) {
            return Some(cur);
        }
        return None;
    }
    // No previous sibling; the parent itself is the preceding node. (Even the
    // root may be returned here: previousNode() filters the root like any
    // other candidate.)
    if is_within(&parent, root) {
        return Some(parent);
    }
    None
}

fn is_within(node: &NodeRef<HtmlNodeType>, root: &NodeRef<HtmlNodeType>) -> bool {
    let mut cur = Rc::clone(node);
    loop {
        if Rc::ptr_eq(&cur, root) {
            return true;
        }
        let Some(parent) = cur.borrow().parent() else {
            return false;
        };
        cur = parent;
    }
}

// ---------------------------------------------------------------------------
// Shared position tracking (side table keyed by object pointer)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct NodeIteratorState {
    root: NodeRef<HtmlNodeType>,
    filter: FilterSpec,
    reference: NodeRef<HtmlNodeType>,
    pointer_before: bool,
}

#[derive(Clone)]
struct TreeWalkerState {
    root: NodeRef<HtmlNodeType>,
    filter: FilterSpec,
    current: NodeRef<HtmlNodeType>,
}

#[derive(Clone)]
enum IterKind {
    Iterator(NodeIteratorState),
    Walker(TreeWalkerState),
}

thread_local! {
    static STATES: RefCell<HashMap<usize, IterKind>> = RefCell::new(HashMap::new());
}

use std::collections::HashMap;

fn obj_ptr(obj: &JSObject) -> usize {
    obj as *const JSObject as usize
}

fn this_state(_vm: &VM, args: &[JSValue]) -> Option<IterKind> {
    let this = args.first().cloned().unwrap_or(JSValue::undefined());
    let obj = this.as_object()?;
    let ptr = obj_ptr(&obj.borrow());
    STATES.with(|s| s.borrow().get(&ptr).cloned())
}

fn set_walker_current(this: &JSValue, node: NodeRef<HtmlNodeType>) {
    let Some(obj) = this.as_object() else {
        return;
    };
    let ptr = obj_ptr(&obj.borrow());
    STATES.with(|s| {
        let mut m = s.borrow_mut();
        if let Some(IterKind::Walker(w)) = m.get_mut(&ptr) {
            w.current = node;
        }
    });
}

fn set_walker_current_v(vm: &mut VM, this: &JSValue, node: NodeRef<HtmlNodeType>) {
    set_walker_current(this, Rc::clone(&node));
    obj_set_node_prop(vm, this, "$current_node", node);
}

fn obj_set_node_prop(vm: &mut VM, this: &JSValue, name: &str, node: NodeRef<HtmlNodeType>) {
    let _ = vm;
    let Some(obj) = this.as_object() else {
        return;
    };
    let value = expose_node(vm, node).unwrap_or(JSValue::null());
    obj.borrow_mut().set(name.to_string(), value);
}

// ---------------------------------------------------------------------------
// NodeIterator methods
// ---------------------------------------------------------------------------

fn make_iterator_object(vm: &mut VM, state: NodeIteratorState) -> JSValue {
    let mut obj = JSObject::new();
    obj.define_property(
        "root".to_string(),
        Property::read_only(expose_node(vm, Rc::clone(&state.root)).unwrap_or(JSValue::null())),
    );
    obj.define_property(
        "whatToShow".to_string(),
        Property::read_only(JSValue::from_number(state.filter.what_to_show as f64)),
    );
    obj.define_property(
        "referenceNode".to_string(),
        Property::read_only(
            expose_node(vm, Rc::clone(&state.reference)).unwrap_or(JSValue::null()),
        ),
    );
    obj.define_property(
        "pointerBeforeReferenceNode".to_string(),
        Property::read_only(JSValue::from_bool(state.pointer_before)),
    );
    obj.set(
        "nextNode".to_string(),
        JSValue::from_native_function(iterator_next_node),
    );
    obj.set(
        "previousNode".to_string(),
        JSValue::from_native_function(iterator_previous_node),
    );
    let o = JSValue::from_object(Rc::new(RefCell::new(obj)));
    if let Some(inner) = o.as_object() {
        let ptr = obj_ptr(&inner.borrow());
        STATES.with(|s| {
            s.borrow_mut().insert(ptr, IterKind::Iterator(state));
        });
    }
    o
}

fn iterator_next_node(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let IterKind::Iterator(mut st) =
        this_state(vm, &args).unwrap_or(IterKind::Iterator(default_iterator(vm, &args)?))
    else {
        return Ok(JSValue::null());
    };
    let mut node = Rc::clone(&st.reference);
    let mut before = st.pointer_before;
    loop {
        if before {
            before = false;
        } else {
            let next = following(&node, &st.root);
            match next {
                Some(n) => node = n,
                None => break,
            }
        }
        match st.filter.passes(vm, &node)? {
            None => continue,
            Some(true) => {
                let result = expose_node(vm, Rc::clone(&node)).unwrap_or(JSValue::null());
                st.reference = Rc::clone(&node);
                st.pointer_before = false;
                store_iterator(vm, &args, &st);
                return Ok(result);
            }
            Some(false) => continue,
        }
    }
    st.reference = Rc::clone(&st.root);
    st.pointer_before = true;
    store_iterator(vm, &args, &st);
    Ok(JSValue::null())
}

fn iterator_previous_node(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let IterKind::Iterator(mut st) =
        this_state(vm, &args).unwrap_or(IterKind::Iterator(default_iterator(vm, &args)?))
    else {
        return Ok(JSValue::null());
    };
    let mut node = Rc::clone(&st.reference);
    let mut before = st.pointer_before;
    loop {
        if !before {
            before = true;
        } else {
            let prev = preceding(&node, &st.root);
            match prev {
                Some(n) => node = n,
                None => break,
            }
        }
        match st.filter.passes(vm, &node)? {
            None => continue,
            Some(true) => {
                let result = expose_node(vm, Rc::clone(&node)).unwrap_or(JSValue::null());
                st.reference = Rc::clone(&node);
                st.pointer_before = false;
                store_iterator(vm, &args, &st);
                return Ok(result);
            }
            Some(false) => continue,
        }
    }
    st.reference = Rc::clone(&st.root);
    st.pointer_before = true;
    store_iterator(vm, &args, &st);
    Ok(JSValue::null())
}

fn default_iterator(vm: &mut VM, args: &[JSValue]) -> JSResult<NodeIteratorState> {
    let this = args.first().cloned().unwrap_or(JSValue::undefined());
    let root = if let Some(obj) = this.as_object() {
        let rv = obj.borrow().get("root");
        dom_node(vm, &rv).unwrap_or_else(|| TreeNode::new(HtmlNodeType::Document))
    } else {
        TreeNode::new(HtmlNodeType::Document)
    };
    Ok(NodeIteratorState {
        root: Rc::clone(&root),
        filter: FilterSpec {
            what_to_show: SHOW_ALL,
            filter: None,
            filter_is_function: false,
        },
        reference: root,
        pointer_before: true,
    })
}

fn store_iterator(vm: &mut VM, args: &[JSValue], st: &NodeIteratorState) {
    let _ = vm;
    let Some(this) = args.first().cloned() else {
        return;
    };
    let Some(obj) = this.as_object() else {
        return;
    };
    let ptr = obj_ptr(&obj.borrow());
    STATES.with(|s| {
        let mut m = s.borrow_mut();
        if let Some(IterKind::Iterator(cur)) = m.get_mut(&ptr) {
            cur.reference = Rc::clone(&st.reference);
            cur.pointer_before = st.pointer_before;
        }
    });
    let _ = &obj;
}

// ---------------------------------------------------------------------------
// TreeWalker methods
// ---------------------------------------------------------------------------

fn make_walker_object(vm: &mut VM, state: TreeWalkerState) -> JSValue {
    let mut obj = JSObject::new();
    obj.define_property(
        "root".to_string(),
        Property::read_only(expose_node(vm, Rc::clone(&state.root)).unwrap_or(JSValue::null())),
    );
    obj.define_property(
        "whatToShow".to_string(),
        Property::read_only(JSValue::from_number(state.filter.what_to_show as f64)),
    );
    obj.define_property(
        "currentNode".to_string(),
        accessor_property(walker_get_current, walker_set_current),
    );
    obj.set(
        "parentNode".to_string(),
        JSValue::from_native_function(walker_parent_node),
    );
    obj.set(
        "firstChild".to_string(),
        JSValue::from_native_function(walker_first_child),
    );
    obj.set(
        "lastChild".to_string(),
        JSValue::from_native_function(walker_last_child),
    );
    obj.set(
        "previousSibling".to_string(),
        JSValue::from_native_function(walker_previous_sibling),
    );
    obj.set(
        "nextSibling".to_string(),
        JSValue::from_native_function(walker_next_sibling),
    );
    obj.set(
        "previousNode".to_string(),
        JSValue::from_native_function(walker_previous_node),
    );
    obj.set(
        "nextNode".to_string(),
        JSValue::from_native_function(walker_next_node),
    );
    let o = JSValue::from_object(Rc::new(RefCell::new(obj)));
    if let Some(inner) = o.as_object() {
        let ptr = obj_ptr(&inner.borrow());
        STATES.with(|s| {
            s.borrow_mut().insert(ptr, IterKind::Walker(state));
        });
    }
    o
}

fn walker_state(vm: &mut VM, args: &[JSValue]) -> Option<TreeWalkerState> {
    match this_state(vm, args)? {
        IterKind::Walker(w) => Some(w),
        _ => None,
    }
}

fn walker_get_current(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(w) = walker_state(vm, &args) else {
        return Ok(JSValue::null());
    };
    Ok(expose_node(vm, w.current).unwrap_or(JSValue::null()))
}

fn walker_set_current(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(node_val) = args.get(1).cloned() else {
        return Ok(JSValue::undefined());
    };
    if let Some(node) = dom_node(vm, &node_val) {
        set_walker_current_v(vm, args.first().unwrap_or(&JSValue::undefined()), node);
    }
    Ok(JSValue::undefined())
}

fn walker_parent_node(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(w) = walker_state(vm, &args) else {
        return Ok(JSValue::null());
    };
    let mut ancestor = match w.current.borrow().parent() {
        Some(p) => p,
        None => return Ok(JSValue::null()),
    };
    loop {
        // Only climb within the root's subtree.
        if !is_within(&ancestor, &w.root) {
            return Ok(JSValue::null());
        }
        match w.filter.passes(vm, &ancestor)? {
            None | Some(false) => {}
            Some(true) => {
                let result = expose_node(vm, Rc::clone(&ancestor)).unwrap_or(JSValue::null());
                set_walker_current_v(
                    vm,
                    args.first().unwrap_or(&JSValue::undefined()),
                    Rc::clone(&ancestor),
                );
                return Ok(result);
            }
        }
        let parent = ancestor.borrow().parent();
        match parent {
            Some(p) => ancestor = p,
            None => return Ok(JSValue::null()),
        }
    }
}

fn walker_first_child(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    traverse_children(vm, &args, true)
}

fn walker_last_child(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    traverse_children(vm, &args, false)
}

/// Spec DOM §6.2 "traverse children": iterate over walker's current node's
/// descendants filtering for the first/last visible node.
fn traverse_children(vm: &mut VM, args: &[JSValue], first: bool) -> JSResult<JSValue> {
    let Some(w) = walker_state(vm, args) else {
        return Ok(JSValue::null());
    };
    let cursor: fn(&NodeRef<HtmlNodeType>) -> Option<NodeRef<HtmlNodeType>> = if first {
        |n| n.borrow().children().first().cloned()
    } else {
        |n| n.borrow().children().last().cloned()
    };
    let mut node = match cursor(&w.current) {
        Some(n) => n,
        None => return Ok(JSValue::null()),
    };
    loop {
        let result = w.filter.filter_result(vm, &node)?;
        if result == FilterResult::Accept {
            let r = expose_node(vm, Rc::clone(&node)).unwrap_or(JSValue::null());
            set_walker_current_v(
                vm,
                args.first().unwrap_or(&JSValue::undefined()),
                Rc::clone(&node),
            );
            return Ok(r);
        }
        if result == FilterResult::Skip
            && let Some(child) = cursor(&node)
        {
            node = child;
            continue;
        }
        // find the next candidate: next/prev sibling, else climb (stop at root/current).
        loop {
            let sibling = {
                let Some(parent) = node.borrow().parent() else {
                    break;
                };
                let siblings = parent.borrow().children().to_vec();
                let idx = siblings.iter().position(|c| Rc::ptr_eq(c, &node));
                if first {
                    idx.and_then(|i| siblings.get(i + 1).cloned())
                } else {
                    idx.and_then(|i| (i > 0).then(|| Rc::clone(&siblings[i - 1])))
                }
            };
            if let Some(sib) = sibling {
                node = sib;
                break;
            }
            let parent = node.borrow().parent();
            match parent {
                Some(p) if !Rc::ptr_eq(&p, &w.root) && !Rc::ptr_eq(&p, &w.current) => node = p,
                _ => return Ok(JSValue::null()),
            }
        }
    }
}

fn walker_previous_sibling(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    traverse_siblings(vm, &args, false)
}

fn walker_next_sibling(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    traverse_siblings(vm, &args, true)
}

/// Spec DOM §6.2 "traverse siblings".
fn traverse_siblings(vm: &mut VM, args: &[JSValue], next: bool) -> JSResult<JSValue> {
    let Some(w) = walker_state(vm, args) else {
        return Ok(JSValue::null());
    };
    let mut node = Rc::clone(&w.current);
    if Rc::ptr_eq(&node, &w.root) {
        return Ok(JSValue::null());
    }
    loop {
        let mut sibling = sibling_of(&node, next);
        while let Some(sib) = sibling {
            node = sib;
            let result = w.filter.filter_result(vm, &node)?;
            if result == FilterResult::Accept {
                let r = expose_node(vm, Rc::clone(&node)).unwrap_or(JSValue::null());
                set_walker_current_v(
                    vm,
                    args.first().unwrap_or(&JSValue::undefined()),
                    Rc::clone(&node),
                );
                return Ok(r);
            }
            sibling = child_of(&node, next);
            if result == FilterResult::Reject || sibling.is_none() {
                sibling = sibling_of(&node, next);
            }
        }
        let Some(parent) = node.borrow().parent() else {
            return Ok(JSValue::null());
        };
        if Rc::ptr_eq(&parent, &w.root) {
            return Ok(JSValue::null());
        }
        if w.filter.filter_result(vm, &parent)? == FilterResult::Accept {
            return Ok(JSValue::null());
        }
        node = parent;
    }
}

/// The previous sibling of `node` (`next` false) or next sibling (`next` true).
fn sibling_of(node: &NodeRef<HtmlNodeType>, next: bool) -> Option<NodeRef<HtmlNodeType>> {
    let parent = node.borrow().parent()?;
    let siblings = parent.borrow().children().to_vec();
    let idx = siblings.iter().position(|c| Rc::ptr_eq(c, node))?;
    if next {
        siblings.get(idx + 1).cloned()
    } else {
        (idx > 0).then(|| Rc::clone(&siblings[idx - 1]))
    }
}

/// The first child of `node` (`next` true) or last child (`next` false).
fn child_of(node: &NodeRef<HtmlNodeType>, next: bool) -> Option<NodeRef<HtmlNodeType>> {
    let children = node.borrow().children().to_vec();
    if next {
        children.first().cloned()
    } else {
        children.last().cloned()
    }
}

fn walker_previous_node(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(w) = walker_state(vm, &args) else {
        return Ok(JSValue::null());
    };
    let mut node = Rc::clone(&w.current);
    while !Rc::ptr_eq(&node, &w.root) {
        let mut sibling = sibling_of(&node, false);
        while let Some(sib) = sibling {
            node = sib;
            let mut result = w.filter.filter_result(vm, &node)?;
            while result != FilterResult::Reject && !node.borrow().children().is_empty() {
                let last = node.borrow().children().last().cloned();
                if let Some(last) = last {
                    node = last;
                } else {
                    break;
                }
                result = w.filter.filter_result(vm, &node)?;
            }
            if result == FilterResult::Accept {
                let r = expose_node(vm, Rc::clone(&node)).unwrap_or(JSValue::null());
                set_walker_current_v(
                    vm,
                    args.first().unwrap_or(&JSValue::undefined()),
                    Rc::clone(&node),
                );
                return Ok(r);
            }
            sibling = sibling_of(&node, false);
        }
        let Some(parent) = node.borrow().parent() else {
            return Ok(JSValue::null());
        };
        node = parent;
        if w.filter.filter_result(vm, &node)? == FilterResult::Accept {
            let r = expose_node(vm, Rc::clone(&node)).unwrap_or(JSValue::null());
            set_walker_current_v(
                vm,
                args.first().unwrap_or(&JSValue::undefined()),
                Rc::clone(&node),
            );
            return Ok(r);
        }
        if Rc::ptr_eq(&node, &w.root) {
            break;
        }
    }
    Ok(JSValue::null())
}

fn walker_next_node(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let Some(w) = walker_state(vm, &args) else {
        return Ok(JSValue::null());
    };
    let mut node = Rc::clone(&w.current);
    let mut result = FilterResult::Accept;
    loop {
        while result != FilterResult::Reject && !node.borrow().children().is_empty() {
            let first = node.borrow().children().first().cloned();
            if let Some(first) = first {
                node = first;
            } else {
                break;
            }
            result = w.filter.filter_result(vm, &node)?;
            if result == FilterResult::Accept {
                let r = expose_node(vm, Rc::clone(&node)).unwrap_or(JSValue::null());
                set_walker_current_v(
                    vm,
                    args.first().unwrap_or(&JSValue::undefined()),
                    Rc::clone(&node),
                );
                return Ok(r);
            }
        }
        let mut sibling = None;
        let mut temporary = Rc::clone(&node);
        loop {
            if Rc::ptr_eq(&temporary, &w.root) {
                return Ok(JSValue::null());
            }
            if let Some(s) = sibling_of(&temporary, true) {
                sibling = Some(s);
                break;
            }
            let parent = temporary.borrow().parent();
            match parent {
                Some(p) => temporary = p,
                None => break,
            }
        }
        let Some(s) = sibling else {
            return Ok(JSValue::null());
        };
        node = s;
        result = w.filter.filter_result(vm, &node)?;
        if result == FilterResult::Accept {
            let r = expose_node(vm, Rc::clone(&node)).unwrap_or(JSValue::null());
            set_walker_current_v(
                vm,
                args.first().unwrap_or(&JSValue::undefined()),
                Rc::clone(&node),
            );
            return Ok(r);
        }
    }
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

pub(crate) fn make_node_iterator(
    vm: &mut VM,
    root: JSValue,
    what_to_show: u32,
    filter: Option<JSValue>,
) -> JSResult<JSValue> {
    let filter = filter.filter(|f| !f.is_null() && !f.is_undefined());
    let what_to_show = if what_to_show == 0 {
        SHOW_ALL
    } else {
        what_to_show
    };
    let Some(root_node) = dom_node(vm, &root) else {
        return Ok(JSValue::null());
    };
    let state = NodeIteratorState {
        root: Rc::clone(&root_node),
        filter: FilterSpec {
            what_to_show,
            filter: filter.clone(),
            filter_is_function: filter.as_ref().map(|f| f.is_callable()).unwrap_or(false),
        },
        reference: Rc::clone(&root_node),
        pointer_before: true,
    };
    Ok(make_iterator_object(vm, state))
}

pub(crate) fn make_tree_walker(
    vm: &mut VM,
    root: JSValue,
    what_to_show: u32,
    filter: Option<JSValue>,
) -> JSResult<JSValue> {
    let filter = filter.filter(|f| !f.is_null() && !f.is_undefined());
    let what_to_show = if what_to_show == 0 {
        SHOW_ALL
    } else {
        what_to_show
    };
    let Some(root_node) = dom_node(vm, &root) else {
        return Ok(JSValue::null());
    };
    let state = TreeWalkerState {
        root: Rc::clone(&root_node),
        filter: FilterSpec {
            what_to_show,
            filter: filter.clone(),
            filter_is_function: filter.as_ref().map(|f| f.is_callable()).unwrap_or(false),
        },
        current: Rc::clone(&root_node),
    };
    Ok(make_walker_object(vm, state))
}
