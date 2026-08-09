//! Layout builder, which transforms a DOM tree into a UI layout.

use crate::engine::bridge::text::{self, GlyphCluster};
use crate::engine::css::{
    matcher::{ElementChain, ElementInfo},
    values::{CssValue, Unit},
};
use crate::engine::html::HtmlNodeType;
use crate::engine::layouter::css_resolver::resolve_inline_value;
use crate::engine::layouter::dom_snapshot::{DomSnapshot, NodeId};
use crate::engine::layouter::types::VerticalAlign;
use crate::engine::tree::NodeRef;

use std::collections::HashMap;
use std::sync::Arc;

use ui_layout::{
    AlignContent, AlignItems, AutoSizeBehavior, BoxSizing, Display, FlexDirection, FlexWrap,
    GridRepeat, GridTrack, InnerDisplay, ItemFragment, JustifyContent, LayoutChild, LayoutNode,
    Length, LengthOrAuto, OuterDisplay, Position, Style,
};

use super::css_resolver::{ResolvedStyles, resolve_inline_style};
use super::text_layouter::TextFlowLayouter;
use super::types::{
    Background, BorderRadius, BorderStyle, Color, ColorScheme, ColorStop, ContainerRole,
    ContainerStyle, CornerRadius, FontStyle, FontWeight, Gradient, GradientKind, InfoNode,
    LineHeight, NodeKind, Overflow, RadialShape, RadialSizeKind, TextAlign, TextDecoration,
    TextStyle, TextTransform,
};
use crate::engine::renderer_model::Image;
use crate::engine::ui::custom_node_bridge::CustomNodeBridge;
use crate::engine::ui::registry::{ComponentRegistry, CustomNodeContext, DomWriteBack};

const DEFAULT_LINE_FACTOR: f32 = 1.2;

