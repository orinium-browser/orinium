use orinium_browser::engine::html::HtmlNodeType;
use orinium_browser::engine::renderer::{NodeKind, RenderTree};
use orinium_browser::engine::share::text::{
    TextMeasureError, TextMeasurement, TextMeasurementRequest, TextMeasurer,
};
use orinium_browser::engine::styler::computed_tree::{
    ComputedStyle, ComputedStyleNode, ComputedTree,
};
use orinium_browser::engine::styler::style_tree::Style;
use orinium_browser::engine::tree::TreeNode;
use std::rc::Rc;

struct MockMeasurer {}
impl TextMeasurer for MockMeasurer {
    fn measure(&self, _req: &TextMeasurementRequest) -> Result<TextMeasurement, TextMeasureError> {
        Ok(TextMeasurement {
            width: 50.0,
            height: 12.0,
            baseline: 9.0,
            glyphs: None,
        })
    }
}

#[test]
fn render_tree_uses_measurer() {
    let html_text_node = TreeNode::new(HtmlNodeType::Text("hello".to_string()));
    let html_weak = Rc::downgrade(&html_text_node);

    let computed = ComputedStyle::compute(Style::default());
    let computed_node = ComputedStyleNode {
        html: html_weak.clone(),
        computed: Some(computed),
    };

    let root_html_node = TreeNode::new(HtmlNodeType::Document);
    let root_weak = Rc::downgrade(&root_html_node);

    assert!(
        html_weak.upgrade().is_some(),
        "html_text_node weak upgrade failed"
    );
    assert!(
        root_weak.upgrade().is_some(),
        "root_html_node weak upgrade failed"
    );

    let root_computed = ComputedStyle::compute(Style::default());
    let tree = ComputedTree::new(ComputedStyleNode {
        html: root_weak,
        computed: Some(root_computed),
    });
    let _child = TreeNode::add_child_value(&tree.root, computed_node);

    let mut render_tree = RenderTree::from_computed_tree(&tree);

    let meas = MockMeasurer {};
    render_tree.layout_with_measurer(&meas);

    let root_node = render_tree.root.borrow();
    if let NodeKind::Scrollable {
        tree: inner_tree, ..
    } = &root_node.value.kind
    {
        let children = inner_tree.root.borrow().children().clone();
        assert!(!children.is_empty(), "no children in inner render tree");
        let first = &children[0];
        let rn = &first.borrow().value;
        assert!((rn.width - 50.0).abs() < 1e-6, "width not set by measurer");
        assert!(
            (rn.height - 12.0).abs() < 1e-6,
            "height not set by measurer"
        );
    } else {
        panic!("expected Scrollable root in RenderTree");
    }
}

// 汚いテストコードだこと
