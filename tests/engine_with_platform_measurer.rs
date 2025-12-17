use std::rc::Rc;
use orinium_browser::engine::html::HtmlNodeType;
use orinium_browser::engine::renderer::RenderTree;
use orinium_browser::engine::styler::computed_tree::{ComputedStyle, ComputedStyleNode, ComputedTree};
use orinium_browser::engine::styler::style_tree::Style;
use orinium_browser::engine::tree::TreeNode;
use orinium_browser::platform::renderer::text_measurer::PlatformTextMeasurer;

#[test]
fn engine_layout_with_platform_measurer() {
    let candidates = [
        "C:\\Windows\\Fonts\\arial.ttf",
        "C:\\Windows\\Fonts\\segoeui.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/System/Library/Fonts/SFNSDisplay.ttf",
    ];

    // 存在するフォントパスを探す
    let mut font_path = None;
    for p in candidates.iter() {
        if std::path::Path::new(p).exists() {
            font_path = Some(p.to_string());
            break;
        }
    }

    // フォントが見つからなければテストをスキップ
    let path = match font_path {
        Some(p) => p,
        None => {
            eprintln!("skipping engine_with_platform_measurer test: no system font found");
            return;
        }
    };

    // テキストノードを作成
    let html_text_node = TreeNode::new(HtmlNodeType::Text("Platform test".to_string()));
    let html_weak = Rc::downgrade(&html_text_node);
    // デフォルトスタイルを計算してComputedStyleNodeを作成
    let computed = ComputedStyle::compute(Style::default());
    let computed_node = ComputedStyleNode { html: html_weak.clone(), computed: Some(computed) };

    // ルートのドキュメントノードとComputedTreeを準備
    let root_html_node = TreeNode::new(HtmlNodeType::Document);
    let root_weak = Rc::downgrade(&root_html_node);
    let root_computed = ComputedStyle::compute(Style::default());
    let tree = ComputedTree::new(ComputedStyleNode { html: root_weak, computed: Some(root_computed) });
    // ルートに子ノード（テキスト）を追加
    let _child = TreeNode::add_child_value(&tree.root, computed_node);

    // ComputedTree から RenderTree を生成
    let mut render_tree = RenderTree::from_computed_tree(&tree);

    // フォントファイルを読み込み、PlatformTextMeasurer を作成
    let bytes = std::fs::read(path).expect("read font");
    let pm = PlatformTextMeasurer::from_bytes("sys", bytes).expect("create measurer");

    // 測定器を使ってレイアウトを実行
    render_tree.layout_with_measurer(&pm);

    // レンダーツリーのルートを検査して、子のサイズが測定されていることを確認
    let root_node = render_tree.root.borrow();
    match &root_node.value.kind {
        // ルートはScrollableであることを期待
        orinium_browser::engine::renderer::NodeKind::Scrollable { tree: inner_tree, .. } => {
            // 内部ツリーの子を取得
            let children = inner_tree.root.borrow().children().clone();
            assert!(!children.is_empty(), "no children in inner render tree");
            let first = &children[0];
            let rn = &first.borrow().value;
            // 測定結果が正の値であることを確認
            assert!(rn.width > 0.0, "measured width should be > 0");
            assert!(rn.height > 0.0, "measured height should be > 0");
        }
        _ => panic!("expected Scrollable root in RenderTree"),
    }
}