fn element_info(html_node: &HtmlNodeType) -> Option<ElementInfo> {
    let HtmlNodeType::Element {
        tag_name,
        attributes,
        ..
    } = html_node
    else {
        return None;
    };
    Some(ElementInfo {
        tag_name: tag_name.clone(),
        id: attributes
            .iter()
            .find(|attribute| attribute.name == "id")
            .map(|attribute| attribute.value.clone()),
        classes: attributes
            .iter()
            .find(|attribute| attribute.name == "class")
            .map(|attribute| {
                attribute
                    .value
                    .split_whitespace()
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        attributes: attributes
            .iter()
            .map(|attribute| (attribute.name.clone(), attribute.value.clone()))
            .collect(),
        element_index: 1,
        element_count: 1,
        type_index: 1,
        type_count: 1,
        previous_siblings: Arc::default(),
    })
}

fn element_sibling_infos(snapshot: &DomSnapshot, children: &[NodeId]) -> Vec<Option<ElementInfo>> {
    let mut type_counts = HashMap::<String, usize>::new();
    for &child in children {
        if let Some(tag_name) = snapshot.node(child).kind.tag_name() {
            *type_counts.entry(tag_name.to_string()).or_default() += 1;
        }
    }
    let element_count = type_counts.values().sum();

    let mut seen_types = HashMap::<String, usize>::new();
    let mut previous_siblings = Vec::new();
    let mut element_index = 0;
    children
        .iter()
        .map(|&child| {
            let mut info = match element_info(&snapshot.node(child).kind) {
                Some(info) => info,
                None => return None,
            };
            element_index += 1;
            let seen = seen_types.entry(info.tag_name.clone()).or_default();
            *seen += 1;
            info.element_index = element_index;
            info.element_count = element_count;
            info.type_index = *seen;
            info.type_count = type_counts[&info.tag_name];
            info.previous_siblings = Arc::from(previous_siblings.clone());

            let mut sibling = info.clone();
            sibling.previous_siblings = Arc::default();
            previous_siblings.push(sibling);
            Some(info)
        })
        .collect()
}

/// Inherited values from parent, passed down through the tree.
///
/// `text_style` carries all inherited text/line-height values.
/// `color_scheme` carries the element's used color scheme (resolved from the
/// `color-scheme` property and the system preference).
/// Add new fields here when additional deferred-resolution properties arise.
#[derive(Clone)]
pub struct InheritedCss {
    pub text_style: TextStyle,
    pub color_scheme: ColorScheme,
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
/// A single frame on the explicit processing stack.
struct StackFrame {
    dom: NodeId,
    chain: ElementChain,
    child: InheritedCss,
    kind: Option<NodeKind>,
    style: Option<Style>,
    child_slots: Vec<ChildSlot>,
    element_children: Vec<NodeId>,
}

enum ChildSlot {
    Inline(LayoutChild, InfoNode),
    Element(usize),
}

pub fn build_layout_and_info(
    dom: &NodeRef<HtmlNodeType>,
    resolved_styles: &ResolvedStyles,
    measurer: Arc<dyn text::TextMeasurer<TextStyle>>,
    parent: InheritedCss,
    chain: ElementChain,
    system_color_scheme: ColorScheme,
) -> (LayoutNode, InfoNode) {
    build_layout_and_info_with_images(
        dom,
        resolved_styles,
        measurer,
        parent,
        chain,
        system_color_scheme,
        &HashMap::new(),
    )
}

/// Builds layout and render trees with decoded images keyed by their `src` value.
pub fn build_layout_and_info_with_images(
    dom: &NodeRef<HtmlNodeType>,
    resolved_styles: &ResolvedStyles,
    measurer: Arc<dyn text::TextMeasurer<TextStyle>>,
    parent: InheritedCss,
    chain: ElementChain,
    system_color_scheme: ColorScheme,
    images: &HashMap<String, Image>,
) -> (LayoutNode, InfoNode) {
    let (snapshot, _dom_refs) = DomSnapshot::from_tree(dom);
    build_layout_and_info_from_snapshot(
        &snapshot,
        snapshot.roots()[0],
        resolved_styles,
        measurer,
        parent,
        chain,
        system_color_scheme,
        images,
        &HashMap::new(),
        None,
    )
}

/// Builds layout and render trees from a [`DomSnapshot`].
///
/// `write_back_sender` (when set) is cloned per text input so value changes
/// are reported as `(node id, value)` on the channel instead of mutating the
/// DOM directly, which allows this function to run off the UI thread.
///
/// `system_color_scheme` seeds the root element's used color scheme and is
/// used to resolve `color-scheme: light dark` and `light-dark()` values.
#[allow(clippy::too_many_arguments)]
pub fn build_layout_and_info_from_snapshot(
    snapshot: &DomSnapshot,
    root: NodeId,
    resolved_styles: &ResolvedStyles,
    measurer: Arc<dyn text::TextMeasurer<TextStyle>>,
    parent: InheritedCss,
    mut chain: ElementChain,
    system_color_scheme: ColorScheme,
    images: &HashMap<String, Image>,
    audio: &HashMap<String, Arc<[u8]>>,
    write_back_sender: Option<DomWriteBack>,
) -> (LayoutNode, InfoNode) {
    let registry = ComponentRegistry::new();
    /*
     * Build the initial element chain for the root node.
     */
    if let Some(info) = element_info(&snapshot.node(root).kind) {
        chain.insert(0, info);
    }

    // ── Explicit post-order stack (index-based to avoid borrow conflicts) ──

    let mut stack: Vec<StackFrame> = Vec::new();
    stack.push(StackFrame {
        dom: root,
        chain,
        child: InheritedCss {
            text_style: parent.text_style,
            color_scheme: system_color_scheme,
        },
        kind: None,
        style: None,
        child_slots: Vec::new(),
        element_children: Vec::new(),
    });

    let mut results: HashMap<NodeId, (LayoutNode, InfoNode)> = HashMap::new();

    // We use an index instead of .last_mut() so that push/pop don't conflict
    // with the mutable reference to the current frame.
    while let Some(top_idx) = {
        if stack.is_empty() {
            None
        } else {
            Some(stack.len() - 1)
        }
    } {
        // Phase check must happen BEFORE borrowing stack[top_idx] mutably.
        let is_entered = stack[top_idx].kind.is_some();

        if !is_entered {
            // ── ENTER phase ──────────────────────────────────────────────
            // Read frame state we need before taking any mutable action.
            let chain_for_css = stack[top_idx].chain.clone();
            let child_css = stack[top_idx].child.clone();

            let html_node = &snapshot.node(stack[top_idx].dom).kind;
            let mut text_style = child_css.text_style.clone();
            let mut container_style = ContainerStyle::default();
            let mut style = Style::default();
            let mut overflow = Overflow::default();

            // Collect CSS candidates.
            let candidates: Option<HashMap<_, _>> = if let HtmlNodeType::Element { .. } = html_node
            {
                Some(collect_candidates(resolved_styles, &chain_for_css))
            } else {
                None
            };

            // Resolve the used color scheme for this element. `light-dark()`
            // and system colors resolve against it, and it is inherited by
            // descendants that do not set `color-scheme` themselves.
            let used_color_scheme = {
                let declaration = candidates
                    .as_ref()
                    .and_then(|c| c.get("color-scheme"))
                    .map(|d| &d.value);
                resolve_used_color_scheme(declaration, child_css.color_scheme, system_color_scheme)
            };

            // Apply CSS declarations.
            if let Some(candidates) = &candidates {
                // The candidates map dedupes per property name (cascade winner),
                // but a shorthand and its longhand (e.g. `padding` and
                // `padding-top`) are distinct keys. Iterating a HashMap applies
                // them in random order, so re-sort by source order to make the
                // cascade deterministic.
                let mut candidates: Vec<_> = candidates.values().collect();
                candidates.sort_by_key(|declaration| declaration.order);
                for declaration in candidates {
                    let name = &declaration.name;
                    if !name.starts_with("--") {
                        apply_declaration(
                            name,
                            &declaration.value,
                            &mut style,
                            &mut container_style,
                            &mut text_style,
                            &mut overflow,
                            used_color_scheme,
                        );
                    }
                }
            }

            // Apply the element's inline `style` attribute. Inline styles are
            // author-origin declarations with the highest specificity, so they
            // override stylesheet rules — unless the stylesheet rule was
            // `!important`, which still wins over a non-`!important` inline
            // declaration.
            if let Some(style_attr) = html_node.get_attr("style") {
                for (name, value, important) in resolve_inline_style(style_attr) {
                    if name.starts_with("--") {
                        continue;
                    }
                    let stylesheet_important = candidates
                        .as_ref()
                        .and_then(|c| c.get(&name))
                        .is_some_and(|declaration| declaration.important);
                    if !important && stylesheet_important {
                        continue;
                    }
                    apply_declaration(
                        &name,
                        &value,
                        &mut style,
                        &mut container_style,
                        &mut text_style,
                        &mut overflow,
                        used_color_scheme,
                    );
                }
            }

            // Apply attribute sizing
            apply_attribute_dimensions(
                html_node,
                &mut style,
                &mut container_style,
                &mut text_style,
                &mut overflow,
                used_color_scheme,
            );

            // Absolutely positioned boxes are blockified before layout. The
            // inner display type remains unchanged.
            blockify_out_of_flow_positioned(&mut style);

            // Resolve line-height.
            style.line_height = match text_style.line_height {
                LineHeight::Number(factor) => Length::Px(text_style.font_size * factor),
                LineHeight::Normal => Length::Px(text_style.font_size * DEFAULT_LINE_FACTOR),
                LineHeight::Px(px) => Length::Px(px),
            };

            let child = InheritedCss {
                text_style: text_style.clone(),
                color_scheme: used_color_scheme,
            };

            if let HtmlNodeType::Text(t) = html_node {
                // ── Text node (leaf) ──
                let t = normalize_whitespace(t);
                let t = match text_style.text_transform {
                    TextTransform::None => t,
                    TextTransform::Uppercase => t.to_ascii_uppercase(),
                    TextTransform::Lowercase => t.to_ascii_lowercase(),
                };
                let Length::Px(line_height) = style.line_height else {
                    unreachable!()
                };
                let (layouter, kind) =
                    create_text_node(t, text_style.clone(), line_height, &*measurer);

                let mut inline_style = style.clone();
                inline_style.display = Display {
                    outer: OuterDisplay::Inline,
                    inner: InnerDisplay::Flow,
                };
                let layout =
                    LayoutNode::with_children(inline_style.clone(), [(inline_style, layouter)]);
                let info = InfoNode {
                    kind,
                    children: Vec::new(),
                    dom_id: Some(stack[top_idx].dom),
                };
                let ptr = stack[top_idx].dom;
                results.insert(ptr, (layout, info));
                stack.pop();
                continue;
            }

            // ── Custom / replaced element (leaf) ──
            if let Some(tag) = html_node.tag_name()
                && registry.tags().contains(&tag)
            {
                // Replaced elements (button/img/input) size by their intrinsic
                // content when auto-sized, not by filling the containing block.
                style.size.auto_behavior = AutoSizeBehavior::ShrinkToFit;
                let media_source = html_node.get_attr("src").map(str::to_string).or_else(|| {
                    snapshot
                        .children(stack[top_idx].dom)
                        .iter()
                        .find_map(|&child| {
                            let child = &snapshot.node(child).kind;
                            (child.tag_name() == Some("source"))
                                .then(|| child.get_attr("src").map(str::to_string))
                                .flatten()
                        })
                });
                let node = registry
                    .create(&CustomNodeContext {
                        tag,
                        inner_text: &snapshot.inner_text(stack[top_idx].dom),
                        media_source: media_source.as_deref(),
                        container_style: &container_style,
                        text_style: &text_style,
                        measurer: Arc::clone(&measurer),
                        images,
                        audio,
                        get_attr: &|name| html_node.get_attr(name).map(str::to_string),
                        write_back: write_back_sender
                            .as_ref()
                            .map(|sender| (sender.clone(), stack[top_idx].dom)),
                    })
                    .expect("registry must handle every tag it reports");

                let bridge = CustomNodeBridge::new(Arc::clone(&node), style.clone());
                let kind = NodeKind::Custom {
                    node,
                    scroll_x: overflow.x,
                    scroll_y: overflow.y,
                    scroll_offset_x: 0.0,
                    scroll_offset_y: 0.0,
                    style: container_style,
                    layout_style: style.clone(),
                    text_style: text_style.clone(),
                };
                let layout = LayoutNode::with_children(style.clone(), [(style, bridge)]);
                let info = InfoNode {
                    kind,
                    children: Vec::new(),
                    dom_id: Some(stack[top_idx].dom),
                };
                let ptr = stack[top_idx].dom;
                results.insert(ptr, (layout, info));
                stack.pop();
                continue;
            }

            // ── Element node ──
            let is_link = html_node.tag_name() == Some("a") && html_node.get_attr("href").is_some();

            let role = match html_node.tag_name() {
                Some("table") => ContainerRole::Table,
                Some("thead" | "tbody" | "tfoot") => ContainerRole::TableRowGroup,
                Some("tr") => ContainerRole::TableRow,
                Some("td" | "th") => ContainerRole::TableCell,
                Some("caption") => ContainerRole::TableCaption,
                _ if is_link => ContainerRole::Link {
                    href: html_node.get_attr("href").unwrap().to_string(),
                },
                _ => ContainerRole::Normal,
            };

            let kind = if is_link {
                NodeKind::Container {
                    scroll_x: overflow.x,
                    scroll_y: overflow.y,
                    scroll_offset_x: 0.0,
                    scroll_offset_y: 0.0,
                    style: container_style,
                    role,
                }
            } else {
                NodeKind::Container {
                    scroll_x: overflow.x,
                    scroll_y: overflow.y,
                    scroll_offset_x: 0.0,
                    scroll_offset_y: 0.0,
                    style: container_style,
                    role,
                }
            };

            // Table → flex overrides
            if let Some(tag) = html_node.tag_name() {
                match tag {
                    "table" | "tbody" | "thead" | "tfoot" => {
                        style.display = Display {
                            outer: OuterDisplay::Block,
                            inner: InnerDisplay::Flex,
                        };
                        style.flex_direction = FlexDirection::Column;
                    }
                    "tr" => {
                        style.display = Display {
                            outer: OuterDisplay::Block,
                            inner: InnerDisplay::Flex,
                        };
                        style.flex_direction = FlexDirection::Row;
                    }
                    _ => {}
                }
            }

            let mut child_slots: Vec<ChildSlot> = Vec::new();
            let mut element_kids: Vec<NodeId> = Vec::new();

            if style.display.outer != OuterDisplay::None {
                for &child in snapshot.children(stack[top_idx].dom) {
                    let child_node = &snapshot.node(child).kind;
                    if let HtmlNodeType::Text(t) = child_node {
                        let t = normalize_whitespace(t);
                        let t = match text_style.text_transform {
                            TextTransform::None => t,
                            TextTransform::Uppercase => t.to_ascii_uppercase(),
                            TextTransform::Lowercase => t.to_ascii_lowercase(),
                        };
                        let Length::Px(line_height) = style.line_height else {
                            unreachable!()
                        };
                        let (layouter, kind) =
                            create_text_node(t, text_style.clone(), line_height, &*measurer);
                        let mut inline_style = style.clone();
                        inline_style.display = Display {
                            outer: OuterDisplay::Inline,
                            inner: InnerDisplay::Flow,
                        };
                        child_slots.push(ChildSlot::Inline(
                            (inline_style, layouter).into(),
                            InfoNode {
                                kind,
                                children: Vec::new(),
                                dom_id: Some(child),
                            },
                        ));
                    } else if child_node.tag_name() == Some("br") {
                        child_slots.push(ChildSlot::Inline(
                            ItemFragment::LineBreak.into(),
                            InfoNode {
                                kind: NodeKind::LineBreak,
                                children: Vec::new(),
                                dom_id: Some(child),
                            },
                        ));
                    } else {
                        child_slots.push(ChildSlot::Element(element_kids.len()));
                        element_kids.push(child);
                    }
                }
            }

            if element_kids.is_empty() {
                // ── No element children → leaf, build immediately ──
                let (layout_children, info_children): (Vec<_>, Vec<_>) = child_slots
                    .into_iter()
                    .filter_map(|slot| match slot {
                        ChildSlot::Inline(layout, info) => Some((layout, info)),
                        ChildSlot::Element(_) => None,
                    })
                    .unzip();
                let layout = LayoutNode::with_children(style.clone(), layout_children);
                let info = InfoNode {
                    kind,
                    children: info_children,
                    dom_id: Some(stack[top_idx].dom),
                };
                let ptr = stack[top_idx].dom;
                results.insert(ptr, (layout, info));
                stack.pop();
            } else {
                // ── Has element children → save state, push children ──
                let parent_chain = stack[top_idx].chain.clone();
                stack[top_idx].kind = Some(kind);
                stack[top_idx].style = Some(style);
                stack[top_idx].child = child;
                stack[top_idx].child_slots = child_slots;
                stack[top_idx].element_children = element_kids;

                // Build child chains and push frames.
                // Clone element_kids before the immutable borrow below
                // so we don't hold &mut stack[] while pushing.
                let kids_for_push: Vec<_> = {
                    let f = &stack[top_idx];
                    f.element_children.clone()
                };
                let child_css = stack[top_idx].child.clone();
                let kid_infos = element_sibling_infos(snapshot, &kids_for_push);
                for (&kid, info) in kids_for_push.iter().zip(kid_infos).rev() {
                    let mut kid_chain = parent_chain.clone();
                    if let Some(info) = info {
                        kid_chain.insert(0, info);
                    }
                    stack.push(StackFrame {
                        dom: kid,
                        chain: kid_chain,
                        child: child_css.clone(),
                        kind: None,
                        style: None,
                        child_slots: Vec::new(),
                        element_children: Vec::new(),
                    });
                }
            }
        } else {
            // ── EXIT phase ────────────────────────────────────────────────
            // Take ownership of frame data for building results.
            let frame = stack.swap_remove(top_idx);

            let style = frame.style.as_ref().unwrap().clone();
            let kind = frame.kind.as_ref().unwrap().clone();

            // Collect element children results.
            let mut element_results: Vec<(LayoutChild, InfoNode)> = Vec::new();

            for &kid in &frame.element_children {
                if let Some((child_layout, child_info)) = results.remove(&kid) {
                    element_results.push((child_layout.into(), child_info));
                }
            }

            // Handle html→body background inheritance.
            let mut final_kind = kind;
            if snapshot.node(frame.dom).kind.tag_name() == Some("html") {
                let should_inherit = final_kind.is_container_with_transparent_bg();
                if should_inherit {
                    for (i, &kid) in frame.element_children.iter().enumerate() {
                        if snapshot.node(kid).kind.tag_name() == Some("body")
                            && i < element_results.len()
                        {
                            let child_bg = element_results[i].1.kind.container_bg();
                            if let Some(bg) = child_bg
                                && let NodeKind::Container { ref mut style, .. } = final_kind
                            {
                                style.background = bg.clone();
                            }
                        }
                    }
                }
            }

            let mut element_results: Vec<_> = element_results.into_iter().map(Some).collect();
            let mut all_layout: Vec<LayoutChild> = Vec::with_capacity(frame.child_slots.len());
            let mut all_info: Vec<InfoNode> = Vec::with_capacity(frame.child_slots.len());

            for slot in frame.child_slots {
                let (lc, ic) = match slot {
                    ChildSlot::Inline(layout, info) => (layout, info),
                    ChildSlot::Element(index) => element_results[index]
                        .take()
                        .expect("element child result must exist"),
                };
                all_layout.push(lc);
                all_info.push(ic);
            }

            let layout = LayoutNode::with_children(style, all_layout);
            let info = InfoNode {
                kind: final_kind,
                children: all_info,
                dom_id: Some(frame.dom),
            };
            let ptr = frame.dom;
            results.insert(ptr, (layout, info));
        }
    }

    results
        .remove(&root)
        .expect("root must have been processed")
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

/// Measure text and create a [`TextFlowLayouter`] + [`NodeKind::Text`].
///
/// Falls back to unshaped measurement when shaped measurement fails.
fn create_text_node(
    text: String,
    text_style: TextStyle,
    line_height: f32,
    measurer: &dyn text::TextMeasurer<TextStyle>,
) -> (TextFlowLayouter, NodeKind) {
    let _t = std::time::Instant::now();
    let request = text::TextMeasureRequest {
        text: text.clone(),
        style: text_style.clone(),
    };
    let clusters = measurer.measure_shaped(&request).unwrap_or_else(|_| {
        measurer
            .measure(&request)
            .map(|ms| {
                let mut offset = 0usize;
                ms.into_iter()
                    .map(|f| {
                        // find this fragment's byte offset in the original text
                        let pos = text[offset..]
                            .find(&f.text)
                            .map(|p| offset + p)
                            .unwrap_or(offset);
                        offset = pos + f.text.len();
                        GlyphCluster {
                            byte_offset: pos,
                            width: f.width,
                            break_allowed: true,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    });
    let preview = if text.len() > 40 {
        let cut = text.floor_char_boundary(40);
        format!("{}...", &text[..cut])
    } else {
        text.clone()
    };
    log::info!(
        target: "Layouter",
        "measure_shaped: text={:?} len={} took={:?}",
        preview,
        text.len(),
        _t.elapsed(),
    );

    let layouter = TextFlowLayouter::new(text.clone(), text_style.clone(), clusters, line_height);
    let kind = NodeKind::Text {
        text,
        style: text_style,
        text_id: layouter.id,
    };
    (layouter, kind)
}

/// How an element's `color-scheme` property constrains its used color scheme.
enum ColorSchemePref {
    /// `normal` or unset: fall back to the inherited (or system) scheme.
    Normal,
    Light,
    Dark,
    /// Both `light` and `dark` listed: follow the system preference.
    Both,
}

/// Parses the winning `color-scheme` declaration into a preference.
fn color_scheme_pref(value: Option<&CssValue>) -> ColorSchemePref {
    let Some(value) = value else {
        return ColorSchemePref::Normal;
    };
    let mut light = false;
    let mut dark = false;
    let mut has = false;
    let mut push = |keyword: &str| {
        has = true;
        match keyword {
            "light" => light = true,
            "dark" => dark = true,
            // `only`, `normal`, unknown keywords are ignored here.
            _ => {}
        }
    };
    match value {
        CssValue::Keyword(k) => push(k),
        CssValue::List(items) => {
            for item in items {
                if let CssValue::Keyword(k) = item {
                    push(k);
                }
            }
        }
        _ => {}
    }
    if !has {
        ColorSchemePref::Normal
    } else if light && dark {
        ColorSchemePref::Both
    } else if light {
        ColorSchemePref::Light
    } else if dark {
        ColorSchemePref::Dark
    } else {
        ColorSchemePref::Normal
    }
}

/// Computes the used color scheme of an element from its `color-scheme`
/// declaration, the inherited scheme, and the system preference.
fn resolve_used_color_scheme(
    declaration: Option<&CssValue>,
    inherited: ColorScheme,
    system: ColorScheme,
) -> ColorScheme {
    match color_scheme_pref(declaration) {
        ColorSchemePref::Normal => inherited,
        ColorSchemePref::Light => ColorScheme::Light,
        ColorSchemePref::Dark => ColorScheme::Dark,
        ColorSchemePref::Both => system,
    }
}

fn collect_candidates(
    resolved_styles: &ResolvedStyles,
    chain: &ElementChain,
) -> HashMap<String, super::css_resolver::ResolvedDeclaration> {
    let mut candidates = HashMap::new();

    for decl in resolved_styles {
        if decl.selector.matches(chain) {
            let entry = candidates.get(&decl.name);

            let should_replace = match entry {
                Some(current) => decl.outranks(current),
                None => true,
            };

            if should_replace {
                candidates.insert(decl.name.clone(), decl.clone());
            }
        }
    }

    candidates
}

fn apply_attribute_dimensions(
    html_node: &HtmlNodeType,
    style: &mut Style,
    container_style: &mut ContainerStyle,
    text_style: &mut TextStyle,
    overflow: &mut Overflow,
    color_scheme: ColorScheme,
) {
    fn apply_attribute_size(
        attr: &str,
        html_node: &HtmlNodeType,
        style: &mut Style,
        container_style: &mut ContainerStyle,
        text_style: &mut TextStyle,
        overflow: &mut Overflow,
        color_scheme: ColorScheme,
    ) {
        if let Some(value) = html_node.get_attr(attr)
            && let Some(mut value) = resolve_inline_value(value)
        {
            if let CssValue::Number(v) = value {
                value = CssValue::Length(v, Unit::Px);
            }

            apply_declaration(
                attr,
                &value,
                style,
                container_style,
                text_style,
                overflow,
                color_scheme,
            );
        }
    }

    apply_attribute_size(
        "width",
        html_node,
        style,
        container_style,
        text_style,
        overflow,
        color_scheme,
    );

    apply_attribute_size(
        "height",
        html_node,
        style,
        container_style,
        text_style,
        overflow,
        color_scheme,
    );
}

fn blockify_out_of_flow_positioned(style: &mut Style) {
    if style.position.kind.is_out_of_flow() && style.display.outer == OuterDisplay::Inline {
        style.display.outer = OuterDisplay::Block;
    }
}

fn resolve_flex_shorthand(
    value: &CssValue,
    text_style: &TextStyle,
) -> Option<(f32, f32, LengthOrAuto)> {
    let zero_basis = LengthOrAuto::Length(Length::Percent(0.0));

    match value {
        CssValue::Keyword(keyword) => match keyword.as_str() {
            "none" => Some((0.0, 0.0, LengthOrAuto::Auto)),
            "auto" => Some((1.0, 1.0, LengthOrAuto::Auto)),
            "initial" => Some((0.0, 1.0, LengthOrAuto::Auto)),
            _ => None,
        },
        CssValue::Number(grow) if *grow >= 0.0 => Some((*grow, 1.0, zero_basis)),
        CssValue::Length(_, _) => {
            Some((1.0, 1.0, resolve_css_len_auto("flex", value, text_style)?))
        }
        CssValue::List(values) => match values.as_slice() {
            [CssValue::Number(grow), CssValue::Number(shrink)]
                if *grow >= 0.0 && *shrink >= 0.0 =>
            {
                Some((*grow, *shrink, zero_basis))
            }
            [CssValue::Number(grow), basis] if *grow >= 0.0 => {
                Some((*grow, 1.0, resolve_css_len_auto("flex", basis, text_style)?))
            }
            [CssValue::Number(grow), CssValue::Number(shrink), basis]
                if *grow >= 0.0 && *shrink >= 0.0 =>
            {
                Some((
                    *grow,
                    *shrink,
                    resolve_css_len_auto("flex", basis, text_style)?,
                ))
            }
            _ => None,
        },
        _ => None,
    }
}

fn flex_direction_keyword(keyword: &str) -> Option<FlexDirection> {
    match keyword {
        "row" => Some(FlexDirection::Row),
        "column" => Some(FlexDirection::Column),
        "row-reverse" => Some(FlexDirection::RowReverse),
        "column-reverse" => Some(FlexDirection::ColumnReverse),
        _ => None,
    }
}

fn flex_wrap_keyword(keyword: &str) -> Option<FlexWrap> {
    match keyword {
        "nowrap" => Some(FlexWrap::NoWrap),
        "wrap" => Some(FlexWrap::Wrap),
        "wrap-reverse" => Some(FlexWrap::WrapReverse),
        _ => None,
    }
}

fn resolve_flex_flow(value: &CssValue) -> Option<(FlexDirection, FlexWrap)> {
    let values = match value {
        CssValue::Keyword(keyword) if keyword == "initial" => {
            return Some((FlexDirection::Row, FlexWrap::NoWrap));
        }
        CssValue::Keyword(_) => std::slice::from_ref(value),
        CssValue::List(values) if (1..=2).contains(&values.len()) => values.as_slice(),
        _ => return None,
    };
    let mut direction = None;
    let mut wrap = None;
    for value in values {
        let CssValue::Keyword(keyword) = value else {
            return None;
        };
        if let Some(parsed) = flex_direction_keyword(keyword) {
            if direction.replace(parsed).is_some() {
                return None;
            }
        } else if let Some(parsed) = flex_wrap_keyword(keyword) {
            if wrap.replace(parsed).is_some() {
                return None;
            }
        } else {
            return None;
        }
    }

    Some((
        direction.unwrap_or(FlexDirection::Row),
        wrap.unwrap_or(FlexWrap::NoWrap),
    ))
}

pub fn apply_declaration(
    name: &str,
    value: &CssValue,
    style: &mut Style,
    container_style: &mut ContainerStyle,
    text_style: &mut TextStyle,
    overflow: &mut Overflow,
    color_scheme: ColorScheme,
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

    /// Split a border-radius value list on the `Keyword("/")` separator into
    /// its horizontal and vertical components. Without a slash the vertical
    /// list is empty (meaning "use the horizontal value").
    fn split_radius_lists<'a>(value: &'a CssValue) -> (Vec<&'a CssValue>, Vec<&'a CssValue>) {
        match value {
            CssValue::List(vals) => {
                let mut horiz: Vec<&'a CssValue> = Vec::new();
                let mut vert: Vec<&'a CssValue> = Vec::new();
                let mut after_slash = false;
                for v in vals {
                    if let CssValue::Keyword(k) = v
                        && k == "/"
                    {
                        after_slash = true;
                        continue;
                    }
                    if after_slash {
                        vert.push(v);
                    } else {
                        horiz.push(v);
                    }
                }
                (horiz, vert)
            }
            _ => (vec![value], Vec::new()),
        }
    }

    /// Expand a 1/2/3/4-value list to per-corner lengths in CSS order
    /// [top-left, top-right, bottom-right, bottom-left].
    fn expand_radius_axis(
        name: &str,
        values: &[&CssValue],
        text_style: &TextStyle,
    ) -> Option<[Length; 4]> {
        let vals: Vec<Length> = values
            .iter()
            .map(|v| resolve_css_len(name, v, text_style))
            .collect::<Option<_>>()?;
        match vals.as_slice() {
            [a] => Some([a.clone(), a.clone(), a.clone(), a.clone()]),
            [v, h] => Some([v.clone(), h.clone(), v.clone(), h.clone()]),
            [t, h, b] => Some([t.clone(), h.clone(), b.clone(), h.clone()]),
            [t, r, b, l] => Some([t.clone(), r.clone(), b.clone(), l.clone()]),
            _ => None,
        }
    }

    /// Parse a `border-radius` shorthand value (1-4 lengths per axis, optional
    /// elliptical `/` form) into the four corners.
    fn parse_border_radius_shorthand(
        name: &str,
        value: &CssValue,
        text_style: &TextStyle,
    ) -> Option<(CornerRadius, CornerRadius, CornerRadius, CornerRadius)> {
        let (horiz, vert) = split_radius_lists(value);
        let h = expand_radius_axis(name, &horiz, text_style)?;
        let v = if vert.is_empty() {
            h.clone()
        } else {
            expand_radius_axis(name, &vert, text_style)?
        };
        Some((
            CornerRadius {
                x: h[0].clone(),
                y: v[0].clone(),
            },
            CornerRadius {
                x: h[1].clone(),
                y: v[1].clone(),
            },
            CornerRadius {
                x: h[2].clone(),
                y: v[2].clone(),
            },
            CornerRadius {
                x: h[3].clone(),
                y: v[3].clone(),
            },
        ))
    }

    /// Parse a single-corner radius value: one length, two lengths (`rx ry`)
    /// or the elliptical `rx / ry` form.
    fn parse_corner_radius(
        name: &str,
        value: &CssValue,
        text_style: &TextStyle,
    ) -> Option<CornerRadius> {
        let (horiz, vert) = split_radius_lists(value);
        let x = expand_radius_axis(name, &horiz, text_style)?;
        let y = if vert.is_empty() {
            x.clone()
        } else {
            expand_radius_axis(name, &vert, text_style)?
        };
        Some(CornerRadius {
            x: x[0].clone(),
            y: y[0].clone(),
        })
    }

    /// Whether a single `overflow` keyword enables scrolling on an axis.
    fn overflow_scrollable(keyword: &str) -> Option<bool> {
        match keyword {
            "visible" | "clip" => Some(false),
            "hidden" | "scroll" | "auto" => Some(true),
            _ => None,
        }
    }

    /// Expand an `overflow` shorthand (1 or 2 keywords) into per-axis flags.
    fn overflow_flags(value: &CssValue) -> Option<(bool, bool)> {
        match value {
            CssValue::Keyword(k) => overflow_scrollable(k).map(|b| (b, b)),
            CssValue::List(l) => {
                let x = match l.first()? {
                    CssValue::Keyword(k) => overflow_scrollable(k)?,
                    _ => return None,
                };
                let y = match l.get(1) {
                    Some(CssValue::Keyword(k)) => overflow_scrollable(k)?,
                    Some(_) => return None,
                    None => x,
                };
                Some((x, y))
            }
            _ => None,
        }
    }

    fn parse_border_shorthand(
        name: &str,
        value: &CssValue,
        text_style: &TextStyle,
        color_scheme: ColorScheme,
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
                    "inset" | "outset" | "groove" | "ridge" | "double" | "hidden" => {
                        // stub
                        style_v = Some(BorderStyle::Solid);
                        continue;
                    }
                    _ => None,
                };

                if let Some(p) = parsed {
                    style_v = Some(p);
                    continue;
                }
            }

            // try as color
            if color_v.is_none()
                && let Some(c) = resolve_css_color(name, token, color_scheme)
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
            style.display = Display::from_css_name(v.as_str())?;
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
                CssValue::Keyword(kw)
                    if kw.eq_ignore_ascii_case("initial") || kw.eq_ignore_ascii_case("unset") =>
                {
                    Background::Color(Color(0, 0, 0, 0))
                }
                _ => Background::Color(resolve_css_color(name, value, color_scheme)?),
            };
        }

        ("background", _) => {
            container_style.background =
                parse_background_shorthand(name, value, text_style, color_scheme)?;
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
                _ => resolve_css_color(name, value, color_scheme)?,
            }
        }

        // The used color scheme is computed before declarations are applied
        // (see `resolve_used_color_scheme`); this arm just accepts the
        // property so it is not treated as unsupported.
        ("color-scheme", _) => {}

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
        }
        ("line-height", CssValue::Keyword(v)) if v == "normal" => {
            text_style.line_height = LineHeight::Normal;
        }
        ("line-height", _) => {
            let len = resolve_css_len(name, value, text_style)?;
            text_style.line_height = LineHeight::Px(length_to_px(&len, text_style.font_size));
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

        ("font-family", _) => {
            let families = extract_font_families(value);
            if !families.is_empty() {
                text_style.font_families = families;
            }
        }

        ("text-decoration", v) => {
            let items: Vec<&CssValue> = match v {
                CssValue::List(list) => list.iter().collect(),
                _ => vec![v],
            };
            for item in items {
                match item {
                    CssValue::Keyword(k) => match k.as_str() {
                        "none" => text_style.text_decoration = TextDecoration::None,
                        "underline" => text_style.text_decoration = TextDecoration::Underline,
                        "line-through" => text_style.text_decoration = TextDecoration::LineThrough,
                        "overline" => text_style.text_decoration = TextDecoration::Overline,
                        _ => {}
                    },
                    _ => {
                        if let Some(c) = resolve_css_color(name, item, color_scheme) {
                            text_style.text_decoration_color = Some(c);
                        }
                    }
                }
            }
        }

        ("text-decoration-color", _) => {
            if let Some(c) = resolve_css_color(name, value, color_scheme) {
                text_style.text_decoration_color = Some(c);
            }
        }

        ("vertical-align", CssValue::Keyword(v)) => {
            match v.as_str() {
                "sub" => {
                    text_style.vertical_align = VerticalAlign::Sub;
                }
                "super" | "sup" => {
                    text_style.vertical_align = VerticalAlign::Super;
                }
                "top" | "middle" | "bottom" => {
                    // Keep default, ignore for now
                }
                _ => {}
            }
        }

        ("text-transform", CssValue::Keyword(v)) => {
            text_style.text_transform = match v.as_str() {
                "none" => TextTransform::None,
                "uppercase" => TextTransform::Uppercase,
                "lowercase" => TextTransform::Lowercase,
                _ => TextTransform::None,
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
                parse_border_shorthand(name, v, text_style, color_scheme)?
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
                parse_border_shorthand(name, value, text_style, color_scheme)?;
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
                parse_border_shorthand(name, value, text_style, color_scheme)?;
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
                parse_border_shorthand(name, value, text_style, color_scheme)?;
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
                parse_border_shorthand(name, value, text_style, color_scheme)?;
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

        ("border-radius", v) => {
            let (tl, tr, br, bl) = parse_border_radius_shorthand(name, v, text_style)?;
            container_style.border_radius = BorderRadius {
                top_left: tl,
                top_right: tr,
                bottom_right: br,
                bottom_left: bl,
            };
        }
        ("border-top-left-radius", v) => {
            container_style.border_radius.top_left = parse_corner_radius(name, v, text_style)?;
        }
        ("border-top-right-radius", v) => {
            container_style.border_radius.top_right = parse_corner_radius(name, v, text_style)?;
        }
        ("border-bottom-right-radius", v) => {
            container_style.border_radius.bottom_right = parse_corner_radius(name, v, text_style)?;
        }
        ("border-bottom-left-radius", v) => {
            container_style.border_radius.bottom_left = parse_corner_radius(name, v, text_style)?;
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
        ("aspect-ratio", _) => {
            style.size.aspect_ratio = match value {
                CssValue::Keyword(v) if v == "auto" => None,
                CssValue::Number(v) if *v > 0.0 => Some(*v),
                CssValue::List(l) => {
                    let mut nums = l.iter().filter_map(|v| match v {
                        CssValue::Number(n) if *n > 0.0 => Some(*n),
                        _ => None,
                    });
                    let w = nums.next()?;
                    let h = nums.next()?;
                    Some(w / h)
                }
                _ => return None,
            };
        }

        /* ======================
         * Position
         * ====================== */
        ("position", _) => {
            style.position.kind = match value {
                CssValue::Keyword(v) => match v.to_ascii_lowercase().as_str() {
                    "static" => Position::Static,
                    "relative" => Position::Relative,
                    "absolute" => Position::Absolute,
                    "fixed" => Position::Fixed,
                    _ => return None,
                },
                _ => return None,
            };
        }
        ("top", _) => {
            style.position.top = resolve_css_len_auto(name, value, text_style)?;
        }
        ("right", _) => {
            style.position.right = resolve_css_len_auto(name, value, text_style)?;
        }
        ("bottom", _) => {
            style.position.bottom = resolve_css_len_auto(name, value, text_style)?;
        }
        ("left", _) => {
            style.position.left = resolve_css_len_auto(name, value, text_style)?;
        }
        ("inset", v) => {
            expand_box(
                name,
                v,
                text_style,
                &|_, cv, ts| match cv {
                    CssValue::Keyword(s) if s.eq_ignore_ascii_case("auto") => {
                        Some(ui_layout::LengthOrAuto::Auto)
                    }
                    _ => resolve_css_len(name, cv, ts).map(ui_layout::LengthOrAuto::Length),
                },
                |t, r, b, l| {
                    style.position.top = t;
                    style.position.right = r;
                    style.position.bottom = b;
                    style.position.left = l;
                },
            )?;
        }

        /* ======================
         * Overflow
         * ====================== */
        ("overflow", _) => {
            let (x, y) = overflow_flags(value)?;
            overflow.x = x;
            overflow.y = y;
        }
        ("overflow-x", CssValue::Keyword(k)) => {
            overflow.x = overflow_scrollable(k)?;
        }
        ("overflow-y", CssValue::Keyword(k)) => {
            overflow.y = overflow_scrollable(k)?;
        }

        /* ======================
         * Flex
         * ====================== */
        ("flex-direction", CssValue::Keyword(v)) => {
            style.flex_direction = flex_direction_keyword(v)?;
        }

        ("flex-wrap", CssValue::Keyword(v)) => {
            style.flex_wrap = flex_wrap_keyword(v)?;
        }

        ("flex-flow", _) => {
            let (direction, wrap) = resolve_flex_flow(value)?;
            style.flex_direction = direction;
            style.flex_wrap = wrap;
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

        ("align-content", CssValue::Keyword(v)) => {
            style.align_content = match v.as_str() {
                "normal" | "stretch" => AlignContent::Stretch,
                "flex-start" | "start" => AlignContent::Start,
                "center" => AlignContent::Center,
                "flex-end" | "end" => AlignContent::End,
                "space-between" => AlignContent::SpaceBetween,
                "space-around" => AlignContent::SpaceAround,
                "space-evenly" => AlignContent::SpaceEvenly,
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

        ("flex", _) => {
            let (grow, shrink, basis) = resolve_flex_shorthand(value, text_style)?;
            style.item_style.flex_grow = grow;
            style.item_style.flex_shrink = shrink;
            style.item_style.flex_basis = basis;
        }

        ("flex-grow", CssValue::Number(v)) => {
            if *v < 0.0 {
                return None;
            }
            style.item_style.flex_grow = *v;
        }

        ("flex-basis", _) => {
            style.item_style.flex_basis = resolve_css_len_auto(name, value, text_style)?;
        }

        ("flex-shrink", CssValue::Number(v)) => {
            if *v < 0.0 {
                return None;
            }
            style.item_style.flex_shrink = *v;
        }

        ("align-self", CssValue::Keyword(v)) => {
            style.item_style.align_self = Some(match v.as_str() {
                "stretch" => AlignItems::Stretch,
                "flex-start" | "start" => AlignItems::Start,
                "center" => AlignItems::Center,
                "flex-end" | "end" => AlignItems::End,
                "auto" => return Some(()),
                _ => return None,
            });
        }

        ("column-gap", _) => {
            style.column_gap = resolve_css_len_auto(name, value, text_style)?;
        }

        ("row-gap", _) => {
            style.row_gap = resolve_css_len_auto(name, value, text_style)?;
        }

        /* ======================
         * Grid
         * ====================== */
        ("grid-template-columns", _) => {
            style.grid_template_columns = parse_grid_tracks(name, value, text_style)?;
        }

        ("grid-template-rows", _) => {
            style.grid_template_rows = parse_grid_tracks(name, value, text_style)?;
        }

        ("grid-template-areas", _) => {
            style.grid_template_areas = parse_grid_template_areas(value)?;
        }

        ("grid-area", CssValue::Keyword(area)) => {
            style.grid_area = Some(area.to_string());
        }

        _ => {
            // log::error!("{name}, {value:?}");
            return None;
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
    color_scheme: ColorScheme,
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
        if let CssValue::Function(fn_name, args) = v
            && (fn_name == "linear-gradient" || fn_name == "radial-gradient")
        {
            maybe_gradient = Some(parse_gradient(fn_name, args, text_style, color_scheme)?);
            continue;
        }

        // color
        if let Some(c) = resolve_css_color(name, v, color_scheme) {
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

fn parse_gradient(
    fn_name: &str,
    args: &[CssValue],
    text_style: &TextStyle,
    color_scheme: ColorScheme,
) -> Option<Gradient> {
    match fn_name {
        "linear-gradient" => parse_linear_gradient(args, text_style, color_scheme),
        "radial-gradient" => parse_radial_gradient(args, text_style, color_scheme),
        _ => None,
    }
}

fn parse_linear_gradient(
    args: &[CssValue],
    _text_style: &TextStyle,
    color_scheme: ColorScheme,
) -> Option<Gradient> {
    if args.is_empty() {
        return None;
    }

    let (skip, angle) = parse_linear_direction(args);
    let angle = angle.unwrap_or(180.0);
    let stops = parse_color_stops(&args[skip..], color_scheme)?;

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
    if let CssValue::Keyword(k) = &args[0]
        && k.as_str() == "to"
        && args.len() > 1
    {
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

    (0, None)
}

fn parse_radial_gradient(
    args: &[CssValue],
    _text_style: &TextStyle,
    color_scheme: ColorScheme,
) -> Option<Gradient> {
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
        if idx < args.len()
            && let CssValue::Keyword(k) = &args[idx]
        {
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
            if idx < args.len()
                && let CssValue::Keyword(k2) = &args[idx]
            {
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

    let stops = parse_color_stops(&args[idx..], color_scheme)?;
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

fn parse_color_stops(args: &[CssValue], color_scheme: ColorScheme) -> Option<Vec<ColorStop>> {
    let mut stops = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let color = resolve_css_color("gradient", &args[i], color_scheme)?;
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
                CssValue::Number(0.0) => {
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

/// Extract font family names from a `font-family` CSS value.
///
/// Accepts a single keyword/string or a comma-separated list.
fn extract_font_families(value: &CssValue) -> Vec<String> {
    let items: Vec<&CssValue> = match value {
        CssValue::List(list) => list.iter().collect(),
        other => vec![other],
    };

    let mut families = Vec::new();
    for item in items {
        let name = match item {
            CssValue::Keyword(k) => k.to_string(),
            CssValue::String(s) => s.clone(),
            _ => continue,
        };
        if !name.is_empty() {
            families.push(name);
        }
    }
    families
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
                None
            }
            Unit::Fr => {
                log::error!(target: "Layouter", "Unexpected fr unit for `{}` (expected length)", name);
                None
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
        CssValue::Function(fn_name, args)
            if (fn_name == "min" || fn_name == "max") && args.len() >= 2 =>
        {
            let mut resolved: Vec<Length> = Vec::with_capacity(args.len());
            for arg in args {
                resolved.push(resolve_css_len(name, arg, text_style)?);
            }
            let mut result = resolved.remove(resolved.len() - 1);
            for arg in resolved.into_iter().rev() {
                result = if fn_name == "min" {
                    Length::Min(Box::new(arg), Box::new(result))
                } else {
                    Length::Max(Box::new(arg), Box::new(result))
                };
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

fn parse_grid_tracks(
    name: &str,
    value: &CssValue,
    text_style: &TextStyle,
) -> Option<Vec<GridTrack>> {
    if matches!(value, CssValue::Keyword(keyword) if keyword == "none") {
        return Some(Vec::new());
    }
    let values: Vec<&CssValue> = match value {
        CssValue::List(values) => values.iter().collect(),
        value => vec![value],
    };
    values
        .into_iter()
        .map(|value| parse_grid_track(name, value, text_style))
        .collect()
}

fn parse_grid_track(name: &str, value: &CssValue, text_style: &TextStyle) -> Option<GridTrack> {
    match value {
        CssValue::Keyword(keyword) if keyword == "auto" => {
            Some(GridTrack::Breadth(LengthOrAuto::Auto))
        }
        CssValue::Length(factor, Unit::Fr) if *factor >= 0.0 => Some(GridTrack::Flex(*factor)),
        CssValue::Function(function, args) if function == "minmax" => {
            let [minimum, maximum] = args.as_slice() else {
                return None;
            };
            Some(GridTrack::MinMax(
                Box::new(parse_grid_track(name, minimum, text_style)?),
                Box::new(parse_grid_track(name, maximum, text_style)?),
            ))
        }
        CssValue::Function(function, args) if function == "repeat" => {
            let (repeat, pattern) = args.split_first()?;
            let repeat = match repeat {
                CssValue::Number(count) if *count >= 1.0 && count.fract().abs() < f32::EPSILON => {
                    GridRepeat::Count(*count as usize)
                }
                CssValue::Keyword(keyword) if keyword == "auto-fit" => GridRepeat::AutoFit,
                CssValue::Keyword(keyword) if keyword == "auto-fill" => GridRepeat::AutoFill,
                _ => return None,
            };
            let pattern = pattern
                .iter()
                .map(|value| parse_grid_track(name, value, text_style))
                .collect::<Option<Vec<_>>>()?;
            (!pattern.is_empty()).then_some(GridTrack::Repeat(repeat, pattern))
        }
        _ => resolve_css_len(name, value, text_style)
            .map(LengthOrAuto::Length)
            .map(GridTrack::Breadth),
    }
}

fn parse_grid_template_areas(value: &CssValue) -> Option<Vec<Vec<String>>> {
    if matches!(value, CssValue::Keyword(keyword) if keyword == "none") {
        return Some(Vec::new());
    }
    let rows: Vec<&str> = match value {
        CssValue::String(row) => vec![row],
        CssValue::List(values) => values
            .iter()
            .map(|value| match value {
                CssValue::String(row) => Some(row.as_str()),
                _ => None,
            })
            .collect::<Option<_>>()?,
        _ => return None,
    };
    let areas: Vec<Vec<String>> = rows
        .into_iter()
        .map(|row| {
            row.split_whitespace()
                .map(|name| {
                    if name.chars().all(|character| character == '.') {
                        ".".to_string()
                    } else {
                        name.to_string()
                    }
                })
                .collect()
        })
        .collect();
    let width = areas.first()?.len();
    if width == 0 || areas.iter().any(|row| row.len() != width) {
        return None;
    }
    for name in areas.iter().flatten().filter(|name| name.as_str() != ".") {
        let cells: Vec<_> = areas
            .iter()
            .enumerate()
            .flat_map(|(row, areas)| {
                areas
                    .iter()
                    .enumerate()
                    .filter(|(_, area)| *area == name)
                    .map(move |(column, _)| (row, column))
            })
            .collect();
        let min_row = cells.iter().map(|(row, _)| *row).min()?;
        let max_row = cells.iter().map(|(row, _)| *row).max()?;
        let min_column = cells.iter().map(|(_, column)| *column).min()?;
        let max_column = cells.iter().map(|(_, column)| *column).max()?;
        if (min_row..=max_row)
            .any(|row| (min_column..=max_column).any(|column| areas[row][column] != *name))
        {
            return None;
        }
    }
    Some(areas)
}

/// Resolve a computed CssValue into a final RGBA Color.
///
/// Assumptions:
/// - This function is called *after* cascade and inheritance resolution.
/// - Keywords like `currentColor`, `inherit`, `initial`, `unset`
///   must NOT reach this stage.
/// - The returned Color is always absolute RGBA.
///
/// `color_scheme` is the element's used color scheme, used to resolve
/// `light-dark()`.
fn resolve_css_color(name: &str, css_color: &CssValue, color_scheme: ColorScheme) -> Option<Color> {
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
            "darkred" => Some(Color(139, 0, 0, 255)),
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
            "wheat" => Some(Color(245, 222, 179, 255)),

            // ===== Yellow =====
            "beige" => Some(Color(245, 245, 220, 255)),
            "gold" => Some(Color(255, 215, 0, 255)),
            "goldenrod" => Some(Color(218, 165, 32, 255)),
            "yellow" => Some(Color(255, 255, 0, 255)),
            "lightyellow" => Some(Color(255, 255, 224, 255)),
            "lemonchiffon" => Some(Color(255, 250, 205, 255)),
            "lightgoldenrodyellow" => Some(Color(250, 250, 210, 255)),
            "khaki" => Some(Color(240, 230, 140, 255)),
            "papayawhip" => Some(Color(255, 239, 213, 255)),
            "moccasin" => Some(Color(255, 228, 181, 255)),

            // ===== Green =====
            "green" => Some(Color(0, 128, 0, 255)),
            "darkgreen" => Some(Color(0, 100, 0, 255)),
            "forestgreen" => Some(Color(34, 139, 34, 255)),
            "lime" => Some(Color(0, 255, 0, 255)),
            "limegreen" => Some(Color(50, 205, 50, 255)),
            "lightgreen" => Some(Color(144, 238, 144, 255)),
            "olive" => Some(Color(128, 128, 0, 255)),
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
            "teal" => Some(Color(0, 128, 128, 255)),
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
            "thistle" => Some(Color(216, 191, 216, 255)),
            "rebeccapurple" => Some(Color(102, 51, 153, 255)),

            // ===== Brown =====
            "bisque" => Some(Color(255, 228, 196, 255)),
            "brown" => Some(Color(165, 42, 42, 255)),
            "saddlebrown" => Some(Color(139, 69, 19, 255)),
            "sienna" => Some(Color(160, 82, 45, 255)),
            "tan" => Some(Color(210, 180, 140, 255)),
            "chocolate" => Some(Color(210, 105, 30, 255)),
            "peru" => Some(Color(205, 133, 63, 255)),
            "burlywood" => Some(Color(222, 184, 135, 255)),

            // ===== White variations =====
            "snow" => Some(Color(255, 250, 250, 255)),
            "honeydew" => Some(Color(240, 255, 240, 255)),
            "mintcream" => Some(Color(245, 255, 250, 255)),
            "ivory" => Some(Color(255, 255, 240, 255)),
            "azure" => Some(Color(240, 255, 255, 255)),
            "aliceblue" => Some(Color(240, 248, 255, 255)),
            "ghostwhite" => Some(Color(248, 248, 255, 255)),
            "linen" => Some(Color(250, 240, 230, 255)),
            "oldlace" => Some(Color(253, 245, 230, 255)),

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
            let mut values = Vec::new();
            let mut has_pct = false;
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
                            values.push(*n);
                        }
                    }
                    CssValue::Length(p, Unit::Percent) => {
                        has_pct = true;
                        if after_slash {
                            alpha = Some(p / 100.0);
                        } else {
                            values.push(*p);
                        }
                    }
                    _ => return None,
                }
            }

            // rgb(r, g, b) -> 3 values
            // rgba(r, g, b, a) -> 4 values
            // rgb(r g b / a) -> 3 values + after_slash
            let (a, values) = if values.len() == 4 && alpha.is_none() {
                (values[3], vec![values[0], values[1], values[2]])
            } else if values.len() == 3 {
                (alpha.unwrap_or(1.0), values)
            } else {
                return None;
            };

            // CSS stores rgb values as 0-255 integers or 0.0-1.0 floats
            // or 0%-100% (already handled above).
            let map_channel = |v: f32| -> f32 {
                if has_pct {
                    v / 100.0 * 255.0
                } else if v > 1.0 {
                    v.clamp(0.0, 255.0)
                } else {
                    v * 255.0
                }
            };

            Some(Color(
                map_channel(values[0]).round() as u8,
                map_channel(values[1]).round() as u8,
                map_channel(values[2]).round() as u8,
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

        // light-dark(<light-color>, <dark-color>)
        CssValue::Function(func, args) if func == "light-dark" && args.len() == 2 => {
            let chosen = match color_scheme {
                ColorScheme::Light => &args[0],
                ColorScheme::Dark => &args[1],
            };
            resolve_css_color(name, chosen, color_scheme)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::bridge::text::FallbackTextMeasurer;
    use crate::engine::css::parser::Parser as CssParser;
    use crate::engine::html::parser::Parser as HtmlParser;
    use crate::engine::layouter::css_resolver::CssResolver;
    use std::sync::Arc;

    fn apply_layout_property(name: &str, value: CssValue) -> Style {
        let mut style = Style::default();
        let mut container_style = ContainerStyle::default();
        let mut text_style = TextStyle::default();
        let mut overflow = Overflow::default();
        let parsed = apply_declaration(
            name,
            &value,
            &mut style,
            &mut container_style,
            &mut text_style,
            &mut overflow,
            ColorScheme::Light,
        );
        assert!(parsed.is_some());
        style
    }

    #[test]
    fn position_keywords_map_to_layout_position() {
        for (keyword, expected) in [
            ("static", Position::Static),
            ("relative", Position::Relative),
            ("absolute", Position::Absolute),
            ("fixed", Position::Fixed),
        ] {
            let style = apply_layout_property("position", CssValue::Keyword(keyword.into()));
            assert_eq!(style.position.kind, expected);
        }
    }

    #[test]
    fn inset_shorthand_expands_lengths_and_auto() {
        let style = apply_layout_property(
            "inset",
            CssValue::List(vec![
                CssValue::Length(10.0, Unit::Px),
                CssValue::Length(20.0, Unit::Percent),
                CssValue::Keyword("auto".into()),
                CssValue::Length(4.0, Unit::Px),
            ]),
        );

        assert_eq!(style.position.top, Length::Px(10.0).into());
        assert_eq!(style.position.right, Length::Percent(20.0).into());
        assert_eq!(style.position.bottom, LengthOrAuto::Auto);
        assert_eq!(style.position.left, Length::Px(4.0).into());
    }

    #[test]
    fn positioned_insets_map_to_individual_sides() {
        for (name, value) in [("top", 1.0), ("right", 2.0), ("bottom", 3.0), ("left", 4.0)] {
            let style = apply_layout_property(name, CssValue::Length(value, Unit::Px));
            let actual = match name {
                "top" => style.position.top,
                "right" => style.position.right,
                "bottom" => style.position.bottom,
                "left" => style.position.left,
                _ => unreachable!(),
            };
            assert_eq!(actual, Length::Px(value).into());
        }
    }

    #[test]
    fn flex_shorthand_expands_common_forms() {
        let one = apply_layout_property("flex", CssValue::Number(1.0));
        assert_eq!(one.item_style.flex_grow, 1.0);
        assert_eq!(one.item_style.flex_shrink, 1.0);
        assert_eq!(
            one.item_style.flex_basis,
            LengthOrAuto::Length(Length::Percent(0.0))
        );

        let explicit = apply_layout_property(
            "flex",
            CssValue::List(vec![
                CssValue::Number(2.0),
                CssValue::Number(0.0),
                CssValue::Length(10.0, Unit::Px),
            ]),
        );
        assert_eq!(explicit.item_style.flex_grow, 2.0);
        assert_eq!(explicit.item_style.flex_shrink, 0.0);
        assert_eq!(
            explicit.item_style.flex_basis,
            LengthOrAuto::Length(Length::Px(10.0))
        );
    }

    #[test]
    fn flex_shorthand_expands_keywords() {
        for (keyword, expected_grow, expected_shrink, expected_basis) in [
            ("none", 0.0, 0.0, LengthOrAuto::Auto),
            ("auto", 1.0, 1.0, LengthOrAuto::Auto),
            ("initial", 0.0, 1.0, LengthOrAuto::Auto),
        ] {
            let style = apply_layout_property("flex", CssValue::Keyword(keyword.into()));
            assert_eq!(style.item_style.flex_grow, expected_grow);
            assert_eq!(style.item_style.flex_shrink, expected_shrink);
            assert_eq!(style.item_style.flex_basis, expected_basis);
        }
    }

    #[test]
    fn flex_wrap_and_align_content_map_to_layout() {
        let wrap = apply_layout_property("flex-wrap", CssValue::Keyword("wrap-reverse".into()));
        assert_eq!(wrap.flex_wrap, FlexWrap::WrapReverse);

        for (keyword, expected) in [
            ("normal", AlignContent::Stretch),
            ("flex-start", AlignContent::Start),
            ("center", AlignContent::Center),
            ("flex-end", AlignContent::End),
            ("space-between", AlignContent::SpaceBetween),
            ("space-around", AlignContent::SpaceAround),
            ("space-evenly", AlignContent::SpaceEvenly),
        ] {
            let style = apply_layout_property("align-content", CssValue::Keyword(keyword.into()));
            assert_eq!(style.align_content, expected);
        }
    }

    #[test]
    fn flex_flow_expands_direction_and_wrap_in_either_order() {
        for values in [
            vec![
                CssValue::Keyword("column".into()),
                CssValue::Keyword("wrap".into()),
            ],
            vec![
                CssValue::Keyword("wrap".into()),
                CssValue::Keyword("column".into()),
            ],
        ] {
            let style = apply_layout_property("flex-flow", CssValue::List(values));
            assert_eq!(style.flex_direction, FlexDirection::Column);
            assert_eq!(style.flex_wrap, FlexWrap::Wrap);
        }

        let wrap_only =
            apply_layout_property("flex-flow", CssValue::Keyword("wrap-reverse".into()));
        assert_eq!(wrap_only.flex_direction, FlexDirection::Row);
        assert_eq!(wrap_only.flex_wrap, FlexWrap::WrapReverse);
    }

    #[test]
    fn grid_tracks_map_lengths_fractions_and_auto() {
        let columns = apply_layout_property(
            "grid-template-columns",
            CssValue::List(vec![
                CssValue::Length(100.0, Unit::Px),
                CssValue::Length(2.0, Unit::Fr),
                CssValue::Keyword("auto".into()),
            ]),
        );
        assert_eq!(
            columns.grid_template_columns,
            vec![
                GridTrack::Breadth(LengthOrAuto::Length(Length::Px(100.0))),
                GridTrack::Flex(2.0),
                GridTrack::default(),
            ]
        );

        let rows = apply_layout_property(
            "grid-template-rows",
            CssValue::List(vec![
                CssValue::Keyword("auto".into()),
                CssValue::Length(25.0, Unit::Percent),
            ]),
        );
        assert_eq!(
            rows.grid_template_rows,
            vec![
                GridTrack::default(),
                GridTrack::Breadth(LengthOrAuto::Length(Length::Percent(25.0))),
            ]
        );

        let none = apply_layout_property("grid-template-columns", CssValue::Keyword("none".into()));
        assert!(none.grid_template_columns.is_empty());
    }

    #[test]
    fn grid_tracks_map_repeat_and_minmax() {
        let style = apply_layout_property(
            "grid-template-columns",
            CssValue::Function(
                "repeat".into(),
                vec![
                    CssValue::Keyword("auto-fit".into()),
                    CssValue::Function(
                        "minmax".into(),
                        vec![
                            CssValue::Length(100.0, Unit::Px),
                            CssValue::Length(1.0, Unit::Fr),
                        ],
                    ),
                ],
            ),
        );
        assert_eq!(
            style.grid_template_columns,
            vec![GridTrack::Repeat(
                GridRepeat::AutoFit,
                vec![GridTrack::MinMax(
                    Box::new(GridTrack::Breadth(LengthOrAuto::Length(Length::Px(100.0,)))),
                    Box::new(GridTrack::Flex(1.0)),
                )],
            )]
        );
    }

    #[test]
    fn grid_named_areas_map_rows_and_item_name() {
        let template = apply_layout_property(
            "grid-template-areas",
            CssValue::List(vec![
                CssValue::String("header header".into()),
                CssValue::String("sidebar main".into()),
                CssValue::String("footer footer".into()),
            ]),
        );
        assert_eq!(
            template.grid_template_areas,
            vec![
                vec!["header".to_string(), "header".to_string()],
                vec!["sidebar".to_string(), "main".to_string()],
                vec!["footer".to_string(), "footer".to_string()],
            ]
        );

        let item = apply_layout_property("grid-area", CssValue::Keyword("header".into()));
        assert_eq!(item.grid_area.as_deref(), Some("header"));
    }

    #[test]
    fn absolute_and_fixed_inline_boxes_are_blockified() {
        for position in [Position::Absolute, Position::Fixed] {
            let mut style = Style {
                display: Display {
                    outer: OuterDisplay::Inline,
                    inner: InnerDisplay::Flow,
                },
                position: ui_layout::PositionStyle {
                    kind: position,
                    ..Default::default()
                },
                ..Default::default()
            };
            blockify_out_of_flow_positioned(&mut style);
            assert_eq!(style.display.outer, OuterDisplay::Block);
        }
    }

    fn apply_overflow(value: CssValue) -> Overflow {
        let mut style = Style::default();
        let mut container_style = ContainerStyle::default();
        let mut text_style = TextStyle::default();
        let mut overflow = Overflow::default();
        let parsed = apply_declaration(
            "overflow",
            &value,
            &mut style,
            &mut container_style,
            &mut text_style,
            &mut overflow,
            ColorScheme::Light,
        );
        assert!(parsed.is_some());
        overflow
    }

    #[test]
    fn overflow_single_keyword_sets_both_axes() {
        assert_eq!(
            apply_overflow(CssValue::Keyword("hidden".into())),
            Overflow { x: true, y: true }
        );
        assert_eq!(
            apply_overflow(CssValue::Keyword("auto".into())),
            Overflow { x: true, y: true }
        );
        assert_eq!(
            apply_overflow(CssValue::Keyword("visible".into())),
            Overflow { x: false, y: false }
        );
        assert_eq!(
            apply_overflow(CssValue::Keyword("clip".into())),
            Overflow { x: false, y: false }
        );
    }

    #[test]
    fn overflow_two_keywords_set_axes_independently() {
        assert_eq!(
            apply_overflow(CssValue::List(vec![
                CssValue::Keyword("hidden".into()),
                CssValue::Keyword("visible".into()),
            ])),
            Overflow { x: true, y: false }
        );
        assert_eq!(
            apply_overflow(CssValue::List(vec![
                CssValue::Keyword("visible".into()),
                CssValue::Keyword("auto".into()),
            ])),
            Overflow { x: false, y: true }
        );
    }

    #[test]
    fn overflow_axis_properties_set_single_axis() {
        let mut style = Style::default();
        let mut container_style = ContainerStyle::default();
        let mut text_style = TextStyle::default();
        let mut overflow = Overflow::default();

        assert!(
            apply_declaration(
                "overflow-x",
                &CssValue::Keyword("scroll".into()),
                &mut style,
                &mut container_style,
                &mut text_style,
                &mut overflow,
                ColorScheme::Light,
            )
            .is_some()
        );
        assert_eq!(overflow, Overflow { x: true, y: false });

        assert!(
            apply_declaration(
                "overflow-y",
                &CssValue::Keyword("auto".into()),
                &mut style,
                &mut container_style,
                &mut text_style,
                &mut overflow,
                ColorScheme::Light,
            )
            .is_some()
        );
        assert_eq!(overflow, Overflow { x: true, y: true });
    }

    #[test]
    fn unsupported_overflow_value_is_rejected() {
        let mut style = Style::default();
        let mut container_style = ContainerStyle::default();
        let mut text_style = TextStyle::default();
        let mut overflow = Overflow::default();
        assert!(
            apply_declaration(
                "overflow",
                &CssValue::Number(1.0),
                &mut style,
                &mut container_style,
                &mut text_style,
                &mut overflow,
                ColorScheme::Light,
            )
            .is_none()
        );
        assert_eq!(overflow, Overflow::default());
    }

    fn layout_for(html: &str, css: &str) -> InfoNode {
        layout_and_info_for(html, css).1
    }

    fn layout_and_info_for(html: &str, css: &str) -> (LayoutNode, InfoNode) {
        let dom = HtmlParser::new(html).parse();
        let mut resolved = ResolvedStyles::default();
        if !css.is_empty() {
            let sheet = CssParser::new(css).parse().unwrap();
            resolved.extend(CssResolver::resolve(&sheet));
        }
        build_layout_and_info(
            &dom.root,
            &resolved,
            Arc::new(FallbackTextMeasurer),
            InheritedCss {
                text_style: TextStyle::default(),
                color_scheme: ColorScheme::Light,
            },
            Vec::new(),
            ColorScheme::Light,
        )
    }

    #[test]
    fn named_grid_area_css_controls_final_layout() {
        let html = r#"<html><body><div class="grid"><div class="header"></div><div class="sidebar"></div><div class="main"></div><div class="footer"></div></div></body></html>"#;
        let css = r#"
            .grid {
                display: grid;
                width: 300px;
                grid-template-areas: "header header" "sidebar main" "footer footer";
                grid-template-columns: 1fr 2fr;
                gap: 10px;
            }
            .grid > div { height: 20px; }
            .header { grid-area: header; }
            .sidebar { grid-area: sidebar; }
            .main { grid-area: main; }
            .footer { grid-area: footer; }
        "#;
        let (mut layout, _) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

        fn find_grid(node: &LayoutNode) -> Option<&LayoutNode> {
            if node.style.display.inner == InnerDisplay::Grid {
                return Some(node);
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(find_grid)
        }
        let grid = find_grid(&layout).expect("grid container");
        let items: Vec<_> = grid.children.iter().filter_map(LayoutChild::node).collect();
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].layout_box.width_box(), 300.0);
        assert_eq!(items[1].layout_box.iter().next().unwrap().border_box.x, 0.0);
        assert!((items[2].layout_box.iter().next().unwrap().border_box.x - 106.66667).abs() < 0.01);
        assert_eq!(items[3].layout_box.width_box(), 300.0);
        assert!(items[3].layout_box.iter().next().unwrap().border_box.y >= 60.0);
    }

    #[test]
    fn flex_wrap_css_creates_multiple_lines() {
        let html = r#"<html><body><div class="flex"><div></div><div></div></div></body></html>"#;
        let css = r#"
            .flex {
                display: flex;
                width: 100px;
                flex-flow: row wrap;
                align-content: flex-start;
            }
            .flex > div {
                width: 60px;
                height: 20px;
            }
        "#;
        let (mut layout, _) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

        fn find_flex(node: &LayoutNode) -> Option<&LayoutNode> {
            if node.style.display.inner == InnerDisplay::Flex {
                return Some(node);
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(find_flex)
        }
        let flex = find_flex(&layout).expect("flex container");
        let items: Vec<_> = flex.children.iter().filter_map(LayoutChild::node).collect();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].layout_box.iter().next().unwrap().border_box.y, 0.0);
        assert_eq!(
            items[1].layout_box.iter().next().unwrap().border_box.y,
            20.0
        );
    }

    #[test]
    fn structural_selectors_apply_styles_from_html_context() {
        let html = r#"
            <html><body>
                <ul><li>first</li><li>second</li><li>third</li></ul>
                <h2>heading</h2><p>adjacent</p>
            </body></html>
        "#;
        let css = r#"
            li:first-child { color: #ff0000; }
            li:nth-child(2) { color: #008000; }
            li:last-child:not(.skip) { color: #0000ff; }
            h2 + p { color: #663399; }
        "#;
        let info = layout_for(html, css);

        assert_eq!(text_style_for(&info, "first").color, Color(255, 0, 0, 255));
        assert_eq!(text_style_for(&info, "second").color, Color(0, 128, 0, 255));
        assert_eq!(text_style_for(&info, "third").color, Color(0, 0, 255, 255));
        assert_eq!(
            text_style_for(&info, "adjacent").color,
            Color(102, 51, 153, 255)
        );
    }

    /// Depth-first search for the first [`NodeKind::Text`] whose content is
    /// `content`, returning a clone of its style.
    fn text_style_for(info: &InfoNode, content: &str) -> TextStyle {
        fn walk(node: &InfoNode, content: &str) -> Option<TextStyle> {
            if let NodeKind::Text {
                style,
                text: actual,
                ..
            } = &node.kind
                && actual == content
            {
                return Some(style.clone());
            }
            node.children.iter().find_map(|child| walk(child, content))
        }
        walk(info, content).expect("text node with expected content must exist")
    }

    #[test]
    fn inline_style_overrides_stylesheet_rule() {
        // A stylesheet sets color to blue; the inline attribute must win.
        let html = r#"<html><body><p id="x" style="color: red;">hello</p></body></html>"#;
        let info = layout_for(html, "p { color: blue; }");

        assert_eq!(text_style_for(&info, "hello").color, Color(255, 0, 0, 255));
    }

    #[test]
    fn inline_style_non_important_loses_to_important_stylesheet() {
        let html = r#"<html><body><p id="x" style="color: red;">hello</p></body></html>"#;
        let info = layout_for(html, "p { color: blue !important; }");

        assert_eq!(text_style_for(&info, "hello").color, Color(0, 0, 255, 255));
    }

    #[test]
    fn inline_style_important_beats_stylesheet_important() {
        let html =
            r#"<html><body><p id="x" style="color: red !important;">hello</p></body></html>"#;
        let info = layout_for(html, "p { color: blue !important; }");

        assert_eq!(text_style_for(&info, "hello").color, Color(255, 0, 0, 255));
    }

    #[test]
    fn inline_style_sets_container_background() {
        let html =
            r#"<html><body><div style="background-color: rgb(0, 128, 0);">x</div></body></html>"#;
        let info = layout_for(html, "");

        fn find_div(node: &InfoNode) -> Option<&ContainerStyle> {
            if let NodeKind::Container { style, .. } = &node.kind
                && style.background != Background::default()
            {
                return Some(style);
            }
            node.children.iter().find_map(find_div)
        }
        let style = find_div(&info).expect("div container with background exists");
        assert_eq!(style.background, Background::Color(Color(0, 128, 0, 255)));
    }
}
