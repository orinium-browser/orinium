//! Generic tree structure for DOM, render tree, or other hierarchical data.
//!
//! # Overview
//! - `TreeNode<T>` stores a node value, parent, and children.
//! - `Tree<T>` stores a root node and provides traversal, mapping, and searching utilities.

use std::cell::{Cell, RefCell};
use std::fmt::{self, Debug, Display, Formatter};
use std::rc::{Rc, Weak};

/// Alias for a reference-counted tree node
pub type NodeRef<T> = Rc<RefCell<TreeNode<T>>>;

/// A single tree node
#[derive(Clone)]
pub struct TreeNode<T> {
    pub value: T,
    parent: Option<Weak<RefCell<TreeNode<T>>>>,
    children: Vec<NodeRef<T>>,
}

impl<T> TreeNode<T> {
    /// Create a new node wrapped in Rc<RefCell<_>>
    pub fn new(value: T) -> NodeRef<T> {
        Rc::new(RefCell::new(Self {
            value,
            parent: None,
            children: Vec::new(),
        }))
    }

    /// Returns the parent node, if any
    pub fn parent(&self) -> Option<NodeRef<T>> {
        self.parent.as_ref().and_then(|w| w.upgrade())
    }

    /// Returns slice of child nodes
    pub fn children(&self) -> &[NodeRef<T>] {
        &self.children
    }

    /// Remove all children of this node
    pub fn clear_children(&mut self) {
        for child in &self.children {
            child.borrow_mut().parent = None;
        }
        self.children.clear();
    }

    /// Add a child node
    pub fn add_child(parent: &NodeRef<T>, child: NodeRef<T>) {
        child.borrow_mut().parent = Some(Rc::downgrade(parent));
        parent.borrow_mut().children.push(child);
    }

    /// Removes `child` from `parent`, returning the detached node when found.
    pub fn remove_child(parent: &NodeRef<T>, child: &NodeRef<T>) -> Option<NodeRef<T>> {
        let position = parent
            .borrow()
            .children
            .iter()
            .position(|candidate| Rc::ptr_eq(candidate, child))?;
        let removed = parent.borrow_mut().children.remove(position);
        removed.borrow_mut().parent = None;
        Some(removed)
    }

    /// Detaches a node from its current parent.
    pub fn detach(node: &NodeRef<T>) -> bool {
        let Some(parent) = node.borrow().parent() else {
            return false;
        };
        Self::remove_child(&parent, node).is_some()
    }

    /// Appends `child`, moving it from its current parent when necessary.
    ///
    /// Returns `false` when the operation would create a tree cycle.
    pub fn append_child(parent: &NodeRef<T>, child: NodeRef<T>) -> bool {
        if Self::is_inclusive_ancestor(&child, parent) {
            return false;
        }
        Self::detach(&child);
        Self::add_child(parent, child);
        true
    }

    /// Inserts `child` immediately before `reference`, moving it from its
    /// current parent when necessary.
    pub fn insert_before(
        parent: &NodeRef<T>,
        child: NodeRef<T>,
        reference: &NodeRef<T>,
    ) -> bool {
        if Rc::ptr_eq(&child, reference) {
            return reference
                .borrow()
                .parent()
                .is_some_and(|candidate| Rc::ptr_eq(&candidate, parent));
        }
        if Self::is_inclusive_ancestor(&child, parent) {
            return false;
        }
        if !reference
            .borrow()
            .parent()
            .is_some_and(|candidate| Rc::ptr_eq(&candidate, parent))
        {
            return false;
        }

        Self::detach(&child);
        let Some(index) = parent
            .borrow()
            .children
            .iter()
            .position(|candidate| Rc::ptr_eq(candidate, reference))
        else {
            return false;
        };
        Self::insert_child_at(parent, index, child);
        true
    }

    fn is_inclusive_ancestor(ancestor: &NodeRef<T>, node: &NodeRef<T>) -> bool {
        let mut current = Some(Rc::clone(node));
        while let Some(candidate) = current {
            if Rc::ptr_eq(ancestor, &candidate) {
                return true;
            }
            current = candidate.borrow().parent();
        }
        false
    }

