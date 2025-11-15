#[derive(Debug, Clone, Copy)]
pub struct ScrollBar {
    /// スクロールバーのトラックの幅（ピクセル）
    pub width: f32,
    /// ビューポート端からのマージン（ピクセル）
    pub margin: f32,
    /// 最小高さ（ピクセル）
    pub min_thumb: f32,
    /// 色（RGBA）
    pub color: [f32; 4],
}

impl Default for ScrollBar {
    fn default() -> Self {
        Self {
            width: 8.0,
            margin: 4.0,
            min_thumb: 20.0,
            color: [0.18, 0.18, 0.18, 0.7],
        }
    }
}

impl ScrollBar {
    pub fn new() -> Self {
        Self::default()
    }

    /// スクリーン座標でサムの矩形を計算する (x1, y1, x2, y2)。
    /// (x1,y1) は左上、(x2,y2) は右下。
    /// コンテンツがビューポートに収まる場合はNone返してスクロールバーを非表示にさせる
    pub fn thumb_rect(
        &self,
        viewport_width: f32,
        viewport_height: f32,
        content_height: f32,
        scroll_y: f32,
    ) -> Option<(f32, f32, f32, f32)> {
        if content_height <= viewport_height || viewport_height <= 0.0 {
            return None;
        }

        let vw = viewport_width;
        let vh = viewport_height;
        let bar_w = self.width;
        let margin = self.margin;

        // ビューポート／コンテンツ比に応じた高さを計算
        let thumb_h = (vh * (vh / content_height)).max(self.min_thumb).min(vh - 2.0 * margin);

        // トラック内で上端が移動できる最大距離
        let max_thumb_top = (vh - 2.0 * margin - thumb_h).max(0.0);

        // scroll_y (0 .. content_height - vh) をトラック上の位置 (0 .. max_thumb_top) にマッピング
        let denom = (content_height - vh).max(1.0);
        let ratio = (scroll_y / denom).clamp(0.0, 1.0);
        let thumb_top = margin + ratio * max_thumb_top;

        let x1 = vw - margin - bar_w;
        let x2 = vw - margin;
        let y1 = thumb_top;
        let y2 = thumb_top + thumb_h;

        Some((x1, y1, x2, y2))
    }

    /// 画面座標の点 (px,py) が矩形に当たる場合にtrue
    pub fn hit_test_thumb(
        &self,
        viewport_width: f32,
        viewport_height: f32,
        content_height: f32,
        scroll_y: f32,
        px: f32,
        py: f32,
    ) -> bool {
        if let Some((x1, y1, x2, y2)) = self.thumb_rect(viewport_width, viewport_height, content_height, scroll_y) {
            px >= x1 && px <= x2 && py >= y1 && py <= y2
        } else {
            false
        }
    }
}