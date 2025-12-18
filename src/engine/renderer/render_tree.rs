use super::render_node::{NodeKind, RenderTree};
use crate::engine::styler::computed_tree::ComputedTree;

impl RenderTree {
    pub fn set_root_size(&mut self, w: f32, h: f32) {
        let mut root = self.root.borrow_mut();
        if let NodeKind::Scrollable { .. } = root.value.kind {
            root.value.width = w;
            root.value.height = h;
        }
    }

    /// ComputedTree から RenderTree を生成
    pub fn from_computed_tree(tree: &ComputedTree) -> RenderTree {
        // Delegate to ComputedTree's layout routine with fallback measurer.
        let fallback = crate::engine::bridge::text::EngineFallbackTextMeasurer::default();
        tree.layout_with_measurer(&fallback, 0.0, 0.0)
    }
}
