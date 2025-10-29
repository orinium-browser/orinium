//! DomTreeやRenderTreeで使用する汎用ツリー構造の実装
//!
//! # 概要
//! `TreeNode` はノードの値・子ノード・親ノードを持ち、  
//! `Tree` はルートノードを保持する汎用的な木構造を表します。
//!
//! DOMツリー、レンダーツリーなどに再利用可能です。

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

    #[cfg(debug_assertions)]
    pub fn from_tree_node(root: Rc<RefCell<TreeNode<T>>>) -> Self {
        Tree { root }
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
