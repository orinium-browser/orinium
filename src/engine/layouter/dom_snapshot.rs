//! Owned, thread-safe snapshot of the DOM.
//!
//! The live DOM tree is built with `Rc<RefCell<TreeNode>>`, which is not
//! [`Send`]. To build layout off the UI thread, we clone the tree into an
//! arena of owned nodes. Pre-order (document-order) node ids index the arena,
//! so the snapshot can be moved to a worker thread and the builder walks it
//! exactly as it walked the `Rc` tree.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use crate::engine::html::HtmlNodeType;
use crate::engine::tree::{NodeRef, TreeNode};

/// Pre-order (document-order) id of a node in the snapshot arena.
pub type NodeId = u32;

/// A single owned snapshot node.
#[derive(Debug, Clone)]
pub struct SnapNode {
    pub kind: HtmlNodeType,
    pub children: Vec<NodeId>,
}

/// An owned snapshot of a DOM subtree.
///
/// Nodes are stored in pre-order so `id == index`. `roots` lists the ids of
/// top-level nodes (a snapshot has exactly one root in practice).
#[derive(Debug, Default)]
pub struct DomSnapshot {
    nodes: Vec<SnapNode>,
    roots: Vec<NodeId>,
}

impl DomSnapshot {
    /// Builds a snapshot of `root` and the matching live-node references.
    ///
    /// `dom_refs[i]` is the live DOM node for snapshot node id `i`. The refs
    /// are kept separate (and are **not** `Send`) so the UI thread can apply
    /// attribute write-backs after the worker finished building.
    pub fn from_tree(
        root: &NodeRef<HtmlNodeType>,
    ) -> (Self, Vec<Weak<RefCell<TreeNode<HtmlNodeType>>>>) {
        let mut snapshot = DomSnapshot::default();
        let mut dom_refs: Vec<Weak<RefCell<TreeNode<HtmlNodeType>>>> = Vec::new();
        let id = snapshot.walk(root, &mut dom_refs);
        snapshot.roots.push(id);
        (snapshot, dom_refs)
    }

    fn walk(
        &mut self,
        node: &NodeRef<HtmlNodeType>,
        dom_refs: &mut Vec<Weak<RefCell<TreeNode<HtmlNodeType>>>>,
    ) -> NodeId {
        let id = self.nodes.len() as NodeId;
        self.nodes.push(SnapNode {
            kind: node.borrow().value.clone(),
            children: Vec::new(),
        });
        dom_refs.push(Rc::downgrade(node));
        let children: Vec<NodeId> = node
            .borrow()
            .children()
            .iter()
            .map(|child| self.walk(child, dom_refs))
            .collect();
        self.nodes[id as usize].children = children;
        id
    }

    /// Root node ids (always a single root in practice).
    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }

    /// The node with the given id.
    pub fn node(&self, id: NodeId) -> &SnapNode {
        &self.nodes[id as usize]
    }

    /// Child ids of the node with the given id.
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[id as usize].children
    }

    /// Concatenated text content of a node, including descendants.
    pub fn inner_text(&self, id: NodeId) -> String {
        let node = &self.nodes[id as usize];
        match &node.kind {
            HtmlNodeType::Text(content) => content.clone(),
            HtmlNodeType::Element { .. } | HtmlNodeType::Document => node
                .children
                .iter()
                .map(|&child| self.inner_text(child))
                .collect(),
            _ => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::html::parser::DomTree;
    use crate::engine::html::parser::Parser as HtmlParser;

    fn tree(html: &str) -> DomTree {
        HtmlParser::new(html).parse()
    }

    #[test]
    fn snapshot_is_built_in_preorder_and_is_send() {
        let dom = tree("<html><body><div><p>hi</p></div><button>ok</button></body></html>");
        let (snapshot, _dom_refs) = DomSnapshot::from_tree(&dom.root);
        let root = snapshot.roots()[0];

        // The parser wraps the tree in a Document node.
        let doc = snapshot.node(root);
        assert_eq!(doc.kind.tag_name(), None);

        // root html has body as its only element child
        let html = doc.children[0];
        let html_node = snapshot.node(html);
        assert_eq!(html_node.kind.tag_name(), Some("html"));
        assert_eq!(html_node.children.len(), 1);

        // Pre-order: body is the first child of html.
        let body = html_node.children[0];
        assert_eq!(snapshot.node(body).kind.tag_name(), Some("body"));

        // A snapshot must be movable to another thread.
        std::thread::spawn(move || {
            let _ = snapshot.inner_text(root);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn inner_text_concatenates_descendants() {
        let dom = tree("<div><p>hello</p><p>world</p></div>");
        let (snapshot, _) = DomSnapshot::from_tree(&dom.root);
        let root = snapshot.roots()[0];
        assert_eq!(snapshot.inner_text(root), "helloworld");
    }

    #[test]
    fn dom_refs_map_node_id_to_live_node() {
        let dom = tree("<input value='a'>");
        let (snapshot, dom_refs) = DomSnapshot::from_tree(&dom.root);
        let root = snapshot.roots()[0];

        // Find the input's snapshot id anywhere below the Document root.
        fn find(snapshot: &DomSnapshot, id: NodeId, tag: &str) -> Option<NodeId> {
            if snapshot.node(id).kind.tag_name() == Some(tag) {
                return Some(id);
            }
            snapshot
                .children(id)
                .iter()
                .find_map(|&c| find(snapshot, c, tag))
        }
        let input_id = find(&snapshot, root, "input").unwrap();

        let live = dom_refs[input_id as usize].upgrade().unwrap();
        assert_eq!(live.borrow().value.tag_name(), Some("input"));
    }
}
