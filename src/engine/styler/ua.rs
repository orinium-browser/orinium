//! UA（User-Agent）デフォルトスタイル
//!
//! html要素ごとの最小限の display / margin / padding を定義する。

use super::style::Style;
use crate::engine::css::values::{
    // Border, Color,
    Display,
    Length,
};
use crate::engine::html::parser::HtmlNodeType;
use crate::engine::html::util;

/// HTML ノードに対するデフォルト Style を返す
pub fn default_style_for(node: &HtmlNodeType) -> Style {
    let mut s = Style {
        display: Some(Display::Inline),
        ..Default::default()
    };

    let tag_name = node.tag_name().unwrap_or("".to_string());
    let tag_name = tag_name.as_str();

    match tag_name {
        // 文書ルート
        "html" => {
            s.display = Some(Display::Block);
        }
        "body" => {
            s.display = Some(Display::Block);
            // ブラウザのデフォルト body margin は一般に 8px 前後
            s.margin_top = Some(Length::Px(8.0));
            s.margin_right = Some(Length::Px(8.0));
            s.margin_bottom = Some(Length::Px(8.0));
            s.margin_left = Some(Length::Px(8.0));
        }

        // 見出しはブロックで上下に余白
        "h1" => {
            s.display = Some(Display::Block);
            s.margin_top = Some(Length::Px(21.0));
            s.margin_bottom = Some(Length::Px(14.0));
            s.font_size = Some(Length::Px(32.0));
        }
        "h2" => {
            s.display = Some(Display::Block);
            s.margin_top = Some(Length::Px(18.0));
            s.margin_bottom = Some(Length::Px(12.0));
            s.font_size = Some(Length::Px(24.0));
        }
        "h3" => {
            s.display = Some(Display::Block);
            s.margin_top = Some(Length::Px(16.0));
            s.margin_bottom = Some(Length::Px(10.0));
            s.font_size = Some(Length::Px(18.0));
        }
        "h4" | "h5" | "h6" => {
            s.display = Some(Display::Block);
            s.margin_top = Some(Length::Px(12.0));
            s.margin_bottom = Some(Length::Px(6.0));
            match tag_name {
                "h4" => s.font_size = Some(Length::Px(16.0)),
                "h5" => s.font_size = Some(Length::Px(14.0)),
                "h6" => s.font_size = Some(Length::Px(12.0)),
                _ => {}
            }
        }

        "p" => {
            s.display = Some(Display::Block);
            s.margin_top = Some(Length::Px(16.0));
            s.margin_bottom = Some(Length::Px(16.0));
            s.font_size = Some(Length::Px(16.0));
        }

        // リスト
        "ul" | "ol" => {
            s.display = Some(Display::Block);
        }
        "li" => {
            s.display = Some(Display::Block);
        }

        // テーブル要素は基本 block / table レイアウトは後で実装
        "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th" => {
            s.display = Some(Display::Block);
        }

        // インライン要素群
        "span" | "a" | "strong" | "em" | "b" | "i" | "small" => {
            s.display = Some(Display::Inline);
        }

        // メディア要素は inline-block 的扱いにしたいが、ここでは block にしておく（後で調整可）
        "img" | "svg" | "canvas" => {
            s.display = Some(Display::Inline);
        }

        // フォーム系
        "input" | "button" | "select" | "textarea" => {
            s.display = Some(Display::Inline);
        }

        // code / pre
        "pre" => {
            s.display = Some(Display::Block);
            s.padding_top = Some(Length::Px(8.0));
            s.padding_bottom = Some(Length::Px(8.0));
            // monospace/背景色等は later (color types)
        }
        "code" => {
            s.display = Some(Display::Inline);
        }

        // その他のブロック要素群
        _ if util::is_block_level_element(tag_name) => {
            s.display = Some(Display::Block);
        }

        _ => {
            // 不明要素は inline のまま
        }
    }

    s
}
