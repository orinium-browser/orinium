use super::layouter::{InfoNode, NodeKind};
use ui_layout::LayoutNode;

pub struct HitItem<'a> {
    pub layout: &'a LayoutNode,
    pub info: &'a InfoNode,
}

pub type HitPath<'a> = Vec<HitItem<'a>>;

pub fn hit_test<'a>(layout: &'a LayoutNode, info: &'a InfoNode, x: f32, y: f32) -> HitPath<'a> {
    let rect = layout.rect;

    // 1. rect 外なら何も返さない
    if x < rect.x || y < rect.y || x > rect.x + rect.width || y > rect.y + rect.height {
        return Vec::new();
    }

    // 2. ローカル座標へ
    let mut local_x = x - rect.x;
    let mut local_y = y - rect.y;

    if let NodeKind::Container {
        scroll_offset_x,
        scroll_offset_y,
        ..
    } = &info.kind
    {
        local_x -= *scroll_offset_x;
        local_y -= *scroll_offset_y;
    }

    // 3. 子を前面から探索
    for (child_layout, child_info) in layout.children.iter().zip(&info.children).rev() {
        let mut path = hit_test(child_layout, child_info, local_x, local_y);
        if !path.is_empty() {
            // 子がヒット → 自分を末尾に追加
            path.push(HitItem { layout, info });
            return path;
        }
    }

    // 4. 子が無い or 当たらない → 自分が hit
    vec![HitItem { layout, info }]
}
