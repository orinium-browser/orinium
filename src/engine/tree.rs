//! Generic tree structure for DOM, render tree, or other hierarchical data.
//!
//! # Overview
//! - `TreeNode<T>` stores a node value, parent, and children.
//! - `Tree<T>` stores a root node and provides traversal, mapping, and searching utilities.

use std::cell::RefCell;
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
        self.children.clear();
    }

    /// Add a child node
    pub fn add_child(parent: &NodeRef<T>, child: NodeRef<T>) {
        child.borrow_mut().parent = Some(Rc::downgrade(parent));
        parent.borrow_mut().children.push(child);
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
}

/// Represents a tree with a single root node
#[derive(Clone)]
pub struct Tree<T> {
    pub root: NodeRef<T>,
}

impl<T: Clone> Tree<T> {
    /// Create a new tree with root value
    pub fn new(root_value: T) -> Self {
        Self {
            root: TreeNode::new(root_value),
        }
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
