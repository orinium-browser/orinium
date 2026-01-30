//! Minimal diff utilities for layout and render-info trees.
#![allow(dead_code)]

use crate::engine::layouter::types::InfoNode;
use ui_layout::LayoutNode;

/// Result of a diff operation.
pub enum DiffResult {
    /// Reuse the previous node as-is.
    Reuse,

    /// Replace the previous node.
    Replace,
}

/* =========================
LayoutNode diff
========================= */

pub fn diff_layout(old: &LayoutNode, new: &LayoutNode) -> DiffResult {
    if old.style != new.style {
        return DiffResult::Replace;
    }

    if old.children.len() != new.children.len() {
        return DiffResult::Replace;
    }

    for (old_child, new_child) in old.children.iter().zip(new.children.iter()) {
        if matches!(diff_layout(old_child, new_child), DiffResult::Replace) {
            return DiffResult::Replace;
        }
    }

    DiffResult::Reuse
}

/* =========================
InfoNode diff
========================= */

pub fn diff_info(old: &InfoNode, new: &InfoNode) -> DiffResult {
    if old.kind != new.kind {
        return DiffResult::Replace;
    }

    if old.children.len() != new.children.len() {
        return DiffResult::Replace;
    }

    for (old_child, new_child) in old.children.iter().zip(new.children.iter()) {
        if matches!(diff_info(old_child, new_child), DiffResult::Replace) {
            return DiffResult::Replace;
        }
    }

    DiffResult::Reuse
}
