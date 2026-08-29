//! Layout builder, which transforms a DOM tree into a UI layout.

mod apply;
mod background;
mod color;
mod css_resolve;
mod layout_fix;
#[cfg(test)]
mod tests;

pub use apply::{apply_declaration, blockify_out_of_flow_positioned};
pub use layout_fix::{
    constrain_auto_grid_track_items, correct_atomic_inline_spacing,
    correct_atomic_inline_spacing_with_info, is_block_layout_child, is_collapsible_whitespace_info,
    maximum_fixed_descendant_width, refresh_missing_text_layout_results,
};

#[allow(unused_imports)]
pub use background::{
    apply_background_shorthand_geometry, parse_background_position, parse_background_repeat,
    parse_background_shorthand, parse_background_size, parse_gradient,
};
pub use color::resolve_css_color;
pub use css_resolve::{
    extract_font_families, one_or_two_values, parse_grid_line, parse_grid_line_end,
    parse_grid_placement, parse_grid_template_areas, parse_grid_tracks, resolve_css_len,
    resolve_css_len_auto, resolve_font_size_px,
};

use crate::{perf_scope, profile_log};

use crate::engine::bridge::text::{self, GlyphCluster};
use crate::engine::css::{
    matcher::{ElementChain, ElementInfo},
    values::{CssValue, Unit},
};
use crate::engine::html::{HtmlNodeType, ScriptingMode};
use crate::engine::layouter::css_resolver::{
    DeclarationResolver, Properties, resolve_inline_value,
};
use crate::engine::layouter::dom_snapshot::{DomSnapshot, NodeId};
use crate::engine::layouter::types::WhiteSpace;
use crate::engine::renderer_model::Image;
use crate::engine::tree::NodeRef;
use crate::engine::ui::custom_node_bridge::CustomNodeBridge;
use crate::engine::ui::registry::{ComponentRegistry, CustomNodeContext, DomWriteBack};

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[allow(unused_imports)]
use ui_layout::{
    AlignContent, AlignItems, AutoSizeBehavior, BoxSizing, Display, FlexDirection, FlexWrap,
    GridPlacement, GridPlacementEnd, GridRepeat, GridTrack, InnerDisplay, ItemFragment,
    JustifyContent, JustifyItems, LayoutChild, LayoutNode, Length, LengthOrAuto, OuterDisplay,
    Position, Style,
};

use super::css_resolver::{
    MediaEnvironment, ResolvedStyles, RuleSet, resolve_inline_style, set_inline_custom_property,
};
use super::text_layouter::TextFlowLayouter;
#[allow(unused_imports)]
use super::types::{
    Background, BackgroundDimension, BackgroundOffset, BackgroundPosition, BackgroundPositionAxis,
    BackgroundRepeat, BackgroundSize, BorderRadius, BorderStyle, Color, ColorScheme, ColorStop,
    ContainerRole, ContainerStyle, CornerRadius, CssFloat, FontStyle, FontWeight, Gradient,
    GradientKind, InfoNode, LineHeight, NodeKind, Overflow, RadialShape, RadialSizeKind, TextAlign,
    TextDecoration, TextFlowStyle, TextStyle, TextTransform,
};

pub(crate) const DEFAULT_LINE_FACTOR: f32 = 1.2;

const GRID_LINE_TO_END: GridPlacementEnd = GridPlacementEnd::Line(usize::MAX);

pub(crate) fn element_info(html_node: &HtmlNodeType) -> Option<ElementInfo> {
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
        previous_siblings: ElementChain::default(),
    })
}

