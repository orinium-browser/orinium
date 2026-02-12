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
    // layout_boxes が空なら何もヒットしない
    if layout.layout_boxes.is_empty() {
        return Vec::new();
    }

    for box_model in layout
        .layout_boxes
        .iter()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        // 後ろの box が前面
        let rect = box_model.padding_box;

        // 1. rect 外なら次の box へ
        if x < rect.x || y < rect.y || x > rect.x + rect.width || y > rect.y + rect.height {
            continue;
        }

        // 2. ローカル座標に変換（スクロールオフセット考慮）
        let mut local_x = x - box_model.content_box.x;
        let mut local_y = y - box_model.content_box.y;

        if let NodeKind::Container {
            scroll_offset_x,
            scroll_offset_y,
            ..
        } = &info.kind
        {
            local_x += *scroll_offset_x;
            local_y += *scroll_offset_y;
        }

        // 3. 子ノードを前面から探索
        for (child_layout, child_info) in layout.children.iter().zip(&info.children).rev() {
            let mut path = hit_test(child_layout, child_info, local_x, local_y);
            if !path.is_empty() {
                // 子がヒット → 自分を末尾に追加
                path.push(HitItem { layout, info });
                return path;
            }
        }

        // 4. 子ノードに当たらなければこの box がヒット
        return vec![HitItem { layout, info }];
    }

    // どの box にもヒットしなかった
    Vec::new()
}
