//! DomTreeやRenderTreeで使用する汎用ツリー構造の実装
//!
//! # 概要
//! `TreeNode` はノードの値・子ノード・親ノードを持ち、  
//! `Tree` はルートノードを保持する汎用的な木構造を表します。
//!
//! DOMツリー、レンダーツリーなどに再利用可能です。

use std::cell::RefCell;
use std::fmt::{self, Debug, Display, Formatter};
use std::rc::{Rc, Weak};

/// ツリーノード
#[derive(Clone)]
pub struct TreeNode<T> {
    pub value: T,
    children: Vec<Rc<RefCell<TreeNode<T>>>>,
    parent: Option<Weak<RefCell<TreeNode<T>>>>,
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

    pub fn children(&self) -> &Vec<Rc<RefCell<TreeNode<T>>>> {
        &self.children
    }

    pub fn parent(&self) -> Option<Rc<RefCell<TreeNode<T>>>> {
        self.parent.as_ref().and_then(|weak| weak.upgrade())
    }

    /// 子ノードを追加
    pub fn add_child(parent: &Rc<RefCell<Self>>, child: Rc<RefCell<Self>>) {
        child.borrow_mut().parent = Some(Rc::downgrade(parent));
        parent.borrow_mut().children.push(child);
    }

    pub fn add_child_at_first(parent: &Rc<RefCell<Self>>, child: Rc<RefCell<Self>>) {
        child.borrow_mut().parent = Some(Rc::downgrade(parent));
        parent.borrow_mut().children.insert(0, child);
    }

    /// 子ノードを作ってそのまま追加する
    pub fn add_child_value(parent: &Rc<RefCell<Self>>, value: T) -> Rc<RefCell<Self>> {
        let child = TreeNode::new(value);
        TreeNode::add_child(parent, Rc::clone(&child));
        child
    }

    /// 指定条件で子ノードを探索
    pub fn find_children_by<F>(&self, predicate: F) -> Vec<Rc<RefCell<TreeNode<T>>>>
    where
        F: Fn(&T) -> bool,
    {
        self.children
            .iter()
            .filter(|child| predicate(&child.borrow().value))
            .cloned()
            .collect()
    }
}

impl<T: Debug + Clone> Display for TreeNode<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        fmt_tree_node(&Rc::new(RefCell::new(self.clone())), f, &[])
    }
}

impl<T: Clone + Debug> Debug for TreeNode<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        // Display実装を流用して文字列化
        write!(f, "{}", self)
    }
}

/// ツリー本体
#[derive(Debug, Clone)]
pub struct Tree<T: Clone> {
    pub root: Rc<RefCell<TreeNode<T>>>,
}

impl<T: Clone> Tree<T> {
    pub fn new(root_value: T) -> Self {
        Tree {
            root: TreeNode::new(root_value),
        }
    }

    /// ツリーを再帰的に走査して処理
    pub fn traverse<F>(&self, f: &mut F)
    where
        F: FnMut(&Rc<RefCell<TreeNode<T>>>),
    {
        fn visit<T, F>(node: &Rc<RefCell<TreeNode<T>>>, f: &mut F)
        where
            F: FnMut(&Rc<RefCell<TreeNode<T>>>),
        {
            f(node);
            for child in &node.borrow().children {
                visit(child, f);
            }
        }
        visit(&self.root, f);
    }

    /// 各ノードの値を別の型に変換して新しいツリーを作成
    pub fn map<U, F>(&self, f: &F) -> Tree<U>
    where
        F: Fn(&T) -> U,
        U: Clone,
    {
        #[rustfmt::skip]
        fn map_node<T, U, F>(
            node: &Rc<RefCell<TreeNode<T>>>,
            f: &F,
        ) -> Rc<RefCell<TreeNode<U>>>
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
            root: map_node(&self.root, f),
        }
    }

    pub fn map_with_node<U, F>(&self, f: &F) -> Tree<U>
    where
        F: Fn(&Rc<RefCell<TreeNode<T>>>) -> U,
        U: Clone,
    {
        #[rustfmt::skip]
        fn map_node<T, U, F>(
            node: &Rc<RefCell<TreeNode<T>>>,
            f: &F,
        ) -> Rc<RefCell<TreeNode<U>>>
        where
            F: Fn(&Rc<RefCell<TreeNode<T>>>) -> U,
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
            root: map_node(&self.root, f),
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
