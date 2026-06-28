//! Layout builder, which transforms a DOM tree into a UI layout.

use crate::engine::bridge::text::{self, FallbackTextMeasurer, MeasuredFragment, TextMeasurer};
use crate::engine::css::{
    matcher::{ElementChain, ElementInfo},
    values::{CssValue, Unit},
};
use crate::engine::html::HtmlNodeType;
use crate::engine::tree::TreeNode;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use ui_layout::{
    AlignItems, BoxSizing, Display, FlexDirection, Fragment, InnerDisplay, ItemFragment,
    JustifyContent, LayoutChild, LayoutNode, Length, LengthOrAuto, OuterDisplay, Style,
};

use super::css_resolver::ResolvedStyles;
use super::types::{
    Background, BorderStyle, Color, ColorStop, ContainerRole, ContainerStyle, FontStyle,
    FontWeight, Gradient, GradientKind, InfoNode, LineHeight, NodeKind, RadialShape,
    RadialSizeKind, TextAlign, TextDecoration, TextStyle,
};

const DEFAULT_LINE_FACTOR: f32 = 1.2;

/// Inherited values from parent, passed down through the tree.
///
/// `text_style` carries all inherited text/line-height values.
/// Add new fields here when additional deferred-resolution properties arise.
#[derive(Clone, Copy)]
pub struct InheritedCss {
    pub text_style: TextStyle,
}

