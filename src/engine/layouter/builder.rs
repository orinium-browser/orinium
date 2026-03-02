use crate::engine::bridge::text;
use crate::engine::css::{
    matcher::{ElementChain, ElementInfo},
    values::{CssValue, Unit},
};
use crate::engine::tree::TreeNode;
use crate::html::HtmlNodeType;

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use ui_layout::{
    AlignItems, BoxSizing, Display, FlexDirection, JustifyContent, LayoutNode, Length, Style,
};

use super::css_resolver::ResolvedStyles;
use super::types::{
    BorderStyle, Color, ContainerRole, ContainerStyle, FontStyle, FontWeight, InfoNode,
    MeasureCache, NodeKind, TextAlign, TextDecoration, TextStyle,
};

/// Builds a layout tree (`LayoutNode`) and a render info tree (`InfoNode`) from the DOM.
///
/// # Overview
/// - Recursively traverses the HTML DOM
/// - Applies resolved CSS declarations
/// - Computes layout-related styles
/// - Collects render-time information (color, font size, text)
///
/// # Style resolution order (low → high priority)
///
/// 1. **Inherited values from parent**
///    - `text_style`
///
/// 2. **Resolved CSS declarations**
///    - Overrides inherited values when specified
///
/// 3. **HTML defaults / semantics**
///    - `display` (block, inline, etc.)
///    - Text measurement for text nodes
///
/// # Inherited properties
///
/// Only the following properties are inherited explicitly:
///
/// - `text_style`
///
/// All other style fields are initialized per node and are **not inherited**.
///
/// # Parameters
///
/// - `parent_text_style`
///
/// These values must be passed from the computed result of the parent when
/// calling this function recursively.
///
/// # Returns
///
/// A tuple of:
/// - `LayoutNode`: used by the layout engine
/// - `InfoNode`: used for rendering (text, color, font size)
pub fn build_layout_and_info(
    dom: &Rc<RefCell<TreeNode<HtmlNodeType>>>,
    resolved_styles: &ResolvedStyles,
    measurer: &dyn text::TextMeasurer<TextStyle>,
    parent_text_style: TextStyle,
    mut chain: ElementChain,
) -> (LayoutNode, InfoNode) {
    let html_node = dom.borrow().value.clone();

    /* -----------------------------
       Initial values (inheritance)
    ----------------------------- */
    let mut style = Style::default();

    let mut text_style = parent_text_style;
    let mut container_style = ContainerStyle::default();

    /* -----------------------------
       Apply resolved CSS
    ----------------------------- */
    if let HtmlNodeType::Element {
        tag_name,
        attributes,
        ..
    } = &html_node
    {
        let id = attributes
            .iter()
            .find(|a| a.name == "id")
            .map(|a| a.value.clone());

        let class_list: Vec<String> = attributes
            .iter()
            .find(|attr| attr.name == "class")
            .map(|attr| {
                attr.value
                    .split_whitespace()
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();

        chain.insert(
            0,
            ElementInfo {
                tag_name: tag_name.clone(),
                id,
                classes: class_list,
            },
        );

        let candidates = collect_candidates(resolved_styles, &chain);

        for (name, (value, _, _)) in candidates {
            if name.starts_with("--") {
                continue;
            }
            apply_declaration(
                &name,
                &value,
                &mut style,
                &mut container_style,
                &mut text_style,
            );
        }
    }

    let mut kind = if let HtmlNodeType::Text(t) = &html_node {
        let t = normalize_whitespace(t);

        let mut kind = NodeKind::Text {
            text: t.clone(),
            style: text_style,
            measured: None,
        };

        ensure_text_measured(&mut style, &mut kind, measurer);

        kind
    } else if let Some(name) = html_node.tag_name()
        && name == "a"
        && let Some(href) = html_node.get_attr("href")
    {
        NodeKind::Container {
            scroll_x: false,
            scroll_y: false,
            scroll_offset_x: 0.0,
            scroll_offset_y: 0.0,
            style: container_style,
            role: ContainerRole::Link {
                href: href.to_string(),
            },
        }
    } else {
        NodeKind::Container {
            scroll_x: false,
            scroll_y: false,
            scroll_offset_x: 0.0,
            scroll_offset_y: 0.0,
            style: container_style,
            role: ContainerRole::Normal,
        }
    };

    /* -----------------------------
       Children
    ----------------------------- */

    // NOTE:
    // Inline / LineBox は未実装。
    // block 要素の直下に text ノードが存在する場合、
    // 親を flex(row) として扱うことで inline flow を暫定的に再現する。
    // 将来的には LineBox 実装に置き換える。
    //
    // Table 要素も未実装。
    // Table 要素は暫定的に Flex に置き換える。
    // TODO: 将来的には TableLayout 実装に置き換える。
    let mut layout_children = Vec::new();
    let mut info_children = Vec::new();

    if !matches!(style.display, Display::None) {
        let mut has_text_child = false;

        for child_dom in dom.borrow().children() {
            if matches!(child_dom.borrow().value, HtmlNodeType::Text(_)) {
                has_text_child = true;
                break;
            }
        }

        // 子に TextNode がある Block 要素は Flex(row) に変換
        if has_text_child && matches!(style.display, Display::Block) {
            style.display = Display::Flex {
                flex_direction: FlexDirection::Row,
            };
        }

        // Table 要素は暫定的に Flex に置き換える。
        match &html_node {
            HtmlNodeType::Element { tag_name, .. }
                if tag_name == "table"
                    || tag_name == "tbody"
                    || tag_name == "thead"
                    || tag_name == "tfoot" =>
            {
                style.display = Display::Flex {
                    flex_direction: FlexDirection::Column,
                };
            }
            HtmlNodeType::Element { tag_name, .. } if tag_name == "tr" => {
                style.display = Display::Flex {
                    flex_direction: FlexDirection::Row,
                };
            }
            _ => {}
        }

        for child_dom in dom.borrow().children() {
            let (child_layout, child_info) = build_layout_and_info(
                child_dom,
                resolved_styles,
                measurer,
                text_style,
                chain.clone(),
            );

            if dom.borrow().value.tag_name() == Some("html")
                && child_dom.borrow().value.tag_name() == Some("body")
            {
                if let NodeKind::Container { style, .. } = &mut kind {
                    if style.background_color == Color(0, 0, 0, 0) {
                        let background_color = {
                            let NodeKind::Container { style, .. } = &child_info.kind else {
                                continue;
                            };
                            style.background_color
                        };
                        // html 要素の body 子要素に背景色が指定されていない場合、
                        // body の背景色を html の背景色で上書きする
                        style.background_color = background_color;
                    }
                }
            }

            layout_children.push(child_layout);
            info_children.push(child_info);
        }
    }

    let layout = LayoutNode::with_children(style, layout_children);

    let info = InfoNode {
        kind,
        children: info_children,
    };

    (layout, info)
}

fn calc_text_measure_hash(text: &str, style: &TextStyle) -> u64 {
    use std::collections::hash_map::DefaultHasher;

    let mut hasher = DefaultHasher::new();

    text.hash(&mut hasher);
    style.font_size.to_bits().hash(&mut hasher);
    style.font_weight.hash(&mut hasher);
    style.font_style.hash(&mut hasher);

    hasher.finish()
}

fn ensure_text_measured(
    node_style: &mut Style,
    kind: &mut NodeKind,
    measurer: &dyn text::TextMeasurer<TextStyle>,
) {
    let NodeKind::Text {
        text,
        style,
        measured,
    } = kind
    else {
        return;
    };

    let hash = calc_text_measure_hash(text, style);

    let needs_measure = measured.as_ref().map(|m| m.hash != hash).unwrap_or(true);

    if !needs_measure {
        return;
    }

    let req = text::TextMeasureRequest {
        text: text.clone(),
        style: *style,
        max_width: None,
        wrap: false,
    };

    let (width, height) = measurer
        .measure(&req)
        .map(|m| (m.width, m.height))
        .unwrap_or((800.0, style.font_size * 1.2));

    *measured = Some(MeasureCache {
        hash,
        width,
        height,
    });

    node_style.size.width = Length::Px(width);
    node_style.size.height = Length::Px(height);
}

fn normalize_whitespace(text: &str) -> String {
    let mut result = String::new();
    let mut prev_was_space = false;

    for c in text.chars() {
        if c.is_whitespace() {
            if !prev_was_space {
                result.push(' ');
                prev_was_space = true;
            }
        } else {
            result.push(c);
            prev_was_space = false;
        }
    }

    result
}

fn collect_candidates(
    resolved_styles: &ResolvedStyles,
    chain: &ElementChain,
) -> HashMap<String, (CssValue, (u32, u32, u32), usize)> {
    let mut candidates: HashMap<String, (CssValue, (u32, u32, u32), usize)> = HashMap::new();

    for decl in resolved_styles {
        if decl.selector.matches(&chain) {
            let entry = candidates.get(&decl.name);

            let should_replace = match entry {
                None => true,
                Some((_, spec, order)) => {
                    decl.specificity > *spec || (decl.specificity == *spec && decl.order > *order)
                }
            };

            if should_replace {
                candidates.insert(
                    decl.name.clone(),
                    (decl.value.clone(), decl.specificity, decl.order),
                );
            }
        }
    }

    candidates
}

fn apply_declaration(
    name: &str,
    value: &CssValue,
    style: &mut Style,
    container_style: &mut ContainerStyle,
    text_style: &mut TextStyle,
) -> Option<()> {
    fn expand_box<F>(
        value: &CssValue,
        text_style: &TextStyle,
        resolve_css_len: &impl Fn(&CssValue, &TextStyle) -> Option<Length>,
        mut set: F,
    ) -> Option<()>
    where
        F: FnMut(Length, Length, Length, Length),
    {
        let resolve = |v: &CssValue| -> Option<Length> { resolve_css_len(v, text_style) };

        match value {
            CssValue::List(values) => {
                let vals: Vec<Length> = values.iter().map(resolve).collect::<Option<_>>()?;

                match vals.as_slice() {
                    [a] => set(a.clone(), a.clone(), a.clone(), a.clone()),
                    [v, h] => set(v.clone(), h.clone(), v.clone(), h.clone()),
                    [t, h, b] => set(t.clone(), h.clone(), b.clone(), h.clone()),
                    [t, r, b, l] => set(t.clone(), r.clone(), b.clone(), l.clone()),
                    _ => return None,
                }
            }

            _ => {
                let v = resolve_css_len(value, text_style)?;
                set(v.clone(), v.clone(), v.clone(), v);
            }
        }

        Some(())
    }

    fn parse_border_shorthand(
        value: &CssValue,
        text_style: &TextStyle,
    ) -> Option<(Option<Length>, Option<BorderStyle>, Option<Color>)> {
        let mut width: Option<Length> = None;
        let mut style_v: Option<BorderStyle> = None;
        let mut color_v: Option<Color> = None;

        let items: Vec<&CssValue> = match value {
            CssValue::List(values) => values.iter().collect(),
            _ => vec![value],
        };

        for v in items {
            let token = v;

            // try as length (numeric lengths)
            if width.is_none() {
                if let Some(l) = resolve_css_len(token, text_style) {
                    width = Some(l);
                    continue;
                }
            }

            // try as width keyword (thin/medium/thick). Check keywords before style keywords.
            if width.is_none() {
                if let CssValue::Keyword(s) = token {
                    match s.as_str().to_ascii_lowercase().as_str() {
                        "thin" => {
                            width = Some(Length::Px(1.0));
                            continue;
                        }
                        "medium" => {
                            width = Some(Length::Px(3.0));
                            continue;
                        }
                        "midium" => {
                            width = Some(Length::Px(3.0));
                            continue;
                        } // common misspelling
                        "thick" => {
                            width = Some(Length::Px(5.0));
                            continue;
                        }
                        _ => {}
                    }
                }
            }

            // try as style keyword
            if style_v.is_none() {
                if let CssValue::Keyword(s) = token {
                    let s_lower = s.as_str();
                    let parsed = match s_lower {
                        "none" => Some(BorderStyle::None),
                        "solid" => Some(BorderStyle::Solid),
                        "dashed" => Some(BorderStyle::Dashed),
                        "dotted" => Some(BorderStyle::Dotted),
                        _ => None,
                    };

                    if let Some(p) = parsed {
                        style_v = Some(p);
                        continue;
                    }
                }
            }

            // try as color
            if color_v.is_none() {
                if let Some(c) = resolve_css_color(token) {
                    color_v = Some(c);
                    continue;
                }
            }

            // unknown token: ignore
        }

        Some((width, style_v, color_v))
    }

    match (name, value) {
        /* ======================
         * Display
         * ====================== */
        ("display", CssValue::Keyword(v)) => {
            style.display = match v.as_str() {
                "block" => Display::Block,
                "flex" => Display::Flex {
                    flex_direction: FlexDirection::Row,
                },
                "inline" => Display::Flex {
                    // tmp：inline = row flex
                    flex_direction: FlexDirection::Row,
                },
                "none" => Display::None,
                _ => style.display,
            };
        }

        /* ======================
         * Color / Text
         * ====================== */
        ("background-color", _) => {
            container_style.background_color = match value {
                CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("inherit") => {
                    // inherit: use parent's text color
                    text_style.color
                }
                CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("currentColor") => {
                    text_style.color
                }
                _ => resolve_css_color(value)?,
            };
        }

        ("background", _) => {
            container_style.background_color = match value {
                CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("inherit") => {
                    // inherit: use parent's text color
                    text_style.color
                }
                CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("currentColor") => {
                    text_style.color
                }
                _ => resolve_css_color(value)?,
            };
        }

        ("color", _) => {
            text_style.color = match value {
                CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("inherit") => {
                    // inherit: use parent's color
                    text_style.color
                }
                CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("currentColor") => {
                    text_style.color
                }
                _ => resolve_css_color(value)?,
            }
        }

        ("font-size", CssValue::Length(_, _)) => {
            // TODO: Add other size
            let len = resolve_css_len(value, text_style)?;
            let px = match &len {
                Length::Px(v) => *v,
                Length::Percent(v) => *v * text_style.font_size / 100.0,
                _ => {
                    log::error!(target: "Layouter", "Unknown size type for font-size: {:?}", len);
                    return None;
                }
            };
            text_style.font_size = px;
        }

        ("font-weight", CssValue::Keyword(v)) => {
            text_style.font_weight = match v.as_str() {
                "normal" => FontWeight::NORMAL,
                "bold" => FontWeight::BOLD,
                _ => text_style.font_weight,
            };
        }
        ("font-weight", CssValue::Number(v)) => {
            text_style.font_weight = FontWeight(*v as u16);
        }

        ("font-style", CssValue::Keyword(v)) => {
            text_style.font_style = match v.as_str() {
                "normal" => FontStyle::Normal,
                "italic" => FontStyle::Italic,
                "oblique" => FontStyle::Oblique,
                _ => text_style.font_style,
            };
        }

        ("text-decoration", CssValue::Keyword(v)) => {
            text_style.text_decoration = match v.as_str() {
                "none" => TextDecoration::None,
                "underline" => TextDecoration::Underline,
                "line-through" => TextDecoration::LineThrough,
                "overline" => TextDecoration::Overline,
                _ => TextDecoration::None,
            };
        }

        ("text-align", CssValue::Keyword(v)) if v == "left" => {
            text_style.text_align = TextAlign::Left;
        }
        ("text-align", CssValue::Keyword(v)) if v == "center" => {
            text_style.text_align = TextAlign::Center;
        }
        ("text-align", CssValue::Keyword(v)) if v == "right" => {
            text_style.text_align = TextAlign::Right;
        }

        /* ======================
         * Box Model
         * ====================== */
        ("box-sizing", CssValue::Keyword(v)) => {
            style.box_sizing = match v.as_str() {
                "content-box" => BoxSizing::ContentBox,
                "border-box" => BoxSizing::BorderBox,
                _ => BoxSizing::ContentBox,
            };
        }

        ("border-style", CssValue::Keyword(v)) => {
            let s = match v.as_str() {
                "none" => BorderStyle::None,
                "solid" => BorderStyle::Solid,
                "dashed" => BorderStyle::Dashed,
                "dotted" => BorderStyle::Dotted,
                _ => BorderStyle::None,
            };

            container_style.border_style.top = s;
            container_style.border_style.right = s;
            container_style.border_style.bottom = s;
            container_style.border_style.left = s;
        }

        ("margin", v) => {
            expand_box(v, text_style, &resolve_css_len, |t, r, b, l| {
                style.spacing.margin_top = t;
                style.spacing.margin_right = r;
                style.spacing.margin_bottom = b;
                style.spacing.margin_left = l;
            })?;
        }
        ("margin-top", _) => {
            style.spacing.margin_top = resolve_css_len(value, text_style)?;
        }
        ("margin-right", _) => {
            style.spacing.margin_right = resolve_css_len(value, text_style)?;
        }
        ("margin-bottom", _) => {
            style.spacing.margin_bottom = resolve_css_len(value, text_style)?;
        }
        ("margin-left", _) => {
            style.spacing.margin_left = resolve_css_len(value, text_style)?;
        }

        ("border", v) => {
            let (maybe_width, maybe_style, maybe_color) = parse_border_shorthand(v, text_style)?;

            if let Some(w) = maybe_width {
                style.spacing.border_top = w.clone();
                style.spacing.border_right = w.clone();
                style.spacing.border_bottom = w.clone();
                style.spacing.border_left = w;
            }

            if let Some(s) = maybe_style {
                container_style.border_style.top = s;
                container_style.border_style.right = s;
                container_style.border_style.bottom = s;
                container_style.border_style.left = s;
            }

            if let Some(c) = maybe_color {
                container_style.border_color.top = c.clone();
                container_style.border_color.right = c.clone();
                container_style.border_color.bottom = c.clone();
                container_style.border_color.left = c;
            }
        }
        ("border-top", _) => {
            if let CssValue::List(_) = value {
                let (maybe_width, maybe_style, maybe_color) =
                    parse_border_shorthand(value, text_style)?;
                if let Some(w) = maybe_width {
                    style.spacing.border_top = w;
                }
                if let Some(s) = maybe_style {
                    container_style.border_style.top = s;
                }
                if let Some(c) = maybe_color {
                    container_style.border_color.top = c;
                }
            } else {
                style.spacing.border_top = resolve_css_len(value, text_style)?;
            }
        }
        ("border-right", _) => {
            if let CssValue::List(_) = value {
                let (maybe_width, maybe_style, maybe_color) =
                    parse_border_shorthand(value, text_style)?;
                if let Some(w) = maybe_width {
                    style.spacing.border_right = w;
                }
                if let Some(s) = maybe_style {
                    container_style.border_style.right = s;
                }
                if let Some(c) = maybe_color {
                    container_style.border_color.right = c;
                }
            } else {
                style.spacing.border_right = resolve_css_len(value, text_style)?;
            }
        }
        ("border-bottom", _) => {
            if let CssValue::List(_) = value {
                let (maybe_width, maybe_style, maybe_color) =
                    parse_border_shorthand(value, text_style)?;
                if let Some(w) = maybe_width {
                    style.spacing.border_bottom = w;
                }
                if let Some(s) = maybe_style {
                    container_style.border_style.bottom = s;
                }
                if let Some(c) = maybe_color {
                    container_style.border_color.bottom = c;
                }
            } else {
                style.spacing.border_bottom = resolve_css_len(value, text_style)?;
            }
        }
        ("border-left", _) => {
            if let CssValue::List(_) = value {
                let (maybe_width, maybe_style, maybe_color) =
                    parse_border_shorthand(value, text_style)?;
                if let Some(w) = maybe_width {
                    style.spacing.border_left = w;
                }
                if let Some(s) = maybe_style {
                    container_style.border_style.left = s;
                }
                if let Some(c) = maybe_color {
                    container_style.border_color.left = c;
                }
            } else {
                style.spacing.border_left = resolve_css_len(value, text_style)?;
            }
        }

        ("padding", v) => {
            expand_box(v, text_style, &resolve_css_len, |t, r, b, l| {
                style.spacing.padding_top = t;
                style.spacing.padding_right = r;
                style.spacing.padding_bottom = b;
                style.spacing.padding_left = l;
            })?;
        }
        ("padding-top", _) => {
            style.spacing.padding_top = resolve_css_len(value, text_style)?;
        }
        ("padding-right", _) => {
            style.spacing.padding_right = resolve_css_len(value, text_style)?;
        }
        ("padding-bottom", _) => {
            style.spacing.padding_bottom = resolve_css_len(value, text_style)?;
        }
        ("padding-left", _) => {
            style.spacing.padding_left = resolve_css_len(value, text_style)?;
        }

        /* ======================
         * Size
         * ====================== */
        ("width", _) => {
            style.size.width = resolve_css_len(value, text_style)?;
        }
        ("height", _) => {
            style.size.height = resolve_css_len(value, text_style)?;
        }
        ("min-width", _) => {
            style.size.min_width = resolve_css_len(value, text_style)?;
        }
        ("min-height", _) => {
            style.size.min_height = resolve_css_len(value, text_style)?;
        }
        ("max-width", _) => {
            style.size.max_width = resolve_css_len(value, text_style)?;
        }
        ("max-height", _) => {
            style.size.max_height = resolve_css_len(value, text_style)?;
        }

        /* ======================
         * Flex
         * ====================== */
        ("flex-direction", CssValue::Keyword(v)) => {
            if let Display::Flex { flex_direction } = &mut style.display {
                *flex_direction = match v.as_str() {
                    "row" => FlexDirection::Row,
                    "column" => FlexDirection::Column,
                    _ => return None,
                };
            }
        }

        ("justify-content", CssValue::Keyword(v)) => {
            style.justify_content = match v.as_str() {
                "flex-start" => JustifyContent::Start,
                "center" => JustifyContent::Center,
                "flex-end" => JustifyContent::End,
                "space-between" => JustifyContent::SpaceBetween,
                "space-around" => JustifyContent::SpaceAround,
                _ => return None,
            };
        }

        ("align-items", CssValue::Keyword(v)) => {
            style.align_items = match v.as_str() {
                "stretch" => AlignItems::Stretch,
                "flex-start" => AlignItems::Start,
                "center" => AlignItems::Center,
                "flex-end" => AlignItems::End,
                _ => return None,
            };
        }

        ("gap", _) => {
            let gap = resolve_css_len(value, text_style)?;
            style.row_gap = gap.clone();
            style.column_gap = gap;
        }

        ("align-self", CssValue::Keyword(v)) => {
            style.item_style.align_self = match v.as_str() {
                "stretch" => Some(AlignItems::Stretch),
                "flex-start" => Some(AlignItems::Start),
                "center" => Some(AlignItems::Center),
                "flex-end" => Some(AlignItems::End),
                _ => return None,
            };
        }

        ("flex-grow", CssValue::Number(v)) => {
            style.item_style.flex_grow = *v;
        }

        ("flex-basis", _) => {
            style.item_style.flex_basis = resolve_css_len(value, text_style)?;
        }

        _ => {}
    }
    Some(())
}

/// Resolve CssValue to Length.
fn resolve_css_len(css_len: &CssValue, text_style: &TextStyle) -> Option<Length> {
    match &css_len {
        CssValue::Length(v, Unit::Em) => Some(Length::Px(text_style.font_size * v)),
        CssValue::Length(v, Unit::Rem) => Some(Length::Px(16.0 * v)), // html sont-size 仮値
        CssValue::Length(v, u) => match u {
            Unit::Percent => Some(Length::Percent(*v)),
            Unit::Px => Some(Length::Px(*v)),
            Unit::Vw => Some(Length::Vw(*v)),
            Unit::Vh => Some(Length::Vh(*v)),
            Unit::Em | Unit::Rem => unreachable!(),
        },
        CssValue::Number(0.0) => Some(Length::Px(0.0)),
        CssValue::Keyword(s) => match s.as_str() {
            "auto" => Some(Length::Auto),
            _ => None,
        },
        CssValue::Function(name, args) if name == "calc" && !args.is_empty() => {
            let mut iter = args.iter();
            let mut result = resolve_css_len(iter.next().unwrap(), text_style)?;

            while let (Some(op), Some(val)) = (iter.next(), iter.next()) {
                match op {
                    CssValue::Keyword(o) if o == "+" => {
                        let val_resolved = resolve_css_len(val, text_style)?;
                        result = Length::Add(Box::new(result), Box::new(val_resolved));
                    }
                    CssValue::Keyword(o) if o == "-" => {
                        let val_resolved = resolve_css_len(val, text_style)?;
                        result = Length::Sub(Box::new(result), Box::new(val_resolved));
                    }
                    CssValue::Keyword(o) if o == "*" => {
                        if let CssValue::Number(factor) = val {
                            result = Length::Mul(Box::new(result), *factor);
                        } else {
                            log::error!(target: "Layouter", "Invalid operand for multiplication in calc(): {:?}", val);
                            return None;
                        }
                    }
                    CssValue::Keyword(o) if o == "/" => {
                        if let CssValue::Number(factor) = val {
                            if *factor == 0.0 {
                                log::error!(target: "Layouter", "Division by zero in calc()");
                                return None;
                            }
                            result = Length::Div(Box::new(result), *factor);
                        } else {
                            log::error!(target: "Layouter", "Invalid operand for division in calc(): {:?}", val);
                            return None;
                        }
                    }
                    _ => {
                        log::error!(target: "Layouter", "Unknown operator for calc function: {:?}", op);
                        return None;
                    }
                }
            }

            Some(result)
        }
        _ => {
            log::error!(target: "Layouter", "Unknown CSS Length type: {:?}", css_len);
            None
        }
    }
}

/// Resolve a computed CssValue into a final RGBA Color.
///
/// Assumptions:
/// - This function is called *after* cascade and inheritance resolution.
/// - Keywords like `currentColor`, `inherit`, `initial`, `unset`
///   must NOT reach this stage.
/// - The returned Color is always absolute RGBA.
fn resolve_css_color(css_color: &CssValue) -> Option<Color> {
    fn keyword_color_to_color(keyword: &str) -> Option<Color> {
        // NOTE:
        // Keyword matching is case-insensitive according to CSS specs.
        // Keep this list limited to commonly used CSS Color Level 3 keywords.
        match keyword.to_ascii_lowercase().as_str() {
            // Basic colors
            "black" => Some(Color(0, 0, 0, 255)),
            "white" => Some(Color(255, 255, 255, 255)),
            "red" => Some(Color(255, 0, 0, 255)),
            "green" => Some(Color(0, 128, 0, 255)),
            "blue" => Some(Color(0, 0, 255, 255)),
            "yellow" => Some(Color(255, 255, 0, 255)),

            // Gray variants (US / UK spelling)
            "gray" | "grey" => Some(Color(128, 128, 128, 255)),
            "lightgray" | "lightgrey" => Some(Color(211, 211, 211, 255)),
            "darkgray" | "darkgrey" => Some(Color(169, 169, 169, 255)),

            // Frequently used named colors
            "royalblue" => Some(Color(65, 105, 225, 255)),
            "cornflowerblue" => Some(Color(100, 149, 237, 255)),
            "skyblue" => Some(Color(135, 206, 235, 255)),
            "lightblue" => Some(Color(173, 216, 230, 255)),

            "orange" => Some(Color(255, 165, 0, 255)),
            "pink" => Some(Color(255, 192, 203, 255)),
            "purple" => Some(Color(128, 0, 128, 255)),
            "brown" => Some(Color(165, 42, 42, 255)),

            // Special keyword
            "transparent" => Some(Color(0, 0, 0, 0)),
            "initial" => Some(Color(0, 0, 0, 255)),

            // CSS Level 4 system colors (approximate)
            // stub implementations
            "buttonface" => Some(Color(240, 240, 240, 255)),
            "buttontext" => Some(Color(0, 0, 0, 255)),
            "linktext" => Some(Color(0, 0, 255, 255)),

            // Stub for none keyword (e.g. border-color: none, background: none, etc.)
            "none" => Some(Color(0, 0, 0, 0)),

            _ => {
                log::error!(target: "Layouter", "Unknown CSS color keyword: {}", keyword);
                None
            }
        }
    }

    /// Convert HSL to RGB (0..255)
    fn hsla_to_rgba(h: f32, s: f32, l: f32, a: f32) -> (u8, u8, u8, u8) {
        // 1. Compute Chroma
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let h_prime = h / 60.0;
        let x = c * (1.0 - ((h_prime % 2.0) - 1.0).abs());

        // 2. Determine preliminary RGB values based on hue sector
        let (r1, g1, b1) = match h_prime as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            5 | 6 => (c, 0.0, x),
            _ => (0.0, 0.0, 0.0),
        };

        // 3. Add m to match the lightness
        let m = l - c / 2.0;
        let r = ((r1 + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        let g = ((g1 + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        let b = ((b1 + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        let a = (a * 255.0).round().clamp(0.0, 255.0) as u8;

        (r, g, b, a)
    }

    match css_color {
        // Already parsed as an absolute color (rgb/rgba/hex, etc.)
        CssValue::Color(_) => {
            let (r, g, b, a) = css_color.to_rgba_tuple()?;
            Some(Color(r, g, b, a))
        }

        // Named color keyword
        CssValue::Keyword(value) => keyword_color_to_color(value),

        // rgba(r,g,b,a)
        CssValue::Function(func, args) if func == "rgba" && args.len() == 4 => {
            if let (
                CssValue::Number(r),
                CssValue::Number(g),
                CssValue::Number(b),
                CssValue::Number(a),
            ) = (&args[0], &args[1], &args[2], &args[3])
            {
                Some(Color(
                    (*r * 255.0).round() as u8,
                    (*g * 255.0).round() as u8,
                    (*b * 255.0).round() as u8,
                    (*a * 255.0).round() as u8,
                ))
            } else {
                None
            }
        }

        // rgb(r,g,b)
        CssValue::Function(func, args) if func == "rgb" && args.len() == 3 => {
            if let (CssValue::Number(r), CssValue::Number(g), CssValue::Number(b)) =
                (&args[0], &args[1], &args[2])
            {
                Some(Color(
                    (*r * 255.0).round() as u8,
                    (*g * 255.0).round() as u8,
                    (*b * 255.0).round() as u8,
                    255,
                ))
            } else {
                None
            }
        }

        // hsl(h,s,l)
        CssValue::Function(func, args) if func == "hsl" && args.len() == 3 => {
            if let (CssValue::Number(h), CssValue::Number(s), CssValue::Number(l)) =
                (&args[0], &args[1], &args[2])
            {
                let (r, g, b, a) = hsla_to_rgba(*h, *s, *l, 1.0);
                Some(Color(r, g, b, a))
            } else {
                None
            }
        }

        // hsla(h,s,l,a)
        CssValue::Function(func, args) if func == "hsla" && args.len() == 4 => {
            if let (
                CssValue::Number(h),
                CssValue::Number(s),
                CssValue::Number(l),
                CssValue::Number(a),
            ) = (&args[0], &args[1], &args[2], &args[3])
            {
                let (r, g, b, a) = hsla_to_rgba(*h, *s, *l, *a);
                Some(Color(r, g, b, a))
            } else {
                None
            }
        }

        // Any other value reaching here is a pipeline error
        _ => {
            log::error!(
                target: "Layouter",
                "Unexpected CSS color value at layout stage: {:?}",
                css_color
            );
            None
        }
    }
}
