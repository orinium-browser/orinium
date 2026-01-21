//! Minimal diff utilities for layout and render-info trees.
//!
//! Design goals:
//! - Deterministic
//! - Thread-safe (pure functions)
//! - Cheap to run
//!
//! Non-goals:
//! - Smart reconciliation
//! - Node reordering
//! - Partial property updates

use crate::engine::layouter::types::InfoNode;
use ui_layout::LayoutNode;

/// Result of a diff operation.
pub enum DiffResult<T> {
    /// Reuse the previous node as-is.
    Reuse,

    /// Replace the previous node with a new one.
    Replace(T),
}

/* =========================
LayoutNode diff
========================= */

pub fn diff_layout(old: &LayoutNode, new: LayoutNode) -> DiffResult<LayoutNode> {
    // If node-level properties differ, replace entirely.
    if old.style != new.style {
        return DiffResult::Replace(new);
    }

    if old.children.len() != new.children.len() {
        return DiffResult::Replace(new);
    }

    // Check children recursively
    for (old_child, new_child) in old.children.iter().zip(new.children.iter()) {
        match diff_layout(old_child, new_child.clone()) {
            DiffResult::Reuse => {}
            DiffResult::Replace(_) => return DiffResult::Replace(new),
        }
    }

    DiffResult::Reuse
}

/* =========================
InfoNode diff
========================= */

pub fn diff_info(old: &InfoNode, new: InfoNode) -> DiffResult<InfoNode> {
    if old.kind != new.kind {
        return DiffResult::Replace(new);
    }

    if old.children.len() != new.children.len() {
        return DiffResult::Replace(new);
    }

    for (old_child, new_child) in old.children.iter().zip(new.children.iter()) {
        match diff_info(old_child, new_child.clone()) {
            DiffResult::Reuse => {}
            DiffResult::Replace(_) => return DiffResult::Replace(new),
        }
    }

    DiffResult::Reuse
}