/// Convert a resolved `Length` to an absolute pixel value for `LineHeight::Px`.
fn length_to_px(len: &Length, font_size: f32) -> f32 {
    match len {
        Length::Px(v) => *v,
        Length::Percent(v) => v * font_size / 100.0,
        Length::Add(a, b) => length_to_px(a, font_size) + length_to_px(b, font_size),
        Length::Sub(a, b) => length_to_px(a, font_size) - length_to_px(b, font_size),
        Length::Mul(a, f) => length_to_px(a, font_size) * f,
        Length::Div(a, f) => length_to_px(a, font_size) / f,
        _ => font_size * DEFAULT_LINE_FACTOR,
    }
}

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
    parent: InheritedCss,
    mut chain: ElementChain,
) -> (LayoutNode, InfoNode) {
    let html_node = dom.borrow().value.clone();

    let mut text_style = parent.text_style;
    let mut container_style = ContainerStyle::default();
    let mut style = Style::default();

    /* -----------------------------
       Build element chain
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
    }

    /* -----------------------------
       Collect CSS candidates
    ----------------------------- */
    let candidates: Option<HashMap<String, (CssValue, (u32, u32, u32), usize)>> =
        if let HtmlNodeType::Element { .. } = &html_node {
            Some(collect_candidates(resolved_styles, &chain))
        } else {
            None
        };

    /* -----------------------------
       Phase 1: Apply element-specific CSS declarations
    ----------------------------- */
    if let Some(candidates) = &candidates {
        for (name, (value, _, _)) in candidates {
            if name.starts_with("--") {
                continue;
            }
            apply_declaration(
                name,
                value,
                &mut style,
                &mut container_style,
                &mut text_style,
            );
        }
    }

    /* -----------------------------
       Phase 2: Resolve line-height using final font_size.
       text_style.line_height was either inherited from parent
       (via parent.text_style) or set by an explicit declaration
       in Phase 1 (via apply_declaration).
    ----------------------------- */
    style.line_height = match text_style.line_height {
        LineHeight::Number(factor) => Length::Px(text_style.font_size * factor),
        LineHeight::Normal => Length::Px(text_style.font_size * DEFAULT_LINE_FACTOR),
        LineHeight::Px(px) => Length::Px(px),
    };

    let child = InheritedCss { text_style };

    let (mut kind, inline_fragments_opt) = if let HtmlNodeType::Text(t) = &html_node {
        let t = normalize_whitespace(t);

        let _t = std::time::Instant::now();
        let measured = measurer
            .measure(&text::TextMeasureRequest {
                text: t.clone(),
                style: text_style,
            })
            .expect("text measure failed");
        let preview = if t.len() > 40 {
            let cut = t.floor_char_boundary(40);
            format!("{}...", &t[..cut])
        } else {
            t.clone()
        };
        log::info!(
            target: "Layouter",
            "measure inline: text={:?} len={} took={:?}",
            preview,
            t.len(),
            _t.elapsed(),
        );

        let kind = NodeKind::Text {
            texts: measured.iter().map(|f| f.text.clone()).collect(),
            style: text_style,
        };

        let inline_fragments: Vec<ItemFragment> = measured
            .into_iter()
            .map(|f| {
                ItemFragment::Fragment(Fragment {
                    width: f.width,
                    height: f.height,
                })
            })
            .collect();

        (kind, Some(inline_fragments))
    } else if let Some(name) = html_node.tag_name()
        && name == "a"
        && let Some(href) = html_node.get_attr("href")
    {
        (
            NodeKind::Container {
                scroll_x: false,
                scroll_y: false,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                style: container_style,
                role: ContainerRole::Link {
                    href: href.to_string(),
                },
            },
            None,
        )
    } else {
        (
            NodeKind::Container {
                scroll_x: false,
                scroll_y: false,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                style: container_style,
                role: ContainerRole::Normal,
            },
            None,
        )
    };

    // Process Children if there are no inline fragments (i.e. text nodes).
    let (layout, info) = if let Some(inline_fragments) = inline_fragments_opt {
        /* -----------------------------
           Text Node with inline fragments
        ----------------------------- */

        let style = Style {
            display: Display {
                outer: OuterDisplay::Inline,
                inner: InnerDisplay::Flow,
            },
            ..style
        };

        let layout = LayoutNode::with_children(style, inline_fragments);

        let info = InfoNode {
            kind,
            children: vec![],
        };

        (layout, info)
    } else {
        /* -----------------------------
           Children
        ----------------------------- */

        // NOTE:
        // Table 要素は未実装。
        // 暫定的に Flex に置き換える。
        // TODO: 将来的には TableLayout 実装に置き換える。
        let mut layout_children: Vec<LayoutChild> = Vec::new();
        let mut info_children = Vec::new();

        if style.display.outer != OuterDisplay::None {
            // Table 要素は暫定的に Flex に置き換える。
            match &html_node {
                HtmlNodeType::Element { tag_name, .. }
                    if tag_name == "table"
                        || tag_name == "tbody"
                        || tag_name == "thead"
                        || tag_name == "tfoot" =>
                {
                    style.display = Display {
                        outer: OuterDisplay::Block,
                        inner: InnerDisplay::Flex,
                    };
                    style.flex_direction = FlexDirection::Column;
                }
                HtmlNodeType::Element { tag_name, .. } if tag_name == "tr" => {
                    style.display = Display {
                        outer: OuterDisplay::Block,
                        inner: InnerDisplay::Flex,
                    };
                    style.flex_direction = FlexDirection::Row;
                }
                _ => {}
            }

            for child_dom in dom.borrow().children() {
                let child_node = child_dom.borrow().value.clone();

                if let HtmlNodeType::Text(t) = &child_node {
                    let t = normalize_whitespace(t);
                    let _t = std::time::Instant::now();
                    let request = &text::TextMeasureRequest {
                        text: t.clone(),
                        style: text_style,
                    };
                    let measured = measurer.measure(request).unwrap_or_else(|_|
                            // FallbackTextMeasurer won't return any errors.
                            FallbackTextMeasurer.measure(request).unwrap());
                    let preview = if t.len() > 40 {
                        let cut = t.floor_char_boundary(40);
                        format!("{}...", &t[..cut])
                    } else {
                        t.clone()
                    };
                    log::info!(
                        target: "Layouter",
                        "measure child: text={:?} len={} took={:?}",
                        preview,
                        t.len(),
                        _t.elapsed(),
                    );

                    let text_kind = NodeKind::Text {
                        texts: measured.iter().map(|f| f.text.clone()).collect(),
                        style: text_style,
                    };

                    for fragment in &measured {
                        layout_children.push(generate_fragment_node(fragment).into());
                    }

                    info_children.push(InfoNode {
                        kind: text_kind,
                        children: vec![],
                    });
                } else {
                    if child_dom.borrow().value.tag_name() == Some("br") {
                        layout_children.push(ItemFragment::LineBreak.into());
                        info_children.push(InfoNode {
                            kind: NodeKind::LineBreak,
                            children: vec![],
                        });
                        continue;
                    }

                    let (child_layout, child_info) = build_layout_and_info(
                        child_dom,
                        resolved_styles,
                        measurer,
                        child,
                        chain.clone(),
                    );

                    if dom.borrow().value.tag_name() == Some("html")
                        && child_dom.borrow().value.tag_name() == Some("body")
                        && let NodeKind::Container { style, .. } = &mut kind
                        && style.background == Background::Color(Color(0, 0, 0, 0))
                    {
                        let background = {
                            let NodeKind::Container { style, .. } = &child_info.kind else {
                                continue;
                            };
                            style.background.clone()
                        };
                        // html 要素の body 子要素に背景色が指定されていない場合、
                        // body の背景色を html の背景色で上書きする
                        style.background = background;
                    }

                    layout_children.push(child_layout.into());
                    info_children.push(child_info);
                }
            }
        }

        let layout = LayoutNode::with_children(style, layout_children);

        let info = InfoNode {
            kind,
            children: info_children,
        };

        (layout, info)
    };

    (layout, info)
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

fn generate_fragment_node(fragment: &MeasuredFragment) -> ItemFragment {
    if fragment.text == "\n" {
        ItemFragment::LineBreak
    } else {
        ItemFragment::Fragment(Fragment {
            width: fragment.width,
            height: fragment.height,
        })
    }
}

fn collect_candidates(
    resolved_styles: &ResolvedStyles,
    chain: &ElementChain,
) -> HashMap<String, (CssValue, (u32, u32, u32), usize)> {
    let mut candidates: HashMap<String, (CssValue, (u32, u32, u32), usize)> = HashMap::new();

    for decl in resolved_styles {
        if decl.selector.matches(chain) {
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
    fn expand_box<T: Clone, F>(
        name: &str,
        value: &CssValue,
        text_style: &TextStyle,
        resolve: &impl Fn(&str, &CssValue, &TextStyle) -> Option<T>,
        mut set: F,
    ) -> Option<()>
    where
        F: FnMut(T, T, T, T),
    {
        let resolve = |v: &CssValue| -> Option<T> { resolve(name, v, text_style) };

        match value {
            CssValue::List(values) => {
                let vals: Vec<T> = values.iter().map(resolve).collect::<Option<_>>()?;

                match vals.as_slice() {
                    [a] => set(a.clone(), a.clone(), a.clone(), a.clone()),
                    [v, h] => set(v.clone(), h.clone(), v.clone(), h.clone()),
                    [t, h, b] => set(t.clone(), h.clone(), b.clone(), h.clone()),
                    [t, r, b, l] => set(t.clone(), r.clone(), b.clone(), l.clone()),
                    _ => return None,
                }
            }

            _ => {
                let v = resolve(value)?;
                set(v.clone(), v.clone(), v.clone(), v);
            }
        }

        Some(())
    }

    fn parse_border_shorthand(
        name: &str,
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
            if width.is_none()
                && let Some(l) = resolve_css_len(name, token, text_style)
            {
                width = Some(l);
                continue;
            }

            // try as width keyword (thin/medium/thick). Check keywords before style keywords.
            if width.is_none()
                && let CssValue::Keyword(s) = token
            {
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

            // try as style keyword
            if style_v.is_none()
                && let CssValue::Keyword(s) = token
            {
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

            // try as color
            if color_v.is_none()
                && let Some(c) = resolve_css_color(name, token)
            {
                color_v = Some(c);
                continue;
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
            if let Some(parsed_display) = Display::from_css_name(v.as_str()) {
                style.display = parsed_display;
            }
        }

        /* ======================
         * Color / Text
         * ====================== */
        ("background-color", _) => {
            container_style.background = match value {
                CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("inherit") => {
                    Background::Color(text_style.color)
                }
                CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("currentColor") => {
                    Background::Color(text_style.color)
                }
                CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("initial") => {
                    Background::Color(Color(0, 0, 0, 0))
                }
                _ => Background::Color(resolve_css_color(name, value)?),
            };
        }

        ("background", _) => {
            container_style.background = parse_background_shorthand(name, value, text_style)?;
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
                _ => resolve_css_color(name, value)?,
            }
        }

        ("font-size", CssValue::Length(_, _)) => {
            // TODO: Add other size
            let len = resolve_css_len(name, value, text_style)?;
            let px = match &len {
                Length::Px(v) => *v,
                Length::Percent(v) => *v * text_style.font_size / 100.0,
                _ => {
                    log::error!(target: "Layouter", "Unknown size type for `{}`: {:?}", name, len);
                    return None;
                }
            };
            text_style.font_size = px;
        }

        ("line-height", CssValue::Number(factor)) => {
            text_style.line_height = LineHeight::Number(*factor);
            style.line_height = Length::Px(text_style.font_size * factor);
        }
        ("line-height", CssValue::Keyword(v)) if v == "normal" => {
            text_style.line_height = LineHeight::Normal;
            style.line_height = Length::Px(text_style.font_size * DEFAULT_LINE_FACTOR);
        }
        ("line-height", _) => {
            let len = resolve_css_len(name, value, text_style)?;
            text_style.line_height = LineHeight::Px(length_to_px(&len, text_style.font_size));
            style.line_height = len;
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
            expand_box(
                name,
                v,
                text_style,
                &|_, cv, ts| match cv {
                    CssValue::Keyword(s) if s == "auto" => Some(ui_layout::LengthOrAuto::Auto),
                    _ => resolve_css_len(name, cv, ts).map(ui_layout::LengthOrAuto::Length),
                },
                |t, r, b, l| {
                    style.spacing.margin_top = t;
                    style.spacing.margin_right = r;
                    style.spacing.margin_bottom = b;
                    style.spacing.margin_left = l;
                },
            )?;
        }
        ("margin-top", _) => {
            style.spacing.margin_top = resolve_css_len_auto(name, value, text_style)?;
        }
        ("margin-right", _) => {
            style.spacing.margin_right = resolve_css_len_auto(name, value, text_style)?;
        }
        ("margin-bottom", _) => {
            style.spacing.margin_bottom = resolve_css_len_auto(name, value, text_style)?;
        }
        ("margin-left", _) => {
            style.spacing.margin_left = resolve_css_len_auto(name, value, text_style)?;
        }

        ("border", v) => {
            let (maybe_width, maybe_style, maybe_color) = if let CssValue::Keyword(k) = v
                && (k.eq_ignore_ascii_case("inset") || k.eq_ignore_ascii_case("initial"))
            {
                (Some(Length::Px(0.0)), None, None)
            } else {
                parse_border_shorthand(name, v, text_style)?
            };

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
                container_style.border_color.top = c;
                container_style.border_color.right = c;
                container_style.border_color.bottom = c;
                container_style.border_color.left = c;
            }
        }
        ("border-top", _) => {
            let (maybe_width, maybe_style, maybe_color) =
                parse_border_shorthand(name, value, text_style)?;
            if let Some(w) = maybe_width {
                style.spacing.border_top = w;
            }
            if let Some(s) = maybe_style {
                container_style.border_style.top = s;
            }
            if let Some(c) = maybe_color {
                container_style.border_color.top = c;
            }
        }
        ("border-right", _) => {
            let (maybe_width, maybe_style, maybe_color) =
                parse_border_shorthand(name, value, text_style)?;
            if let Some(w) = maybe_width {
                style.spacing.border_right = w;
            }
            if let Some(s) = maybe_style {
                container_style.border_style.right = s;
            }
            if let Some(c) = maybe_color {
                container_style.border_color.right = c;
            }
        }
        ("border-bottom", _) => {
            let (maybe_width, maybe_style, maybe_color) =
                parse_border_shorthand(name, value, text_style)?;
            if let Some(w) = maybe_width {
                style.spacing.border_bottom = w;
            }
            if let Some(s) = maybe_style {
                container_style.border_style.bottom = s;
            }
            if let Some(c) = maybe_color {
                container_style.border_color.bottom = c;
            }
        }
        ("border-left", _) => {
            let (maybe_width, maybe_style, maybe_color) =
                parse_border_shorthand(name, value, text_style)?;
            if let Some(w) = maybe_width {
                style.spacing.border_left = w;
            }
            if let Some(s) = maybe_style {
                container_style.border_style.left = s;
            }
            if let Some(c) = maybe_color {
                container_style.border_color.left = c;
            }
        }

        ("padding", v) => {
            expand_box(
                name,
                v,
                text_style,
                &|_, v, ts| resolve_css_len(name, v, ts),
                |t, r, b, l| {
                    style.spacing.padding_top = t;
                    style.spacing.padding_right = r;
                    style.spacing.padding_bottom = b;
                    style.spacing.padding_left = l;
                },
            )?;
        }
        ("padding-top", _) => {
            style.spacing.padding_top = resolve_css_len(name, value, text_style)?;
        }
        ("padding-right", _) => {
            style.spacing.padding_right = resolve_css_len(name, value, text_style)?;
        }
        ("padding-bottom", _) => {
            style.spacing.padding_bottom = resolve_css_len(name, value, text_style)?;
        }
        ("padding-left", _) => {
            style.spacing.padding_left = resolve_css_len(name, value, text_style)?;
        }

        /* ======================
         * Size
         * ====================== */
        ("width", _) => {
            style.size.width = resolve_css_len_auto(name, value, text_style)?;
        }
        ("height", _) => {
            style.size.height = resolve_css_len_auto(name, value, text_style)?;
        }
        ("min-width", _) => {
            style.size.min_width = resolve_css_len_auto(name, value, text_style)?;
        }
        ("min-height", _) => {
            style.size.min_height = resolve_css_len_auto(name, value, text_style)?;
        }
        ("max-width", _) => {
            style.size.max_width = resolve_css_len_auto(name, value, text_style)?;
        }
        ("max-height", _) => {
            style.size.max_height = resolve_css_len_auto(name, value, text_style)?;
        }

        /* ======================
         * Flex
         * ====================== */
        ("flex-direction", CssValue::Keyword(v)) => {
            style.flex_direction = match v.as_str() {
                "row" => FlexDirection::Row,
                "column" => FlexDirection::Column,
                "row-reverse" => FlexDirection::RowReverse,
                "column-reverse" => FlexDirection::ColumnReverse,
                _ => return None,
            };
        }

        ("justify-content", CssValue::Keyword(v)) => {
            style.justify_content = match v.as_str() {
                "flex-start" | "start" => JustifyContent::Start,
                "center" => JustifyContent::Center,
                "flex-end" | "end" => JustifyContent::End,
                "space-between" => JustifyContent::SpaceBetween,
                "space-around" => JustifyContent::SpaceAround,
                "space-evenly" => JustifyContent::SpaceEvenly,
                _ => return None,
            };
        }

        ("align-items", CssValue::Keyword(v)) => {
            style.align_items = match v.as_str() {
                "stretch" => AlignItems::Stretch,
                "flex-start" | "start" => AlignItems::Start,
                "center" => AlignItems::Center,
                "flex-end" | "end" => AlignItems::End,
                _ => return None,
            };
        }

        ("gap", _) => match value {
            CssValue::List(l) if l.len() == 2 => {
                let mut l = l.iter();
                let gap = resolve_css_len_auto(name, l.next()?, text_style)?;
                style.row_gap = gap;
                let gap = resolve_css_len_auto(name, l.next()?, text_style)?;
                style.column_gap = gap;
            }
            CssValue::Length(_, _) => {
                let gap = resolve_css_len_auto(name, value, text_style)?;
                style.row_gap = gap.clone();
                style.column_gap = gap;
            }
            _ => {}
        },

        ("flex-grow", CssValue::Number(v)) => {
            style.item_style.flex_grow = *v;
        }

        ("flex-basis", _) => {
            style.item_style.flex_basis = resolve_css_len_auto(name, value, text_style)?;
        }

        _ => {
            // log::error!("{name}, {value:?}");
        }
    }
    Some(())
}

// =========================
//   Background Shorthand
// =========================

fn parse_background_shorthand(
    name: &str,
    value: &CssValue,
    text_style: &TextStyle,
) -> Option<Background> {
    let items: Vec<&CssValue> = match value {
        CssValue::List(values) => values.iter().collect(),
        _ => vec![value],
    };

    let mut maybe_color: Option<Color> = None;
    let mut maybe_gradient: Option<Gradient> = None;

    for v in items {
        // inherit
        if let CssValue::Keyword(kw) = v {
            if kw.eq_ignore_ascii_case("inherit") {
                maybe_color = Some(text_style.color);
                continue;
            }
            if kw.eq_ignore_ascii_case("currentColor") {
                maybe_color = Some(text_style.color);
                continue;
            }
        }

        if let CssValue::Number(0.0) = v {
            maybe_color = Some(Color(0, 0, 0, 0));
            continue;
        }

        // gradient
        if let CssValue::Function(fn_name, args) = v {
            if fn_name == "linear-gradient" || fn_name == "radial-gradient" {
                maybe_gradient = Some(parse_gradient(fn_name, args, text_style)?);
                continue;
            }
        }

        // color
        if let Some(c) = resolve_css_color(name, v) {
            maybe_color = Some(c);
            continue;
        }
    }

    if let Some(g) = maybe_gradient {
        return Some(Background::Gradient(g));
    }
    if let Some(c) = maybe_color {
        return Some(Background::Color(c));
    }

    None
}

// =========================
//   Gradient Parsing
// =========================

fn parse_gradient(fn_name: &str, args: &[CssValue], text_style: &TextStyle) -> Option<Gradient> {
    match fn_name {
        "linear-gradient" => parse_linear_gradient(args, text_style),
        "radial-gradient" => parse_radial_gradient(args, text_style),
        _ => None,
    }
}

fn parse_linear_gradient(args: &[CssValue], _text_style: &TextStyle) -> Option<Gradient> {
    if args.is_empty() {
        return None;
    }

    let (skip, angle) = parse_linear_direction(args);
    let angle = angle.unwrap_or(180.0);
    let stops = parse_color_stops(&args[skip..])?;

    Some(Gradient {
        kind: GradientKind::Linear { angle },
        stops,
    })
}

/// Returns (number_of_consumed_args, optional_angle_in_degrees).
fn parse_linear_direction(args: &[CssValue]) -> (usize, Option<f32>) {
    if args.is_empty() {
        return (0, None);
    }

    // <angle>
    if let CssValue::Length(v, Unit::Deg) = &args[0] {
        return (1, Some(*v));
    }

    // "to" <side-or-corner>
    if let CssValue::Keyword(k) = &args[0] {
        if k.as_str() == "to" && args.len() > 1 {
            let mut idx = 1;
            let mut sides: Vec<&str> = Vec::new();
            while idx < args.len() {
                if let CssValue::Keyword(k) = &args[idx] {
                    match k.as_str() {
                        "top" | "bottom" | "left" | "right" => {
                            sides.push(k.as_str());
                            idx += 1;
                        }
                        _ => break,
                    }
                } else {
                    break;
                }
            }
            if !sides.is_empty() {
                let angle = match sides.as_slice() {
                    ["top"] => Some(0.0),
                    ["top", "left"] => Some(315.0),
                    ["top", "right"] => Some(45.0),
                    ["bottom"] => Some(180.0),
                    ["bottom", "left"] => Some(225.0),
                    ["bottom", "right"] => Some(135.0),
                    ["left"] => Some(270.0),
                    ["right"] => Some(90.0),
                    _ => None,
                };
                return (idx, angle);
            }
        }
    }

    (0, None)
}

fn parse_radial_gradient(args: &[CssValue], _text_style: &TextStyle) -> Option<Gradient> {
    let mut shape = RadialShape::Ellipse;
    let mut size = RadialSizeKind::default();
    let mut position = (0.5f32, 0.5f32);

    let mut idx = 0;

    // Consume known radial keywords before color stops
    while idx < args.len() {
        if let CssValue::Keyword(k) = &args[idx] {
            match k.as_str() {
                "circle" => {
                    shape = RadialShape::Circle;
                    idx += 1;
                    continue;
                }
                "ellipse" => {
                    shape = RadialShape::Ellipse;
                    idx += 1;
                    continue;
                }
                "closest-side" => {
                    size = RadialSizeKind::ClosestSide;
                    idx += 1;
                    continue;
                }
                "farthest-side" => {
                    size = RadialSizeKind::FarthestSide;
                    idx += 1;
                    continue;
                }
                "closest-corner" => {
                    size = RadialSizeKind::ClosestCorner;
                    idx += 1;
                    continue;
                }
                "farthest-corner" => {
                    size = RadialSizeKind::FarthestCorner;
                    idx += 1;
                    continue;
                }
                _ => break,
            }
        } else {
            break;
        }
    }

    // Optional "at <position>" — simplified to "at center" / "at top left" etc.
    if idx < args.len() && args[idx] == CssValue::Keyword("at".into()) {
        idx += 1; // skip "at"
        if idx < args.len() {
            if let CssValue::Keyword(k) = &args[idx] {
                // Parse position keywords
                match k.as_str() {
                    "center" => position = (0.5, 0.5),
                    "top" => position = (0.5, 0.0),
                    "bottom" => position = (0.5, 1.0),
                    "left" => position = (0.0, 0.5),
                    "right" => position = (1.0, 0.5),
                    _ => {} // ignore unknown
                }
                idx += 1;
                // Optional second keyword (e.g. "top left")
                if idx < args.len() {
                    if let CssValue::Keyword(k2) = &args[idx] {
                        match (k.as_str(), k2.as_str()) {
                            ("top", "left") | ("left", "top") => position = (0.0, 0.0),
                            ("top", "right") | ("right", "top") => position = (1.0, 0.0),
                            ("bottom", "left") | ("left", "bottom") => position = (0.0, 1.0),
                            ("bottom", "right") | ("right", "bottom") => position = (1.0, 1.0),
                            _ => {}
                        }
                        idx += 1;
                    }
                }
            }
        }
    }

    let stops = parse_color_stops(&args[idx..])?;
    if stops.is_empty() {
        return None;
    }
    Some(Gradient {
        kind: GradientKind::Radial {
            shape,
            size,
            position,
        },
        stops,
    })
}

fn parse_color_stops(args: &[CssValue]) -> Option<Vec<ColorStop>> {
    let mut stops = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let color = resolve_css_color("gradient", &args[i])?;
        i += 1;

        let position = if i < args.len() {
            match &args[i] {
                CssValue::Length(v, Unit::Percent) => {
                    i += 1;
                    Some((*v / 100.0).clamp(0.0, 1.0))
                }
                CssValue::Length(_v, Unit::Px) => {
                    i += 1;
                    None
                }
                _ => None,
            }
        } else {
            None
        };

        stops.push(ColorStop { color, position });
    }

    Some(stops)
}

/// Resolve CssValue to LengthOrAuto.
fn resolve_css_len_auto(
    name: &str,
    css_len: &CssValue,
    text_style: &TextStyle,
) -> Option<LengthOrAuto> {
    match &css_len {
        CssValue::Keyword(s) if s == "auto" => Some(LengthOrAuto::Auto),
        _ => resolve_css_len(name, css_len, text_style).map(|l| l.into()),
    }
}

/// Resolve CssValue to Length.
fn resolve_css_len(name: &str, css_len: &CssValue, text_style: &TextStyle) -> Option<Length> {
    match &css_len {
        CssValue::Length(v, Unit::Em) => Some(Length::Px(text_style.font_size * v)),
        CssValue::Length(v, Unit::Rem) => Some(Length::Px(16.0 * v)), // html sont-size 仮値
        CssValue::Length(v, u) => match u {
            Unit::Percent => Some(Length::Percent(*v)),
            Unit::Px => Some(Length::Px(*v)),
            Unit::Vw => Some(Length::Vw(*v)),
            Unit::Vh => Some(Length::Vh(*v)),
            Unit::Em | Unit::Rem => unreachable!(),
            Unit::Deg => {
                log::error!(target: "Layouter", "Unexpected deg unit for `{}` (expected length)", name);
                return None;
            }
        },
        CssValue::Number(0.0) => Some(Length::Px(0.0)),
        CssValue::Keyword(_) => None,
        CssValue::Function(fn_name, args) if fn_name == "calc" && !args.is_empty() => {
            let mut iter = args.iter();
            let mut result = resolve_css_len(name, iter.next().unwrap(), text_style)?;

            while let (Some(op), Some(val)) = (iter.next(), iter.next()) {
                match op {
                    CssValue::Keyword(o) if o == "+" => {
                        let val_resolved = resolve_css_len(name, val, text_style)?;
                        result = Length::Add(Box::new(result), Box::new(val_resolved));
                    }
                    CssValue::Keyword(o) if o == "-" => {
                        let val_resolved = resolve_css_len(name, val, text_style)?;
                        result = Length::Sub(Box::new(result), Box::new(val_resolved));
                    }
                    CssValue::Keyword(o) if o == "*" => {
                        if let CssValue::Number(factor) = val {
                            result = Length::Mul(Box::new(result), *factor);
                        } else {
                            log::error!(target: "Layouter", "Invalid operand for multiplication in calc() for `{}`: {:?}", name, val);
                            return None;
                        }
                    }
                    CssValue::Keyword(o) if o == "/" => {
                        if let CssValue::Number(factor) = val {
                            if *factor == 0.0 {
                                log::error!(target: "Layouter", "Division by zero in calc() for `{}`", name);
                                return None;
                            }
                            result = Length::Div(Box::new(result), *factor);
                        } else {
                            log::error!(target: "Layouter", "Invalid operand for division in calc() for `{}`: {:?}", name, val);
                            return None;
                        }
                    }
                    _ => {
                        log::error!(target: "Layouter", "Unknown operator in calc() for `{}`: {:?}", name, op);
                        return None;
                    }
                }
            }

            Some(result)
        }
        CssValue::Color(_) => None,
        _ => {
            log::error!(target: "Layouter", "Unknown CSS Length type for `{}`: {:?}", name, css_len);
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
fn resolve_css_color(name: &str, css_color: &CssValue) -> Option<Color> {
    fn keyword_color_to_color(name: &str, keyword: &str) -> Option<Color> {
        // NOTE:
        // Keyword matching is case-insensitive according to CSS specs.
        // Keep this list limited to commonly used CSS Color Level 3 keywords.
        match keyword.to_ascii_lowercase().as_str() {
            // ===== Basic =====
            "black" => Some(Color(0, 0, 0, 255)),
            "silver" => Some(Color(192, 192, 192, 255)),
            "gray" | "grey" => Some(Color(128, 128, 128, 255)),
            "white" => Some(Color(255, 255, 255, 255)),

            // ===== Red =====
            "maroon" => Some(Color(128, 0, 0, 255)),
            "red" => Some(Color(255, 0, 0, 255)),
            "firebrick" => Some(Color(178, 34, 34, 255)),
            "crimson" => Some(Color(220, 20, 60, 255)),
            "indianred" => Some(Color(205, 92, 92, 255)),
            "lightcoral" => Some(Color(240, 128, 128, 255)),
            "salmon" => Some(Color(250, 128, 114, 255)),
            "darksalmon" => Some(Color(233, 150, 122, 255)),
            "lightsalmon" => Some(Color(255, 160, 122, 255)),

            // ===== Pink =====
            "pink" => Some(Color(255, 192, 203, 255)),
            "lightpink" => Some(Color(255, 182, 193, 255)),
            "hotpink" => Some(Color(255, 105, 180, 255)),
            "deeppink" => Some(Color(255, 20, 147, 255)),
            "palevioletred" => Some(Color(219, 112, 147, 255)),
            "magenta" | "fuchsia" => Some(Color(255, 0, 255, 255)),

            // ===== Orange =====
            "coral" => Some(Color(255, 127, 80, 255)),
            "tomato" => Some(Color(255, 99, 71, 255)),
            "orangered" => Some(Color(255, 69, 0, 255)),
            "orange" => Some(Color(255, 165, 0, 255)),

            // ===== Yellow =====
            "gold" => Some(Color(255, 215, 0, 255)),
            "yellow" => Some(Color(255, 255, 0, 255)),
            "lightyellow" => Some(Color(255, 255, 224, 255)),
            "lemonchiffon" => Some(Color(255, 250, 205, 255)),
            "lightgoldenrodyellow" => Some(Color(250, 250, 210, 255)),
            "papayawhip" => Some(Color(255, 239, 213, 255)),
            "moccasin" => Some(Color(255, 228, 181, 255)),

            // ===== Green =====
            "green" => Some(Color(0, 128, 0, 255)),
            "darkgreen" => Some(Color(0, 100, 0, 255)),
            "forestgreen" => Some(Color(34, 139, 34, 255)),
            "lime" => Some(Color(0, 255, 0, 255)),
            "limegreen" => Some(Color(50, 205, 50, 255)),
            "lightgreen" => Some(Color(144, 238, 144, 255)),
            "palegreen" => Some(Color(152, 251, 152, 255)),
            "springgreen" => Some(Color(0, 255, 127, 255)),
            "seagreen" => Some(Color(46, 139, 87, 255)),
            "mediumseagreen" => Some(Color(60, 179, 113, 255)),
            "yellowgreen" => Some(Color(154, 205, 50, 255)),

            // ===== Cyan / Aqua =====
            "aqua" | "cyan" => Some(Color(0, 255, 255, 255)),
            "lightcyan" => Some(Color(224, 255, 255, 255)),
            "paleturquoise" => Some(Color(175, 238, 238, 255)),
            "turquoise" => Some(Color(64, 224, 208, 255)),
            "mediumturquoise" => Some(Color(72, 209, 204, 255)),

            // ===== Blue =====
            "blue" => Some(Color(0, 0, 255, 255)),
            "mediumblue" => Some(Color(0, 0, 205, 255)),
            "darkblue" => Some(Color(0, 0, 139, 255)),
            "navy" => Some(Color(0, 0, 128, 255)),
            "royalblue" => Some(Color(65, 105, 225, 255)),
            "cornflowerblue" => Some(Color(100, 149, 237, 255)),
            "skyblue" => Some(Color(135, 206, 235, 255)),
            "lightblue" => Some(Color(173, 216, 230, 255)),
            "deepskyblue" => Some(Color(0, 191, 255, 255)),

            // ===== Purple =====
            "purple" => Some(Color(128, 0, 128, 255)),
            "indigo" => Some(Color(75, 0, 130, 255)),
            "violet" => Some(Color(238, 130, 238, 255)),
            "plum" => Some(Color(221, 160, 221, 255)),
            "orchid" => Some(Color(218, 112, 214, 255)),
            "mediumpurple" => Some(Color(147, 112, 219, 255)),
            "rebeccapurple" => Some(Color(102, 51, 153, 255)),

            // ===== Brown =====
            "brown" => Some(Color(165, 42, 42, 255)),
            "saddlebrown" => Some(Color(139, 69, 19, 255)),
            "sienna" => Some(Color(160, 82, 45, 255)),
            "chocolate" => Some(Color(210, 105, 30, 255)),
            "peru" => Some(Color(205, 133, 63, 255)),
            "burlywood" => Some(Color(222, 184, 135, 255)),

            // ===== White variations =====
            "snow" => Some(Color(255, 250, 250, 255)),
            "honeydew" => Some(Color(240, 255, 240, 255)),
            "mintcream" => Some(Color(245, 255, 250, 255)),
            "azure" => Some(Color(240, 255, 255, 255)),
            "aliceblue" => Some(Color(240, 248, 255, 255)),
            "ghostwhite" => Some(Color(248, 248, 255, 255)),

            // ===== Gray scale =====
            "gainsboro" => Some(Color(220, 220, 220, 255)),
            "lightgray" | "lightgrey" => Some(Color(211, 211, 211, 255)),
            "darkgray" | "darkgrey" => Some(Color(169, 169, 169, 255)),
            "dimgray" | "dimgrey" => Some(Color(105, 105, 105, 255)),
            "lightslategray" | "lightslategrey" => Some(Color(119, 136, 153, 255)),
            "slategray" | "slategrey" => Some(Color(112, 128, 144, 255)),

            // ===== CSS System Colors =====
            "buttonface" => Some(Color(240, 240, 240, 255)),
            "buttontext" => Some(Color(0, 0, 0, 255)),

            "linktext" => Some(Color(0, 0, 238, 255)),
            "visitedtext" => Some(Color(85, 26, 139, 255)),
            "activetext" => Some(Color(255, 0, 0, 255)),

            "canvas" => Some(Color(255, 255, 255, 255)),
            "canvastext" => Some(Color(0, 0, 0, 255)),

            "field" => Some(Color(255, 255, 255, 255)),
            "fieldtext" => Some(Color(0, 0, 0, 255)),

            "highlight" => Some(Color(0, 120, 215, 255)),
            "highlighttext" => Some(Color(255, 255, 255, 255)),

            "graytext" => Some(Color(128, 128, 128, 255)),

            // ===== Special =====
            "transparent" => Some(Color(0, 0, 0, 0)),
            "none" => Some(Color(0, 0, 0, 0)),

            _ => {
                log::error!(target: "Layouter", "Unknown CSS color keyword `{}` for `{}`", keyword, name);
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
        CssValue::Keyword(value) => keyword_color_to_color(name, value),

        // rgb() / rgba() unified
        CssValue::Function(func, args) if func == "rgb" || func == "rgba" => {
            // Extract numeric components, ignoring commas and handling '/'
            let mut numbers = Vec::new();
            let mut alpha: Option<f32> = None;
            let mut after_slash = false;

            for arg in args {
                match arg {
                    CssValue::Keyword(k) if k == "/" => {
                        after_slash = true;
                    }
                    CssValue::Number(n) => {
                        if after_slash {
                            alpha = Some(*n);
                        } else {
                            numbers.push(*n);
                        }
                    }
                    _ => return None,
                }
            }

            if numbers.len() != 3 {
                return None;
            }

            let a = alpha.unwrap_or(1.0);

            Some(Color(
                (numbers[0] * 255.0).round() as u8,
                (numbers[1] * 255.0).round() as u8,
                (numbers[2] * 255.0).round() as u8,
                (a * 255.0).round() as u8,
            ))
        }

        // hsl() / hsla() unified
        CssValue::Function(func, args) if func == "hsl" || func == "hsla" => {
            // Collect h, s, l and optional alpha
            let mut numbers = Vec::new();
            let mut alpha: Option<f32> = None;
            let mut after_slash = false;

            for arg in args {
                match arg {
                    CssValue::Keyword(k) if k == "/" => {
                        after_slash = true;
                    }
                    CssValue::Number(n) => {
                        if after_slash {
                            alpha = Some(*n);
                        } else {
                            numbers.push(*n);
                        }
                    }
                    _ => return None,
                }
            }

            if numbers.len() != 3 {
                return None;
            }

            let a = alpha.unwrap_or(1.0);
            let (r, g, b, a) = hsla_to_rgba(numbers[0], numbers[1], numbers[2], a);

            Some(Color(r, g, b, a))
        }

        // Any other value reaching here is a pipeline error
        _ => {
            log::error!(
                target: "Layouter",
                "Unexpected CSS color value for `{}` at layout stage: {:?}",
                name,
                css_color
            );
            None
        }
    }
}