    /// Insert a child at a given position
    pub fn insert_child_at(parent: &NodeRef<T>, index: usize, child: NodeRef<T>) {
        child.borrow_mut().parent = Some(Rc::downgrade(parent));
        parent.borrow_mut().children.insert(index, child);
    }

    /// Create a child with value and add it to parent
    pub fn add_child_value(parent: &NodeRef<T>, value: T) -> NodeRef<T> {
        let child = Self::new(value);
        Self::add_child(parent, Rc::clone(&child));
        child
    }

    /// Replace child at given index
    pub fn replace_child(
        parent: &NodeRef<T>,
        index: usize,
        new_child: NodeRef<T>,
    ) -> Option<NodeRef<T>> {
        let mut p = parent.borrow_mut();
        if index < p.children.len() {
            let old_child = std::mem::replace(&mut p.children[index], new_child);
            old_child.borrow_mut().parent = None;
            Some(old_child)
        } else {
            None
        }
    }

    /// Find direct children matching predicate
    pub fn find_children_by<F>(&self, predicate: F) -> Vec<NodeRef<T>>
    where
        F: Fn(&T) -> bool,
    {
        self.children
            .iter()
            .filter(|c| predicate(&c.borrow().value))
            .cloned()
            .collect()
    }

    /// Clone node (optionally deep)
    pub fn clone_node(&self, deep: bool) -> NodeRef<T>
    where
        T: Clone,
    {
        let new_node = Rc::new(RefCell::new(TreeNode {
            value: self.value.clone(),
            children: Vec::new(),
            parent: None,
        }));

        if deep {
            for child in &self.children {
                let child_clone = child.borrow().clone_node(true);
                child_clone.borrow_mut().parent = Some(Rc::downgrade(&new_node));
                new_node.borrow_mut().children.push(child_clone);
            }
        }

        new_node
    }
}

/// Represents a tree with a single root node
#[derive(Clone)]
pub struct Tree<T> {
    pub root: NodeRef<T>,
    /// Monotonic counter bumped by [`Tree::mark_dirty`] on every mutation.
    ///
    /// Layout snapshots cache the DOM and are reused while the version is
    /// unchanged, so any mutation (currently the text-input write-back, later
    /// JS DOM manipulation) must call [`Tree::mark_dirty`] to invalidate them.
    version: Cell<u64>,
}

impl<T: Clone> Tree<T> {
    /// Create a new tree with root value
    pub fn new(root_value: T) -> Self {
        Self {
            root: TreeNode::new(root_value),
            version: Cell::new(0),
        }
    }

    /// Records a DOM mutation by bumping the tree's version counter.
    pub fn mark_dirty(&self) {
        self.version.set(self.version.get() + 1);
    }

    /// The current mutation version of the tree.
    pub fn version(&self) -> u64 {
        self.version.get()
    }

    /// Recursively traverse all nodes, applying a function
    pub fn traverse<F>(&self, mut f: F)
    where
        F: FnMut(&NodeRef<T>),
    {
        fn visit<T, F>(node: &NodeRef<T>, f: &mut F)
        where
            F: FnMut(&NodeRef<T>),
        {
            f(node);
            for child in &node.borrow().children {
                visit(child, f);
            }
        }
        visit(&self.root, &mut f);
    }

    /// Map each node value to another type, returning a new Tree
    pub fn map<U, F>(&self, f: F) -> Tree<U>
    where
        F: Fn(&T) -> U,
        U: Clone,
    {
        fn map_node<T, U, F>(node: &NodeRef<T>, f: &F) -> NodeRef<U>
        where
            F: Fn(&T) -> U,
            U: Clone,
        {
            let n = node.borrow();
            let new_node = TreeNode::new(f(&n.value));
            for child in &n.children {
                let mapped_child = map_node(child, f);
                TreeNode::add_child(&new_node, mapped_child);
            }
            new_node
        }

        Tree {
            root: map_node(&self.root, &f),
            version: Cell::new(0),
        }
    }

