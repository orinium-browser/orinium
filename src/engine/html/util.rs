//! # HTML関連のユーティリティ関数群
//! ## タグのカテゴリ分けと判定
//! - タグを明確にカテゴリ分け（block / inline / inline-block / table-ish / other）
//! - 重複が起きないように定義し、判定関数は既存の名前で使えるようにしている
//!   - element_category, is_block_level_element, is_inline_element
//!
//! 注:
//! - 「デフォルトのUA stylesheet による display の振る舞い」を基準に簡易判定しています。
//! - CSS による display の上書きやカスタム要素は考慮していません。
//! - 必要に応じてカテゴリやタグの追加・調整をしてください。
//!
//! ## htmlエスケープ処理
//! - 基本的なHTMLエスケープ文字列をデコードする関数を提供
//!   - decode_entity
//!

use entities::{Codepoints, ENTITIES};
use once_cell::sync::Lazy;
use std::collections::HashMap;

static NAMED_ENTITIES: Lazy<HashMap<&'static str, String>> = Lazy::new(|| {
    let mut map = HashMap::new();
    for ent in ENTITIES.iter() {
        let key = ent.entity.trim_start_matches('&').trim_end_matches(';');
        // Codepoints をマッチさせて String に変換
        let value = match ent.codepoints {
            Codepoints::Single(cp) => char::from_u32(cp)
                .map(|c| c.to_string())
                .unwrap_or_default(),
            Codepoints::Double(cp1, cp2) => {
                let mut s = String::new();
                if let Some(c1) = char::from_u32(cp1) {
                    s.push(c1);
                }
                if let Some(c2) = char::from_u32(cp2) {
                    s.push(c2);
                }
                s
            }
        };
        map.insert(key, value);
    }
    map
});

pub fn decode_entity(entity: &str) -> Option<String> {
    if let Some(val) = NAMED_ENTITIES.get(entity) {
        return Some(val.clone());
    }

    if entity.starts_with("#x") || entity.starts_with("#X") {
        return u32::from_str_radix(&entity[2..], 16)
            .ok()
            .and_then(char::from_u32)
            .map(|c| c.to_string());
    }

    if let Some(entity_number) = entity.strip_prefix('#') {
        return entity_number
            .parse::<u32>()
            .ok()
            .and_then(char::from_u32)
            .map(|c| c.to_string());
    }

    None
}

fn normalize(tag_name: &str) -> String {
    tag_name.trim().to_ascii_lowercase()
}

/// 内部カテゴリ配列（重複なし）
/// - block: 通常 `display:block` またはブロックに準ずる振る舞い（p, div, h1.. など）
/// - inline: 通常 `display:inline`（a, span, em, img 等）
/// - inline_block: 通常 `display:inline-block` / replaced inline-block（button, select など）
/// - tableish: table 系（display: table / table-row / table-cell など）
/// - other: 上のどれにも該当しない雑多な要素
const BLOCK_TAGS: &[&str] = &[
    // セクショナル / グループ
    "html",
    "body",
    "main",
    "header",
    "footer",
    "section",
    "nav",
    "article",
    "aside",
    // 見出し
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    // 段落系
    "p",
    "pre",
    "blockquote",
    "address",
    "hr",
    // レイアウト・グループ
    "div",
    "fieldset",
    "figure",
    "figcaption",
    "details",
    "summary",
    // リスト系（li/dt/dd は list-item / block-like）
    "ul",
    "ol",
    "li",
    "dl",
    "dt",
    "dd",
    // フォーム系（幅取りがある要素をブロック扱いしたい場合に含めるがここでは block扱い）
    "form",
    "textarea",
    // 埋め込み（ブロック的に扱われることが多いが厳密には元の display を参照）
    "iframe",
    "canvas",
    "object",
    "embed",
];

const INLINE_TAGS: &[&str] = &[
    // テキスト系
    "a", "span", "em", "strong", "b", "i", "u", "small", "sub", "sup", "mark", "code", "q", "cite",
    "time", "var", "samp", "kbd", "dfn",
    // 画像・改行等（img は UA stylesheet では inline と定義される）
    "img", "br", "wbr", // フォーム系の一部（input は通常 inline）
    "input", "label",
];

const INLINE_BLOCK_TAGS: &[&str] = &[
    // ボタンやセレクト類はブラウザによって inline-block 規定が多い
    // 明確に inline-block として扱いたい要素をここに分離
    "button", "select", "option",
];

const TABLEISH_TAGS: &[&str] = &[
    // 表関連は table 系独特の display を持つため別カテゴリ
    "table", "thead", "tbody", "tfoot", "tr", "td", "th", "caption", "colgroup", "col",
];

const OTHER_TAGS: &[&str] = &[
    // 上のどれにも入れなかった代表的要素
    "svg", // svg は通常 inline だが独自挙動のため other に分離してもよい
];

/// 要素の「カテゴリ文字列」を返すユーティリティ（テスト・デバッグ用）
/// 戻り値: "block" | "inline" | "inline-block" | "table" | "other" | "unknown"
pub fn element_category(tag_name: &str) -> &'static str {
    let tag = normalize(tag_name);
    let t = tag.as_str();
    if BLOCK_TAGS.contains(&t) {
        "block"
    } else if INLINE_TAGS.contains(&t) {
        "inline"
    } else if INLINE_BLOCK_TAGS.contains(&t) {
        "inline-block"
    } else if TABLEISH_TAGS.contains(&t) {
        "table"
    } else if OTHER_TAGS.contains(&t) {
        "other"
    } else {
        "unknown"
    }
}

// 互換性のための関数:
/// - is_block_level_element は "block" と "table" を block-like として true を返す
pub fn is_block_level_element(tag_name: &str) -> bool {
    matches!(element_category(tag_name), "block" | "table")
}

/// - is_inline_element は "inline" のみ true を返す（inline-block は false）
pub fn is_inline_element(tag_name: &str) -> bool {
    matches!(element_category(tag_name), "inline")
}
