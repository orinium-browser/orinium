use crate::engine::tree::TreeNode;
use crate::html::HtmlNodeType;
use crate::html::tokenizer::Attribute;
use std::cell::RefCell;
use std::rc::Rc;

/// 単純セレクタ（タグ, .class, #id の組み合わせ）をノードに対して判定する。
fn simple_selector_matches(simple: &str, tag: &str, attrs: &[Attribute]) -> bool {
    // 例: div, .foo, #bar, div.foo#bar
    let mut pos = 0;
    let s = simple.trim();
    let bytes = s.as_bytes();

    // タグ名（先頭にタグ名が来ている場合）
    let mut tag_name = "";
    if !s.is_empty() && bytes[0] != b'.' && bytes[0] != b'#' {
        // read until . or #
        let mut end = 0;
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'.' || b == b'#' {
                break;
            }
            end = i + 1;
        }
        tag_name = &s[0..end];
        pos = end;
        if tag_name != tag {
            return false;
        }
    }

    // 残りの部分は .class や #id の繰り返し
    while pos < s.len() {
        let ch = s.as_bytes()[pos] as char;
        if ch == '.' {
            pos += 1;
            let start = pos;
            while pos < s.len() {
                let c = s.as_bytes()[pos] as char;
                if c == '.' || c == '#' {
                    break;
                }
                pos += 1;
            }
            let class = &s[start..pos];
            let has = attrs
                .iter()
                .any(|a| a.name == "class" && a.value.split_whitespace().any(|c| c == class));
            if !has {
                return false;
            }
        } else if ch == '#' {
            pos += 1;
            let start = pos;
            while pos < s.len() {
                let c = s.as_bytes()[pos] as char;
                if c == '.' || c == '#' {
                    break;
                }
                pos += 1;
            }
            let id = &s[start..pos];
            let has = attrs.iter().any(|a| a.name == "id" && a.value == id);
            if !has {
                return false;
            }
        } else {
            // Unknown token, fail-safe
            return false;
        }
    }

    true
}

/// 複合セレクタ（子孫セレクタを含む）をノードに対して判定する。
/// 例: "div .foo #bar" といったスペースで区切られたセレクタをサポートする。
pub fn selector_matches_on_node(
    selector: &str,
    node: &Rc<RefCell<TreeNode<HtmlNodeType>>>,
) -> bool {
    let selector = selector.trim();
    if selector.is_empty() {
        return false;
    }

    // セレクタを空白で分割して右からマッチさせる（子孫セレクタ）
    let parts: Vec<&str> = selector.split_whitespace().collect();
    let mut current_node = Some(Rc::clone(node));
    let mut part_idx = parts.len();

    // 右側のセレクタから順にマッチ
    while part_idx > 0 {
        part_idx -= 1;
        let part = parts[part_idx];

        // 現在のノード（またはその祖先）のどれかがこの simple selector にマッチする必要がある
        let mut matched = false;
        let mut search_node = current_node.clone();
        while let Some(n) = search_node {
            let n_borrow = n.borrow();
            if let HtmlNodeType::Element {
                tag_name,
                attributes,
                ..
            } = &n_borrow.value
                && simple_selector_matches(part, tag_name, attributes)
            {
                matched = true;
                // 次のパートをマッチさせるため、祖先からさらに探索する
                current_node = n_borrow.parent();
                break;
            }
            search_node = n_borrow.parent();
        }

        if !matched {
            return false;
        }
    }

    true
}