pub(crate) fn element_sibling_infos(
    snapshot: &DomSnapshot,
    children: &[NodeId],
) -> Vec<Option<ElementInfo>> {
    let mut type_counts = HashMap::<String, usize>::new();
    for &child in children {
        if let Some(tag_name) = snapshot.node(child).kind.tag_name() {
            *type_counts.entry(tag_name.to_string()).or_default() += 1;
        }
    }
    let element_count = type_counts.values().sum();

    let mut seen_types = HashMap::<String, usize>::new();
    let mut previous_siblings = ElementChain::default();
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
            info.previous_siblings = previous_siblings.clone();

            let mut sibling = info.clone();
            sibling.previous_siblings = ElementChain::default();
            previous_siblings = previous_siblings.prepend(Some(sibling));
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
#[derive(Clone, Default)]
pub struct InheritedCss {
    /// Inherited CSS custom properties, shared copy-on-write: descendants that
    /// do not introduce new `--var` values reuse the parent's map via `Arc`
    /// instead of deep-cloning it per element.
    pub custom_props: Arc<Properties>,
    pub text_style: TextStyle,
    pub text_flow_style: TextFlowStyle,
    pub color_scheme: ColorScheme,
}

/// Convert a resolved `Length` to an absolute pixel value for `LineHeight::Px`.
pub(super) fn length_to_px(len: &Length, font_size: f32) -> f32 {
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

struct StackFrame {
    dom: NodeId,
    chain: ElementChain,
    child: Arc<InheritedCss>,
    kind: Option<NodeKind>,
    style: Option<Style>,
    parent_style: Style,
    parent_container_style: ContainerStyle,
    child_slots: Vec<ChildSlot>,
    element_children: Vec<NodeId>,
}

enum ChildSlot {
    Inline(LayoutChild, Box<InfoNode>),
    Element(usize),
}

pub fn build_layout_and_info(
    dom: &NodeRef<HtmlNodeType>,
    resolved_styles: &ResolvedStyles,
    measurer: Arc<dyn text::TextMeasurer>,
    parent: InheritedCss,
    chain: ElementChain,
    system_color_scheme: ColorScheme,
    scripting_mode: ScriptingMode,
) -> (LayoutNode, InfoNode) {
    build_layout_and_info_with_images(
        dom,
        resolved_styles,
        measurer,
        parent,
        chain,
        system_color_scheme,
        scripting_mode,
        &HashMap::new(),
    )
}

/// Builds layout and render trees with decoded images keyed by their `src` value.
#[allow(clippy::too_many_arguments)]
pub fn build_layout_and_info_with_images(
    dom: &NodeRef<HtmlNodeType>,
    resolved_styles: &ResolvedStyles,
    measurer: Arc<dyn text::TextMeasurer>,
    parent: InheritedCss,
    chain: ElementChain,
    system_color_scheme: ColorScheme,
    scripting_mode: ScriptingMode,
    images: &HashMap<String, Image>,
) -> (LayoutNode, InfoNode) {
    let (snapshot, _dom_refs) = DomSnapshot::from_tree(dom);
    let media_environment = MediaEnvironment::new((0.0, 0.0), system_color_scheme);
    let rule_set = RuleSet::from_declarations(resolved_styles, &media_environment);
    build_layout_and_info_from_snapshot(
        &snapshot,
        snapshot.roots()[0],
        &rule_set,
        measurer,
        parent,
        chain,
        system_color_scheme,
        scripting_mode,
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
    rule_set: &RuleSet,
    measurer: Arc<dyn text::TextMeasurer>,
    parent: InheritedCss,
    mut chain: ElementChain,
    system_color_scheme: ColorScheme,
    scripting_mode: ScriptingMode,
    images: &HashMap<String, Image>,
    audio: &HashMap<String, Arc<[u8]>>,
    write_back_sender: Option<DomWriteBack>,
) -> (LayoutNode, InfoNode) {
    perf_scope!(total);

    #[cfg(any(feature = "profile", debug_assertions))]
    let mut css_match_time = std::time::Duration::ZERO;
    #[cfg(any(feature = "profile", debug_assertions))]
    let mut apply_decl_time = std::time::Duration::ZERO;
    #[cfg(any(feature = "profile", debug_assertions))]
    let mut custom_node_time = std::time::Duration::ZERO;
    #[cfg(any(feature = "profile", debug_assertions))]
    let mut text_layout_time = std::time::Duration::ZERO;
    #[cfg(any(feature = "profile", debug_assertions))]
    let mut exit_phase_time = std::time::Duration::ZERO;
    #[cfg(any(feature = "profile", debug_assertions))]
    let mut sibling_info_time = std::time::Duration::ZERO;
    #[cfg(any(feature = "profile", debug_assertions))]
    let mut whitespace_keep_time = std::time::Duration::ZERO;
    #[cfg(any(feature = "profile", debug_assertions))]
    let mut enter_prep_time = std::time::Duration::ZERO;
    #[cfg(any(feature = "profile", debug_assertions))]
    let mut child_slot_build_time = std::time::Duration::ZERO;
    #[cfg(any(feature = "profile", debug_assertions))]
    let mut leaf_assemble_time = std::time::Duration::ZERO;
    #[cfg(any(feature = "profile", debug_assertions))]
    let mut push_children_time = std::time::Duration::ZERO;
    #[cfg(any(feature = "profile", debug_assertions))]
    let mut exit_setup_time = std::time::Duration::ZERO;
    #[cfg(any(feature = "profile", debug_assertions))]
    let mut exit_build_time = std::time::Duration::ZERO;
    #[cfg(any(feature = "profile", debug_assertions))]
    let mut node_count = 0u64;

    #[cfg(any(feature = "profile", debug_assertions))]
    let mut cand_stats = CandidateMetrics::default();

    let registry = ComponentRegistry::new();
    /*
     * Build the initial element chain for the root node.
     */
    if let Some(info) = element_info(&snapshot.node(root).kind) {
        chain = chain.prepend(Some(info));
    }

    // ── Explicit post-order stack (index-based to avoid borrow conflicts) ──

    let mut stack: Vec<StackFrame> = Vec::new();
    stack.push(StackFrame {
        dom: root,
        chain,
        child: Arc::new(parent),
        kind: None,
        style: None,
        parent_style: Style::default(),
        parent_container_style: ContainerStyle::default(),
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
            let child_css = Arc::clone(&stack[top_idx].child);

            let html_node = &snapshot.node(stack[top_idx].dom).kind;
            let mut text_style = child_css.text_style.clone();
            let mut text_flow_style = child_css.text_flow_style;
            let mut container_style = ContainerStyle::default();
            let mut style = Style::default();
            let mut overflow = Overflow::default();
            let parent_style = stack[top_idx].parent_style.clone();
            let parent_container_style = stack[top_idx].parent_container_style.clone();
            let parent_text_style = text_style.clone();
            let parent_text_flow_style = text_flow_style;

            // Inherit container visibility
            container_style.visibility = parent_container_style.visibility;

            // Collect CSS candidates.
            perf_scope!(css_match);
            let (candidates, custom_property_candidates) =
                if let HtmlNodeType::Element { .. } = html_node {
                    Some(collect_candidates(
                        rule_set,
                        &chain_for_css,
                        #[cfg(any(feature = "profile", debug_assertions))]
                        &mut cand_stats,
                    ))
                } else {
                    None
                }
                .unzip();
            #[cfg(any(feature = "profile", debug_assertions))]
            {
                css_match_time += css_match.elapsed();
            }

            perf_scope!(enter_prep);
            // Inherit custom properties by sharing the parent's map unless this
            // element introduces new `--var` values (copy-on-write applies
            // cascade-discovered custom properties lazily).
            let mut custom_properties = Arc::clone(&child_css.custom_props);
            if let Some(own) = custom_property_candidates
                && !own.is_empty()
            {
                Arc::make_mut(&mut custom_properties).extend(own);
            }

            // Resolve the used color scheme for this element. `light-dark()`
            // and system colors resolve against it, and it is inherited by
            // descendants that do not set `color-scheme` themselves.
            let used_color_scheme = {
                let declaration = candidates
                    .as_ref()
                    .and_then(|c| c.get("color-scheme"))
                    .and_then(|d| {
                        DeclarationResolver::resolve_var(
                            &d.value,
                            &custom_properties,
                            &mut HashSet::new(),
                        )
                    });
                resolve_used_color_scheme(
                    declaration.as_ref(),
                    child_css.color_scheme,
                    system_color_scheme,
                )
            };
            #[cfg(any(feature = "profile", debug_assertions))]
            {
                enter_prep_time += enter_prep.elapsed();
            }

            // Apply CSS declarations.
            perf_scope!(apply_decl);
            if let Some(candidates) = &candidates {
                // The candidates map dedupes per property name (cascade winner),
                // but a shorthand and its longhand (e.g. `padding` and
                // `padding-top`) are distinct keys. Iterating a HashMap applies
                // them in random order, so re-sort by source order to make the
                // cascade deterministic.
                let mut candidates: Vec<_> = candidates.values().collect();
                candidates.sort_by_key(|declaration| declaration.order);

                for declaration in candidates {
                    if declaration.name.starts_with("--") {
                        continue;
                    }
                    let Some(value) = DeclarationResolver::resolve_var(
                        &declaration.value,
                        &custom_properties,
                        &mut HashSet::new(),
                    ) else {
                        continue;
                    };
                    apply_declaration(
                        &declaration.name,
                        &value,
                        &mut style,
                        &mut container_style,
                        &mut text_style,
                        &mut text_flow_style,
                        &parent_style,
                        &parent_container_style,
                        &parent_text_style,
                        &parent_text_flow_style,
                        &mut overflow,
                        used_color_scheme,
                    );
                }
            }

            // Apply the element's inline `style` attribute. Inline styles are
            // author-origin declarations with the highest specificity, so they
            // override stylesheet rules — unless the stylesheet rule was
            // `!important`, which still wins over a non-`!important` inline
            // declaration.
            if let Some(style_attr) = html_node.get_attr("style") {
                let inline_declarations = resolve_inline_style(style_attr);
                for (name, value, important) in &inline_declarations {
                    if name.starts_with("--") {
                        let stylesheet_important = candidates
                            .as_ref()
                            .and_then(|c| c.get(name))
                            .is_some_and(|declaration| declaration.important);
                        if *important || !stylesheet_important {
                            set_inline_custom_property(
                                Arc::make_mut(&mut custom_properties),
                                name.clone(),
                                value.clone(),
                                *important,
                            );
                        }
                    }
                }
                for (name, value, important) in inline_declarations {
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
                    let Some(value) = DeclarationResolver::resolve_var(
                        &value,
                        &custom_properties,
                        &mut HashSet::new(),
                    ) else {
                        continue;
                    };
                    apply_declaration(
                        &name,
                        &value,
                        &mut style,
                        &mut container_style,
                        &mut text_style,
                        &mut text_flow_style,
                        &parent_style,
                        &parent_container_style,
                        &parent_text_style,
                        &parent_text_flow_style,
                        &mut overflow,
                        used_color_scheme,
                    );
                }
            }

            // Apply attribute sizing
            for attr in ["width", "height"] {
                if let Some(value) = html_node.get_attr(attr)
                    && let Some(mut value) = resolve_inline_value(value)
                {
                    if let CssValue::Number(v) = value {
                        value = CssValue::Length(v, Unit::Px);
                    }
                    apply_declaration(
                        attr,
                        &value,
                        &mut style,
                        &mut container_style,
                        &mut text_style,
                        &mut text_flow_style,
                        &parent_style,
                        &parent_container_style,
                        &parent_text_style,
                        &parent_text_flow_style,
                        &mut overflow,
                        used_color_scheme,
                    );
                }
            }
            #[cfg(any(feature = "profile", debug_assertions))]
            {
                apply_decl_time += apply_decl.elapsed();
            }

            if let Background::Image { source, image, .. } = &mut container_style.background {
                *image = images.get(source).cloned();
            }

            if container_style.css_float != CssFloat::None && !style.position.kind.is_out_of_flow()
            {
                style.display = Display {
                    outer: OuterDisplay::Inline,
                    inner: InnerDisplay::FlowRoot,
                };
                style.size.auto_behavior = AutoSizeBehavior::ShrinkToFit;
            }

            // Absolutely positioned boxes are blockified before layout. The
            // inner display type remains unchanged.
            blockify_out_of_flow_positioned(&mut style);

            // Resolve line-height.
            style.line_height = match text_flow_style.line_height {
                LineHeight::Number(factor) => Length::Px(text_flow_style.font_size * factor),
                LineHeight::Normal => Length::Px(text_flow_style.font_size * DEFAULT_LINE_FACTOR),
                LineHeight::Px(px) => Length::Px(px),
            };
            container_style.text_align = text_flow_style.text_align;

            let child = Arc::new(InheritedCss {
                custom_props: custom_properties,
                text_style: text_style.clone(),
                text_flow_style,
                color_scheme: used_color_scheme,
            });

            if let HtmlNodeType::Text(_) = html_node {
                unreachable!();
            }

            // ── Custom / replaced element (leaf) ──
            if let Some(tag) = html_node.tag_name()
                && registry.tags().contains(&tag)
            {
                perf_scope!(custom_node);
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
                        dom_snapshot: snapshot,
                        dom_id: stack[top_idx].dom,
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
                    text_flow_style,
                };
                let layout = LayoutNode::with_children(style.clone(), [(style, bridge)]);
                let info = InfoNode {
                    kind,
                    children: Vec::new(),
                    dom_id: Some(stack[top_idx].dom),
                };
                let ptr = stack[top_idx].dom;
                results.insert(ptr, (layout, info));
                #[cfg(any(feature = "profile", debug_assertions))]
                {
                    custom_node_time += custom_node.elapsed();
                    node_count += 1;
                }
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

            let kind = NodeKind::Container {
                scroll_x: overflow.x,
                scroll_y: overflow.y,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                style: container_style,
                role,
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

            perf_scope!(child_slot_build);
            if style.display.outer != OuterDisplay::None {
                let parent_tag_name = snapshot.node(stack[top_idx].dom).kind.tag_name();
                for &child in snapshot.children(stack[top_idx].dom) {
                    let child_node = &snapshot.node(child).kind;
                    if let HtmlNodeType::Text(t) = child_node {
                        let t = if parent_tag_name == Some("pre") {
                            let t = t.strip_prefix('\n').unwrap_or(t);
                            normalize_whitespace(t, text_flow_style.white_space)
                        } else if t.chars().all(is_css_newline)
                            && matches!(
                                text_flow_style.white_space,
                                WhiteSpace::Normal | WhiteSpace::Nowrap
                            )
                        {
                            continue;
                        } else {
                            normalize_whitespace(t, text_flow_style.white_space)
                        };

                        let t = match text_style.text_transform {
                            TextTransform::None => t,
                            TextTransform::Uppercase => t.to_ascii_uppercase(),
                            TextTransform::Lowercase => t.to_ascii_lowercase(),
                        };
                        perf_scope!(text_layout);
                        let (layouter, kind) =
                            create_text_node(t, text_style.clone(), text_flow_style, &*measurer);
                        #[cfg(any(feature = "profile", debug_assertions))]
                        {
                            text_layout_time += text_layout.elapsed();
                            node_count += 1;
                        }
                        let mut inline_style = style.clone();
                        inline_style.display = Display {
                            outer: OuterDisplay::Inline,
                            inner: InnerDisplay::Flow,
                        };
                        child_slots.push(ChildSlot::Inline(
                            (inline_style, layouter).into(),
                            Box::new(InfoNode {
                                kind,
                                children: Vec::new(),
                                dom_id: Some(child),
                            }),
                        ));
                    } else if child_node.tag_name() == Some("br") {
                        child_slots.push(ChildSlot::Inline(
                            ItemFragment::LineBreak.into(),
                            Box::new(InfoNode {
                                kind: NodeKind::LineBreak,
                                children: Vec::new(),
                                dom_id: Some(child),
                            }),
                        ));
                    } else if child_node.tag_name() == Some("noscript")
                        && scripting_mode == ScriptingMode::Enabled
                    {
                        // Skip
                    } else {
                        child_slots.push(ChildSlot::Element(element_kids.len()));
                        element_kids.push(child);
                    }
                }
            }
            #[cfg(any(feature = "profile", debug_assertions))]
            {
                child_slot_build_time += child_slot_build.elapsed();
            }

            if element_kids.is_empty() {
                // ── No element children → leaf, build immediately ──
                perf_scope!(leaf_assemble);
                perf_scope!(whitespace_keep);
                let keep = compute_whitespace_keep(&child_slots, &[]);
                #[cfg(any(feature = "profile", debug_assertions))]
                {
                    whitespace_keep_time += whitespace_keep.elapsed();
                }
                let (layout_children, info_children): (Vec<_>, Vec<_>) = child_slots
                    .into_iter()
                    .enumerate()
                    .filter_map(|(i, slot)| {
                        if !keep[i] {
                            return None;
                        }
                        match slot {
                            ChildSlot::Inline(layout, info) => Some((layout, *info)),
                            ChildSlot::Element(_) => None,
                        }
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
                #[cfg(any(feature = "profile", debug_assertions))]
                {
                    leaf_assemble_time += leaf_assemble.elapsed();
                    node_count += 1;
                }
                stack.pop();
            } else {
                // ── Has element children → save state, push children ──
                perf_scope!(push_children);
                let parent_chain = stack[top_idx].chain.clone();
                let parent_container = match &kind {
                    NodeKind::Container { style, .. } => style.clone(),
                    _ => ContainerStyle::default(),
                };
                stack[top_idx].kind = Some(kind);
                stack[top_idx].parent_style = style.clone();
                stack[top_idx].parent_container_style = parent_container;
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
                let child_css = Arc::clone(&stack[top_idx].child);
                perf_scope!(sibling_info);
                let kid_infos = element_sibling_infos(snapshot, &kids_for_push);
                #[cfg(any(feature = "profile", debug_assertions))]
                {
                    sibling_info_time += sibling_info.elapsed();
                }
                let parent_style_for_children = stack[top_idx].parent_style.clone();
                let parent_container_for_children = stack[top_idx].parent_container_style.clone();
                for (&kid, info) in kids_for_push.iter().zip(kid_infos).rev() {
                    stack.push(StackFrame {
                        dom: kid,
                        chain: parent_chain.prepend(info),
                        child: Arc::clone(&child_css),
                        kind: None,
                        style: None,
                        parent_style: parent_style_for_children.clone(),
                        parent_container_style: parent_container_for_children.clone(),
                        child_slots: Vec::new(),
                        element_children: Vec::new(),
                    });
                }
                #[cfg(any(feature = "profile", debug_assertions))]
                {
                    push_children_time += push_children.elapsed();
                }
            }
        } else {
            // ── EXIT phase ────────────────────────────────────────────────
            // Take ownership of frame data for building results.
            perf_scope!(exit_phase);
            perf_scope!(exit_setup);

            let frame = stack.swap_remove(top_idx);

            let mut style = frame.style.as_ref().unwrap().clone();
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

            #[cfg(any(feature = "profile", debug_assertions))]
            {
                exit_setup_time += exit_setup.elapsed();
            }

            perf_scope!(exit_build);
            let mut element_results: Vec<_> = element_results.into_iter().map(Some).collect();

            // Whitespace-only text nodes between two block-level siblings, or adjacent
            // to a `<br>`, would otherwise create stray inline boxes and spurious line
            // boxes in block, flex, and grid containers. Drop them now that every
            // sibling's display is resolved.
            perf_scope!(whitespace_keep);
            let keep = compute_whitespace_keep(&frame.child_slots, &element_results);
            #[cfg(any(feature = "profile", debug_assertions))]
            {
                whitespace_keep_time += whitespace_keep.elapsed();
            }

            let mut all_layout: Vec<LayoutChild> = Vec::with_capacity(frame.child_slots.len());
            let mut all_info: Vec<InfoNode> = Vec::with_capacity(frame.child_slots.len());

            for (i, slot) in frame.child_slots.into_iter().enumerate() {
                if !keep[i] {
                    continue;
                }
                let (lc, ic) = match slot {
                    ChildSlot::Inline(layout, info) => (layout, *info),
                    ChildSlot::Element(index) => element_results[index]
                        .take()
                        .expect("element child result must exist"),
                };
                all_layout.push(lc);
                all_info.push(ic);
            }

            // Collapsible whitespace at a block boundary does not create an
            // anonymous line box. Keeping indentation-only DOM text here made
            // a block such as Scratch's `.page` start one default line-height
            // below the viewport.
            let keep: Vec<bool> = (0..all_layout.len())
                .map(|index| {
                    let collapsible = is_collapsible_whitespace_info(&all_info[index]);
                    let next_to_block = index == 0
                        || index + 1 == all_layout.len()
                        || index
                            .checked_sub(1)
                            .is_some_and(|previous| is_block_layout_child(&all_layout[previous]))
                        || all_layout.get(index + 1).is_some_and(is_block_layout_child);
                    !(collapsible && next_to_block)
                })
                .collect();
            all_layout = all_layout
                .into_iter()
                .zip(&keep)
                .filter_map(|(layout, keep)| keep.then_some(layout))
                .collect();
            all_info = all_info
                .into_iter()
                .zip(keep)
                .filter_map(|(info, keep)| keep.then_some(info))
                .collect();

            // ui_layout currently resolves an auto-width inline flow-root
            // against all available inline space. Floats are shrink-to-fit
            // boxes instead. When their contents expose a fixed CSS width,
            // use that width as the float's content width so carousel slides
            // do not each expand to the full track width.
            if style.display
                == (Display {
                    outer: OuterDisplay::Inline,
                    inner: InnerDisplay::FlowRoot,
                })
                && style.size.auto_behavior == AutoSizeBehavior::ShrinkToFit
                && matches!(style.size.width, LengthOrAuto::Auto)
                && let Some(width) = maximum_fixed_descendant_width(&all_layout)
            {
                style.size.width = LengthOrAuto::Length(Length::Px(width));
            }

            // Grid and flex items are blockified by CSS Display. Keeping an
            // inline direct child makes its text-flow coordinates remain in
            // the parent's inline space, so item placement cannot move the
            // text with its box.
            if matches!(style.display.inner, InnerDisplay::Grid | InnerDisplay::Flex) {
                if style.display.inner == InnerDisplay::Grid {
                    let columns = explicit_grid_track_count(&style.grid_template_columns);
                    let rows = explicit_grid_track_count(&style.grid_template_rows);
                    for child in &mut all_layout {
                        if let LayoutChild::Node(child) = child {
                            resolve_named_grid_area(child, &style.grid_template_areas);
                            resolve_grid_end_line(&mut child.style.grid_column, columns);
                            resolve_grid_end_line(&mut child.style.grid_row, rows);
                        }
                    }
                }
                for child in &mut all_layout {
                    if let LayoutChild::Node(child) = child
                        && child.style.display.outer == OuterDisplay::Inline
                        && !child.style.position.kind.is_out_of_flow()
                    {
                        child.style.display.outer = OuterDisplay::Block;
                    }
                }
            }

            let layout = LayoutNode::with_children(style, all_layout);
            let info = InfoNode {
                kind: final_kind,
                children: all_info,
                dom_id: Some(frame.dom),
            };
            let ptr = frame.dom;
            results.insert(ptr, (layout, info));
            #[cfg(any(feature = "profile", debug_assertions))]
            {
                exit_build_time += exit_build.elapsed();
                exit_phase_time += exit_phase.elapsed();
                node_count += 1;
            }
        }
    }

    profile_log!(
        target: "LayoutRun",
        log::Level::Info,
        "[LayoutMetrics] total: {:?} (nodes: {})",
        total.elapsed(),
        node_count,
    );
    profile_log!(
        target: "LayoutRun",
        log::Level::Info,
        "[LayoutMetrics] css_match: {:?} | apply_decl: {:?}",
        css_match_time,
        apply_decl_time,
    );
    profile_log!(
        target: "LayoutRun",
        log::Level::Info,
        "[LayoutCandidates] elements: {} | examined: {} | matched: {}",
        cand_stats.elements_checked,
        cand_stats.candidates_examined,
        cand_stats.selectors_matched,
    );
    profile_log!(
        target: "LayoutRun",
        log::Level::Info,
        "[LayoutCandidates] query_time: {:?} | sel_match_time: {:?} | insert_time: {:?}",
        cand_stats.query_candidates_time,
        cand_stats.selector_match_time,
        cand_stats.cascade_insert_time,
    );
    profile_log!(
        target: "LayoutRun",
        log::Level::Info,
        "[LayoutMetrics] custom_node: {:?} | text_layout: {:?} | exit_phase: {:?}",
        custom_node_time,
        text_layout_time,
        exit_phase_time,
    );
    profile_log!(
        target: "LayoutRun",
        log::Level::Info,
        "[LayoutMetrics] sibling_info: {:?} | whitespace_keep: {:?}",
        sibling_info_time,
        whitespace_keep_time,
    );
    profile_log!(
        target: "LayoutRun",
        log::Level::Info,
        "[LayoutMetrics] enter_prep: {:?} | child_slot_build: {:?} | leaf_assemble: {:?}",
        enter_prep_time,
        child_slot_build_time,
        leaf_assemble_time,
    );
    profile_log!(
        target: "LayoutRun",
        log::Level::Info,
        "[LayoutMetrics] push_children: {:?} | exit_setup: {:?} | exit_build: {:?}",
        push_children_time,
        exit_setup_time,
        exit_build_time,
    );
    profile_log!(
        target: "LayoutRun",
        log::Level::Info,
        "[LayoutMetrics] rest: {:?}",
        total.elapsed().saturating_sub(
            css_match_time
                + apply_decl_time
                + custom_node_time
                + enter_prep_time
                + child_slot_build_time
                + leaf_assemble_time
                + push_children_time
                + exit_setup_time
                + exit_build_time
        ),
    );

    results
        .remove(&root)
        .expect("root must have been processed")
}

// ── Whitespace helpers ──────────────────────────────────────────────────────

fn is_css_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

fn is_css_newline(c: char) -> bool {
    matches!(c, '\n' | '\r')
}

/// True when an inline child is a whitespace-only text node that renders as
/// nothing more than a collapsible space.
fn is_collapsible_whitespace_text(info: &InfoNode) -> bool {
    matches!(
        &info.kind,
        NodeKind::Text {
            text, flow_style, ..
        } if text.chars().all(is_css_whitespace)
            && matches!(
                flow_style.white_space,
                WhiteSpace::Normal | WhiteSpace::Nowrap
            )
    )
}

/// Classification of the nearest layout-participating sibling of a slot.
#[derive(PartialEq)]
enum Neighbour {
    /// No participating sibling (container edge) or only `display:none` boxes.
    None,
    /// A block-level element box.
    Block,
    /// A `<br>` line break.
    LineBreak,
    /// Any other inline content (text, inline element, replaced, …).
    Inline,
}

/// Inspects the nearest layout-participating sibling of `slot_index` in the
/// given `step` direction (`-1` = previous, `+1` = next).
///
/// Collapsible-whitespace-only text siblings are always skipped. `<br>`
/// siblings are skipped when `skip_line_break` is set, so the caller can tell
/// "adjacent to a `<br>`" apart from "the nearest block beyond a `<br>`".
fn neighbour(
    slots: &[ChildSlot],
    element_results: &[Option<(LayoutChild, InfoNode)>],
    slot_index: usize,
    step: isize,
    skip_line_break: bool,
) -> Neighbour {
    let mut idx = slot_index;

    loop {
        let Some(next) = (if step < 0 {
            idx.checked_sub(1)
        } else {
            idx.checked_add(1)
        }) else {
            return Neighbour::None;
        };
        if next >= slots.len() {
            return Neighbour::None;
        }
        idx = next;

        match &slots[idx] {
            ChildSlot::Element(element_index) => {
                let Some((child, _)) = element_results[*element_index].as_ref() else {
                    continue;
                };

                // display:none elements do not participate in layout.
                if matches!(
                    child,
                    LayoutChild::Node(node)
                        if node.style.display.outer == OuterDisplay::None
                ) {
                    continue;
                }

                return if matches!(
                    child,
                    LayoutChild::Node(node)
                        if node.style.display.outer == OuterDisplay::Block
                ) {
                    Neighbour::Block
                } else {
                    Neighbour::Inline
                };
            }
            ChildSlot::Inline(_, info) => {
                if matches!(info.kind, NodeKind::LineBreak) {
                    if !skip_line_break {
                        return Neighbour::LineBreak;
                    }
                } else if !is_collapsible_whitespace_text(info) {
                    return Neighbour::Inline;
                }
            }
        }
    }
}

/// Per-child keep decision for whitespace-only text nodes.
///
/// A whitespace-only text node is dropped when it is adjacent on *either*
/// side to a block-level sibling or to a `<br>`; otherwise it would become a
/// stray inline box that creates a spurious line box in block containers.
fn compute_whitespace_keep(
    slots: &[ChildSlot],
    element_results: &[Option<(LayoutChild, InfoNode)>],
) -> Vec<bool> {
    (0..slots.len())
        .map(|i| match &slots[i] {
            ChildSlot::Inline(_, info) => {
                if !is_collapsible_whitespace_text(info) {
                    return true;
                }
                // Drop if either side is a block-level box or a `<br>` (a line
                // break starts a new line, so adjacent whitespace is spurious).
                let side_drops = |step: isize| {
                    neighbour(slots, element_results, i, step, true) == Neighbour::Block
                        || neighbour(slots, element_results, i, step, false) == Neighbour::LineBreak
                };
                !(side_drops(-1) || side_drops(1))
            }
            ChildSlot::Element(_) => true,
        })
        .collect()
}

// ── Grid layout helpers ─────────────────────────────────────────────────────

fn explicit_grid_track_count(tracks: &[GridTrack]) -> usize {
    tracks
        .iter()
        .map(|track| match track {
            GridTrack::Repeat(GridRepeat::Count(count), pattern) => {
                count.saturating_mul(explicit_grid_track_count(pattern))
            }
            GridTrack::Repeat(GridRepeat::AutoFit | GridRepeat::AutoFill, _) => 0,
            _ => 1,
        })
        .sum()
}

fn resolve_grid_end_line(placement: &mut GridPlacement, track_count: usize) {
    if !matches!(placement.end, GRID_LINE_TO_END) || track_count == 0 {
        return;
    }

    placement.end = GridPlacementEnd::Line(track_count + 1);
}

fn resolve_named_grid_area(node: &mut LayoutNode, areas: &[Vec<String>]) {
    let Some(name) = node.style.grid_area.as_deref() else {
        return;
    };
    let mut min_column = usize::MAX;
    let mut max_column = 0;
    let mut min_row = usize::MAX;
    let mut max_row = 0;
    for (row, names) in areas.iter().enumerate() {
        for (column, area) in names.iter().enumerate() {
            if area == name {
                min_column = min_column.min(column);
                max_column = max_column.max(column);
                min_row = min_row.min(row);
                max_row = max_row.max(row);
            }
        }
    }
    if min_column == usize::MAX {
        return;
    }
    node.style.grid_column = GridPlacement {
        start: Some(min_column + 1),
        end: GridPlacementEnd::Line(max_column + 2),
    };
    node.style.grid_row = GridPlacement {
        start: Some(min_row + 1),
        end: GridPlacementEnd::Line(max_row + 2),
    };
    node.style.grid_area = None;
}

// ── Text normalization ──────────────────────────────────────────────────────

pub fn normalize_whitespace(text: &str, white_space: WhiteSpace) -> String {
    let text = text.replace("\r\n", "\n");
    let text = text.replace(['\r', '\x0c'], "\n");

    let mut result = String::new();
    let mut prev_was_space = false;

    for c in text.chars() {
        match white_space {
            WhiteSpace::Normal | WhiteSpace::Nowrap => {
                if is_css_whitespace(c) {
                    if !prev_was_space {
                        result.push(' ');
                    }
                    prev_was_space = true;
                } else {
                    result.push(c);
                    prev_was_space = false;
                }
            }

            WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces => {
                result.push(c);
                prev_was_space = false;
            }

            WhiteSpace::PreLine => {
                if c == '\n' {
                    result.push('\n');
                    prev_was_space = false;
                } else if is_css_whitespace(c) {
                    if !prev_was_space {
                        result.push(' ');
                    }
                    prev_was_space = true;
                } else {
                    result.push(c);
                    prev_was_space = false;
                }
            }
        }
    }

    if white_space == WhiteSpace::PreLine {
        while result.ends_with('\n') {
            result.pop();
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
    text_flow_style: TextFlowStyle,
    measurer: &dyn text::TextMeasurer,
) -> (TextFlowLayouter, NodeKind) {
    perf_scope!(measure);
    let request = text::TextMeasureRequest {
        text: text.clone(),
        attribute: text::TextAttribute {
            style: text_style.clone(),
            flow_style: text_flow_style,
        },
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
    profile_log!(
        target: "Layouter",
        log::Level::Info,
        "measure_shaped: text={:?} len={} took={:?}",
        crate::profile::text_preview(&text),
        text.len(),
        measure.elapsed(),
    );

    let layouter = TextFlowLayouter::new(text.clone(), text_flow_style, clusters);
    let kind = NodeKind::Text {
        text,
        style: text_style,
        flow_style: text_flow_style,
        text_id: layouter.id,
    };
    (layouter, kind)
}

// ── Color scheme resolution ─────────────────────────────────────────────────

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

// ── CSS candidate collection ────────────────────────────────────────────────

/// Aggregated CSS candidate-matching statistics for a single layout build.
#[cfg(any(feature = "profile", debug_assertions))]
#[derive(Default)]
struct CandidateMetrics {
    elements_checked: u64,
    candidates_examined: u64,
    selectors_matched: u64,
    query_candidates_time: std::time::Duration,
    selector_match_time: std::time::Duration,
    cascade_insert_time: std::time::Duration,
}

fn collect_candidates(
    rule_set: &RuleSet,
    chain: &ElementChain,
    #[cfg(any(feature = "profile", debug_assertions))] stats: &mut CandidateMetrics,
) -> (Properties, Properties) {
    let mut properties = HashMap::new();
    let mut custom_properties = HashMap::new();

    let element = match chain.first() {
        Some(el) => el,
        None => return (properties, custom_properties),
    };

    #[cfg(any(feature = "profile", debug_assertions))]
    {
        stats.elements_checked += 1;
    }

    // The candidate iterator is lazy; under profiling it is materialized so
    // query time is measured separately from selector matching.
    perf_scope!(query);
    #[cfg(any(feature = "profile", debug_assertions))]
    let candidates_iter: Vec<_> = rule_set.query_candidates(element).collect();
    #[cfg(not(any(feature = "profile", debug_assertions)))]
    let candidates_iter = rule_set.query_candidates(element);
    #[cfg(any(feature = "profile", debug_assertions))]
    {
        stats.query_candidates_time += query.elapsed();
    }

    for group in candidates_iter {
        #[cfg(any(feature = "profile", debug_assertions))]
        {
            stats.candidates_examined += 1;
        }

        // Declarations sharing an identical selector are grouped, so the
        // (comparatively expensive) selector walk happens once per group.
        perf_scope!(sel_match);
        let matches_sel = group.selector.matches(chain);
        #[cfg(any(feature = "profile", debug_assertions))]
        {
            stats.selector_match_time += sel_match.elapsed();
            if matches_sel {
                stats.selectors_matched += 1;
            }
        }
        if !matches_sel {
            continue;
        }

        for &decl_idx in &group.decls {
            let decl = &rule_set.declarations()[decl_idx];

            let target = if decl.name.starts_with("--") {
                &mut custom_properties
            } else {
                &mut properties
            };

            perf_scope!(cascade);
            let should_replace = match target.get(&decl.name) {
                Some(current) => decl.outranks(current),
                None => true,
            };

            if should_replace {
                target.insert(decl.name.clone(), decl.clone());
            }

            #[cfg(any(feature = "profile", debug_assertions))]
            {
                stats.cascade_insert_time += cascade.elapsed();
            }
        }
    }

    (properties, custom_properties)
}