    /// Map using the NodeRef itself (for access to parent/children)
    pub fn map_with_node<U, F>(&self, f: F) -> Tree<U>
    where
        F: Fn(&NodeRef<T>) -> U,
        U: Clone,
    {
        fn map_node<T, U, F>(node: &NodeRef<T>, f: &F) -> NodeRef<U>
        where
            F: Fn(&NodeRef<T>) -> U,
            U: Clone,
        {
            let new_node = TreeNode::new(f(node));
            for child in &node.borrow().children {
                let mapped_child = map_node(child, f);
                TreeNode::add_child(&new_node, mapped_child);
            }
            new_node
        }

        Tree {
            root: map_node(&self.root, &f),
            version: Cell::new(0),
        }
    }

    /// Find all nodes in the tree that satisfy a predicate
    pub fn find_all<F>(&self, predicate: F) -> Vec<NodeRef<T>>
    where
        F: Fn(&T) -> bool,
    {
        let mut result = Vec::new();
        self.traverse(|node| {
            if predicate(&node.borrow().value) {
                result.push(Rc::clone(node));
            }
        });
        result
    }
}

impl<T: Clone + Debug> Display for Tree<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        fn fmt_node<T: Clone + Debug>(
            node: &NodeRef<T>,
            f: &mut Formatter<'_>,
            prefix: &str,
            is_last: bool,
        ) -> fmt::Result {
            let n = node.borrow();
            let connector = if prefix.is_empty() {
                ""
            } else if is_last {
                "└── "
            } else {
                "├── "
            };
            writeln!(f, "{}{}{:?}", prefix, connector, n.value)?;
            let child_count = n.children.len();
            for (i, child) in n.children.iter().enumerate() {
                let mut new_prefix = prefix.to_string();
                new_prefix.push_str(if is_last { "    " } else { "│   " });
                fmt_node(child, f, &new_prefix, i == child_count - 1)?;
            }
            Ok(())
        }
        fmt_node(&self.root, f, "", true)
    }
}

impl<T: Clone + Debug> Debug for Tree<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_child_reparents_and_prevents_cycles() {
        let root = TreeNode::new("root");
        let first = TreeNode::new("first");
        let second = TreeNode::new("second");
        TreeNode::add_child(&root, Rc::clone(&first));
        TreeNode::add_child(&first, Rc::clone(&second));

        assert!(TreeNode::append_child(&root, Rc::clone(&second)));
        assert!(first.borrow().children().is_empty());
        assert!(Rc::ptr_eq(&second.borrow().parent().unwrap(), &root));
        assert!(!TreeNode::append_child(&second, Rc::clone(&root)));
    }

    #[test]
    fn detach_and_clear_children_reset_parent_links() {
        let root = TreeNode::new("root");
        let first = TreeNode::new("first");
        let second = TreeNode::new("second");
        TreeNode::add_child(&root, Rc::clone(&first));
        TreeNode::add_child(&root, Rc::clone(&second));

        assert!(TreeNode::detach(&first));
        assert!(first.borrow().parent().is_none());
        root.borrow_mut().clear_children();
        assert!(second.borrow().parent().is_none());
    }

    #[test]
    fn insert_before_moves_nodes_and_preserves_order() {
        let root = TreeNode::new("root");
        let first = TreeNode::new("first");
        let second = TreeNode::new("second");
        let moving = TreeNode::new("moving");
        TreeNode::add_child(&root, Rc::clone(&first));
        TreeNode::add_child(&root, Rc::clone(&second));
        TreeNode::add_child(&first, Rc::clone(&moving));

        assert!(TreeNode::insert_before(
            &root,
            Rc::clone(&moving),
            &second
        ));
        let children = root.borrow().children().to_vec();
        assert!(Rc::ptr_eq(&children[0], &first));
        assert!(Rc::ptr_eq(&children[1], &moving));
        assert!(Rc::ptr_eq(&children[2], &second));
        assert!(first.borrow().children().is_empty());
    }
}
