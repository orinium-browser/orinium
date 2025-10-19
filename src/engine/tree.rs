//! DomTreeやRenderTreeで使用する汎用ツリー構造の実装
//!
//! # 概要
//! `TreeNode` はノードの値・子ノード・親ノードを持ち、  
//! `Tree` はルートノードを保持する汎用的な木構造を表します。
//!
//! DOMツリー、レンダーツリーなどに再利用可能です。
//!
//! # 例
//! ```
//! use std::rc::Rc;
//! use std::cell::RefCell;
//! use orinium::engine::tree::{Tree, TreeNode};
//!
//! #[derive(Debug, Clone, PartialEq, Eq)]
//! enum NodeType {
//!     Document,
//!     Element(&'static str),
//!     Text(&'static str),
//! }
//!
//! // ツリーを構築
//! let tree = Tree::new(NodeType::Document);
//!
//! let html = TreeNode::new(NodeType::Element("html"));
//! TreeNode::add_child(&tree.root, Rc::clone(&html));
//!
//! let head = TreeNode::new(NodeType::Element("head"));
//! let body = TreeNode::new(NodeType::Element("body"));
//! TreeNode::add_child(&html, Rc::clone(&head));
//! TreeNode::add_child(&html, Rc::clone(&body));
//!
//! let title = TreeNode::new(NodeType::Element("title"));
//! let text = TreeNode::new(NodeType::Text("Hello"));
//! TreeNode::add_child(&title, Rc::clone(&text));
//! TreeNode::add_child(&head, Rc::clone(&title));
//!
//! println!("{}", tree);
//!
//! // ツリー構造の検証
//! assert_eq!(html.borrow().parent.as_ref().unwrap().borrow().value, NodeType::Document);
//! assert_eq!(body.borrow().parent.as_ref().unwrap().borrow().value, NodeType::Element("html"));
//! assert_eq!(text.borrow().parent.as_ref().unwrap().borrow().value, NodeType::Element("title"));
//! ```
//!
//! 出力例：
//! ```text
//! Document
//! └── Element("html")
//!     ├── Element("head")
//!     │   └── Element("title")
//!     │       └── Text("Hello")
//!     └── Element("body")
//! ```

use std::cell::RefCell;
use std::fmt::{self, Debug, Display, Formatter};
use std::rc::Rc;

/// ツリーノード
#[derive(Clone, PartialEq, Eq)]
pub struct TreeNode<T> {
    pub value: T,
    pub children: Vec<Rc<RefCell<TreeNode<T>>>>,
    pub parent: Option<Rc<RefCell<TreeNode<T>>>>,
}

impl<T> TreeNode<T> {
    /// 新しいノードを作成
    pub fn new(value: T) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(TreeNode {
            value,
            children: Vec::new(),
            parent: None,
        }))
    }

    /// 子ノードを追加
    pub fn add_child(parent: &Rc<RefCell<Self>>, child: Rc<RefCell<Self>>) {
        child.borrow_mut().parent = Some(Rc::clone(parent));
        parent.borrow_mut().children.push(child);
    }
}

/// ツリー本体
#[derive(Clone)]
pub struct Tree<T> {
    pub root: Rc<RefCell<TreeNode<T>>>,
}

impl<T> Tree<T> {
    pub fn new(root_value: T) -> Self {
        Tree {
            root: TreeNode::new(root_value),
        }
    }
}

impl<T: Debug + Clone> Display for Tree<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        fmt_tree_node(&self.root, f, &[])
    }
}

/// 再帰的にツリーを表示するヘルパー関数
fn fmt_tree_node<T: Debug + Clone>(
    node: &std::rc::Rc<std::cell::RefCell<TreeNode<T>>>,
    f: &mut Formatter<'_>,
    ancestors_last: &[bool],
) -> fmt::Result {
    let n = node.borrow();

    let is_last = *ancestors_last.last().unwrap_or(&true);
    let connector = if ancestors_last.is_empty() {
        ""
    } else if is_last {
        "└── "
    } else {
        "├── "
    };

    let mut prefix = String::new();
    for &ancestor_last in &ancestors_last[..ancestors_last.len().saturating_sub(1)] {
        prefix.push_str(if ancestor_last { "    " } else { "│   " });
    }

    writeln!(f, "{}{}{:?}", prefix, connector, n.value)?;

    let child_count = n.children.len();
    for (i, child) in n.children.iter().enumerate() {
        let child_is_last = i == child_count - 1;
        let mut new_ancestors = ancestors_last.to_vec();
        new_ancestors.push(child_is_last);
        fmt_tree_node(child, f, &new_ancestors)?;
    }

    Ok(())
}
