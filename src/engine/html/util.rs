//!HTML関連のユーティリティ関数群

/// is_block_level_element - タグ名が典型的なブロック要素かどうか判定する
///
/// 注意:
/// - HTML5 の「デフォルトでブロック扱いされる要素」を代表例で列挙していますが、
///   仕様の解釈やブラウザ依存・CSSでのdisplay変更には触れていません。
/// - 必要なら要素リストに追加・削除してください。
pub fn is_block_level_element(tag_name: &str) -> bool {
    let tag = tag_name.trim().to_ascii_lowercase();
    matches!(
        tag.as_str(),
        // 主要なセクショナル要素
        "html" | "body" | "main" | "header" | "footer" | "section" | "nav" | "article" | "aside" |
        // 見出し
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" |
        // テキスト／段落系
        "p" | "pre" | "blockquote" | "address" | "hr" |
        // グループ／レイアウト
        "div" | "fieldset" | "legend" | "details" | "summary" | "figure" | "figcaption" |
        // リスト／表組み
        "ul" | "ol" | "li" | "dl" | "dt" | "dd" |
        "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th" |
        // フォーム系（多くはブロック表示される）
        "form" | "textarea" | "output" | "meter" | "progress" |
        // メディア・埋め込み
        "canvas" | "video" | "audio" | "svg" | "object" | "embed" | "iframe"
    )
}