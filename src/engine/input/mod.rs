use super::layouter::types::{InfoNode, NodeKind};
use ui_layout::LayoutNode;

/// ヒットしたノード情報
pub struct HitItem<'a> {
    pub layout: &'a LayoutNode,
    pub info: &'a InfoNode,
}

/// ヒットパス（子→親の順）
pub type HitPath<'a> = Vec<HitItem<'a>>;

/// x, y: グローバル座標
pub fn hit_test<'a>(layout: &'a LayoutNode, info: &'a InfoNode, x: f32, y: f32) -> HitPath<'a> {
    let rect = layout.box_model.padding_box;

    // 1. rect 外ならヒットなし
    if x < rect.x || y < rect.y || x > rect.x + rect.width || y > rect.y + rect.height {
        return Vec::new();
    }

    // 2. ローカル座標に変換（スクロールオフセット考慮）
    let mut local_x = x - rect.x;
    let mut local_y = y - rect.y;

    if let NodeKind::Container {
        scroll_offset_x,
        scroll_offset_y,
        ..
    } = &info.kind
    {
        local_x += *scroll_offset_x;
        local_y += *scroll_offset_y;
    }

    // 3. 子ノードを前面から探索（rev で重なり優先）
    for (child_layout, child_info) in layout.children.iter().zip(&info.children).rev() {
        let mut path = hit_test(child_layout, child_info, local_x, local_y);
        if !path.is_empty() {
            // 子がヒット → 自分を末尾に追加
            path.push(HitItem { layout, info });
            return path;
        }
    }

    // 4. 子ノードに当たらなければ自分がヒット
    vec![HitItem { layout, info }]
}
