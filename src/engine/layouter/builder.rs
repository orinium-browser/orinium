//! Layout builder, which transforms a DOM tree into a UI layout.

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
use crate::engine::layouter::types::{TextFlowStyle, VerticalAlign, Visibility, WhiteSpace};
use crate::engine::tree::NodeRef;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ui_layout::{
    AlignContent, AlignItems, AutoSizeBehavior, BoxSizing, Display, FlexDirection, FlexWrap,
    GridPlacement, GridRepeat, GridTrack, InnerDisplay, ItemFragment, JustifyContent, LayoutChild,
    LayoutNode, Length, LengthOrAuto, OuterDisplay, Position, Style,
};

use super::css_resolver::{
    MediaEnvironment, ResolvedStyles, RuleSet, resolve_inline_style, set_inline_custom_property,
};
use super::text_layouter::TextFlowLayouter;
use super::types::{
    Background, BackgroundDimension, BackgroundOffset, BackgroundPosition, BackgroundPositionAxis,
    BackgroundRepeat, BackgroundSize, BorderRadius, BorderStyle, Color, ColorScheme, ColorStop,
    ContainerRole, ContainerStyle, CornerRadius, CssFloat, FontStyle, FontWeight, Gradient,
    GradientKind, InfoNode, LineHeight, NodeKind, Overflow, RadialShape, RadialSizeKind, TextAlign,
    TextDecoration, TextStyle, TextTransform,
};
use crate::engine::renderer_model::Image;
use crate::engine::ui::custom_node_bridge::CustomNodeBridge;
use crate::engine::ui::registry::{ComponentRegistry, CustomNodeContext, DomWriteBack};

pub(crate) const DEFAULT_LINE_FACTOR: f32 = 1.2;

fn background_dimension(value: &CssValue, font_size: f32) -> Option<BackgroundDimension> {
    match value {
        CssValue::Keyword(keyword) if keyword.eq_ignore_ascii_case("auto") => {
            Some(BackgroundDimension::Auto)
        }
        CssValue::Number(value) if *value == 0.0 => Some(BackgroundDimension::Length(0.0)),
        CssValue::Length(value, Unit::Px) => Some(BackgroundDimension::Length(*value)),
        CssValue::Length(value, Unit::Em) => Some(BackgroundDimension::Length(*value * font_size)),
        CssValue::Length(value, Unit::Rem) => Some(BackgroundDimension::Length(*value * 16.0)),
        CssValue::Length(value, Unit::Percent) => {
            Some(BackgroundDimension::Percent(*value / 100.0))
        }
        _ => None,
    }
}

fn parse_background_size(value: &CssValue, font_size: f32) -> Option<BackgroundSize> {
    match value {
        CssValue::Keyword(keyword) if keyword.eq_ignore_ascii_case("contain") => {
            Some(BackgroundSize::Contain)
        }
        CssValue::Keyword(keyword) if keyword.eq_ignore_ascii_case("cover") => {
            Some(BackgroundSize::Cover)
        }
        CssValue::Keyword(keyword) if keyword.eq_ignore_ascii_case("auto") => {
            Some(BackgroundSize::Auto)
        }
        CssValue::List(values) => match values.as_slice() {
            [width] => Some(BackgroundSize::Explicit {
                width: background_dimension(width, font_size)?,
                height: BackgroundDimension::Auto,
            }),
            [width, height] => Some(BackgroundSize::Explicit {
                width: background_dimension(width, font_size)?,
                height: background_dimension(height, font_size)?,
            }),
            _ => None,
        },
        _ => Some(BackgroundSize::Explicit {
            width: background_dimension(value, font_size)?,
            height: BackgroundDimension::Auto,
        }),
    }
}

fn background_offset(value: &CssValue, font_size: f32) -> Option<BackgroundOffset> {
    match background_dimension(value, font_size)? {
        BackgroundDimension::Length(value) => Some(BackgroundOffset::Length(value)),
        BackgroundDimension::Percent(value) => Some(BackgroundOffset::Percent(value)),
        BackgroundDimension::Auto => None,
    }
}

fn parse_background_position(value: &CssValue, font_size: f32) -> Option<BackgroundPosition> {
    let values: Vec<&CssValue> = match value {
        CssValue::List(values) => values.iter().collect(),
        _ => vec![value],
    };
    if values.is_empty() || values.len() > 4 {
        return None;
    }

    // The 3/4-value syntax consists of edge-and-offset pairs and may put the
    // vertical pair first (Scratch uses `bottom 32px right 50%`).
    if values.len() >= 3 {
        let mut position = BackgroundPosition::default();
        let mut has_x = false;
        let mut has_y = false;
        let mut index = 0;
        while index < values.len() {
            let CssValue::Keyword(edge) = values[index] else {
                return None;
            };
            let offset = values
                .get(index + 1)
                .and_then(|value| background_offset(value, font_size));
            let consumed_offset = offset.is_some();
            let offset = offset.unwrap_or_default();
            match edge.to_ascii_lowercase().as_str() {
                "left" if !has_x => {
                    position.x = BackgroundPositionAxis::Start(offset);
                    has_x = true;
                }
                "right" if !has_x => {
                    position.x = BackgroundPositionAxis::End(offset);
                    has_x = true;
                }
                "top" if !has_y => {
                    position.y = BackgroundPositionAxis::Start(offset);
                    has_y = true;
                }
                "bottom" if !has_y => {
                    position.y = BackgroundPositionAxis::End(offset);
                    has_y = true;
                }
                "center" if !has_x => {
                    position.x = BackgroundPositionAxis::Center(offset);
                    has_x = true;
                }
                "center" if !has_y => {
                    position.y = BackgroundPositionAxis::Center(offset);
                    has_y = true;
                }
                _ => return None,
            }
            index += if consumed_offset { 2 } else { 1 };
        }
        if !has_x {
            position.x = BackgroundPositionAxis::Center(BackgroundOffset::Zero);
        }
        if !has_y {
            position.y = BackgroundPositionAxis::Center(BackgroundOffset::Zero);
        }
        return Some(position);
    }

    fn keyword_axis(keyword: &str, horizontal: bool) -> Option<BackgroundPositionAxis> {
        let zero = BackgroundOffset::Zero;
        match (keyword, horizontal) {
            ("left", true) | ("top", false) => Some(BackgroundPositionAxis::Start(zero)),
            ("right", true) | ("bottom", false) => Some(BackgroundPositionAxis::End(zero)),
            ("center", _) => Some(BackgroundPositionAxis::Center(zero)),
            _ => None,
        }
    }

    if values.len() == 1 {
        return match values[0] {
            CssValue::Keyword(keyword) => {
                let keyword = keyword.to_ascii_lowercase();
                if matches!(keyword.as_str(), "top" | "bottom") {
                    Some(BackgroundPosition {
                        x: BackgroundPositionAxis::Center(BackgroundOffset::Zero),
                        y: keyword_axis(&keyword, false)?,
                    })
                } else {
                    Some(BackgroundPosition {
                        x: keyword_axis(&keyword, true)?,
                        y: BackgroundPositionAxis::Center(BackgroundOffset::Zero),
                    })
                }
            }
            value => Some(BackgroundPosition {
                x: BackgroundPositionAxis::Start(background_offset(value, font_size)?),
                y: BackgroundPositionAxis::Center(BackgroundOffset::Zero),
            }),
        };
    }

    let first_keyword = match values[0] {
        CssValue::Keyword(keyword) => Some(keyword.to_ascii_lowercase()),
        _ => None,
    };
    let second_keyword = match values[1] {
        CssValue::Keyword(keyword) => Some(keyword.to_ascii_lowercase()),
        _ => None,
    };
    let reversed = first_keyword
        .as_deref()
        .is_some_and(|keyword| matches!(keyword, "top" | "bottom"))
        && second_keyword
            .as_deref()
            .is_some_and(|keyword| matches!(keyword, "left" | "right" | "center"));
    let axis = |value: &CssValue, horizontal: bool| match value {
        CssValue::Keyword(keyword) => keyword_axis(&keyword.to_ascii_lowercase(), horizontal),
        value => Some(BackgroundPositionAxis::Start(background_offset(
            value, font_size,
        )?)),
    };
    if reversed {
        Some(BackgroundPosition {
            x: axis(values[1], true)?,
            y: axis(values[0], false)?,
        })
    } else {
        Some(BackgroundPosition {
            x: axis(values[0], true)?,
            y: axis(values[1], false)?,
        })
    }
}

fn parse_background_repeat(value: &CssValue) -> Option<BackgroundRepeat> {
    let keyword = match value {
        CssValue::Keyword(keyword) => keyword.as_str(),
        _ => return None,
    };
    match keyword.to_ascii_lowercase().as_str() {
        "repeat" => Some(BackgroundRepeat::Repeat),
        "repeat-x" => Some(BackgroundRepeat::RepeatX),
        "repeat-y" => Some(BackgroundRepeat::RepeatY),
        "no-repeat" => Some(BackgroundRepeat::NoRepeat),
        _ => None,
    }
}

fn apply_background_shorthand_geometry(
    value: &CssValue,
    font_size: f32,
    container_style: &mut ContainerStyle,
) {
    container_style.background_repeat = BackgroundRepeat::default();
    container_style.background_size = BackgroundSize::default();
    container_style.background_position = BackgroundPosition::default();

    let values: Vec<&CssValue> = match value {
        CssValue::List(values) => values.iter().collect(),
        _ => vec![value],
    };
    if let Some(repeat) = values
        .iter()
        .find_map(|value| parse_background_repeat(value))
    {
        container_style.background_repeat = repeat;
    }

    let slash = values
        .iter()
        .position(|value| matches!(value, CssValue::Keyword(keyword) if keyword.as_str() == "/"));
    if let Some(slash) = slash {
        let size_values: Vec<CssValue> = values[slash + 1..]
            .iter()
            .take_while(|value| {
                background_dimension(value, font_size).is_some()
                    || matches!(value, CssValue::Keyword(keyword) if matches!(keyword.to_ascii_lowercase().as_str(), "contain" | "cover" | "auto"))
            })
            .map(|value| (*value).clone())
            .collect();
        let size_value = match size_values.as_slice() {
            [value] => Some(value.clone()),
            [] => None,
            _ => Some(CssValue::List(size_values)),
        };
        if let Some(size) = size_value
            .as_ref()
            .and_then(|value| parse_background_size(value, font_size))
        {
            container_style.background_size = size;
        }
    }

    let position_values: Vec<CssValue> = values[..slash.unwrap_or(values.len())]
        .iter()
        .filter(|value| {
            background_offset(value, font_size).is_some()
                || matches!(
                    value,
                    CssValue::Keyword(keyword)
                        if matches!(keyword.to_ascii_lowercase().as_str(), "left" | "right" | "top" | "bottom" | "center")
                )
        })
        .map(|value| (*value).clone())
        .collect();
    let position_value = match position_values.as_slice() {
        [value] => Some(value.clone()),
        [] => None,
        _ => Some(CssValue::List(position_values)),
    };
    if let Some(position) = position_value
        .as_ref()
        .and_then(|value| parse_background_position(value, font_size))
    {
        container_style.background_position = position;
    }
}

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
        previous_siblings: ElementChain::default(),
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
    pub custom_props: Properties,
    pub text_style: TextStyle,
    pub text_flow_style: TextFlowStyle,
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
    child: Arc<InheritedCss>,
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
            let mut text_flow_style = child_css.text_flow_style.clone();
            let mut container_style = ContainerStyle::default();
            let mut style = Style::default();
            let mut overflow = Overflow::default();
            // Collect CSS candidates.
            let (candidates, custom_property_candidates) =
                if let HtmlNodeType::Element { .. } = html_node {
                    Some(collect_candidates(
                        &rule_set,
                        &chain_for_css,
                    ))
                } else {
                    None
                }
                .unzip();

            let mut custom_properties = child_css.custom_props.clone();
            if let Some(own) = custom_property_candidates {
                custom_properties.extend(own);
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
                                &mut custom_properties,
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
                &mut text_flow_style,
                &mut overflow,
                used_color_scheme,
            );

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
                        dom_snapshot: &snapshot,
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
                        let (layouter, kind) =
                            create_text_node(t, text_style.clone(), text_flow_style, &*measurer);
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

            if element_kids.is_empty() {
                // ── No element children → leaf, build immediately ──
                let keep = compute_whitespace_keep(&child_slots, &[]);
                let (layout_children, info_children): (Vec<_>, Vec<_>) = child_slots
                    .into_iter()
                    .enumerate()
                    .filter_map(|(i, slot)| {
                        if !keep[i] {
                            return None;
                        }
                        match slot {
                            ChildSlot::Inline(layout, info) => Some((layout, info)),
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
                let child_css = Arc::clone(&stack[top_idx].child);
                let kid_infos = element_sibling_infos(snapshot, &kids_for_push);
                for (&kid, info) in kids_for_push.iter().zip(kid_infos).rev() {
                    stack.push(StackFrame {
                        dom: kid,
                        chain: parent_chain.prepend(info),
                        child: Arc::clone(&child_css),
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

            let mut element_results: Vec<_> = element_results.into_iter().map(Some).collect();

            // Whitespace-only text nodes between two block-level siblings, or adjacent
            // to a `<br>`, would otherwise create stray inline boxes and spurious line
            // boxes in block, flex, and grid containers. Drop them now that every
            // sibling's display is resolved.
            let keep = compute_whitespace_keep(&frame.child_slots, &element_results);

            let mut all_layout: Vec<LayoutChild> = Vec::with_capacity(frame.child_slots.len());
            let mut all_info: Vec<InfoNode> = Vec::with_capacity(frame.child_slots.len());

            for (i, slot) in frame.child_slots.into_iter().enumerate() {
                if !keep[i] {
                    continue;
                }
                let (lc, ic) = match slot {
                    ChildSlot::Inline(layout, info) => (layout, info),
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
                            resolve_grid_end_span(&mut child.style.grid_column, columns);
                            resolve_grid_end_span(&mut child.style.grid_row, rows);
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
        }
    }

    results
        .remove(&root)
        .expect("root must have been processed")
}

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

fn resolve_grid_end_span(placement: &mut GridPlacement, track_count: usize) {
    if placement.span != GRID_SPAN_TO_END || track_count == 0 {
        return;
    }
    let start = placement.start.unwrap_or(1);
    placement.span = track_count.saturating_add(1).saturating_sub(start).max(1);
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
        span: max_column - min_column + 1,
    };
    node.style.grid_row = GridPlacement {
        start: Some(min_row + 1),
        span: max_row - min_row + 1,
    };
    node.style.grid_area = None;
}

fn maximum_fixed_descendant_width(children: &[LayoutChild]) -> Option<f32> {
    children
        .iter()
        .filter_map(|child| match child {
            LayoutChild::Node(node) => {
                let own = match node.style.size.width {
                    LengthOrAuto::Length(Length::Px(width))
                        if width.is_finite() && width >= 0.0 =>
                    {
                        Some(width)
                    }
                    _ => None,
                };
                own.into_iter()
                    .chain(maximum_fixed_descendant_width(&node.children))
                    .max_by(f32::total_cmp)
            }
            _ => None,
        })
        .max_by(f32::total_cmp)
}

fn is_collapsible_whitespace_info(info: &InfoNode) -> bool {
    matches!(&info.kind, NodeKind::Text { text, .. } if text.trim().is_empty())
}

fn is_block_layout_child(child: &LayoutChild) -> bool {
    child.node().is_some_and(|node| {
        node.style.display.outer == OuterDisplay::Block
            || (node.style.display.inner == InnerDisplay::FlowRoot
                && node.style.size.auto_behavior == AutoSizeBehavior::ShrinkToFit)
    })
}

/// Correct the used horizontal margins of oversized block-level boxes.
///
/// CSS 2.1 treats auto horizontal margins as zero when a block's used width
/// exceeds its containing block. `ui_layout` currently divides the negative
/// free space between two auto margins, which incorrectly centers wide
/// carousel tracks outside their clipping viewport.
pub fn correct_oversized_auto_horizontal_margins(node: &mut LayoutNode) {
    let parent_content = node.layout_box.iter().next().map(|model| model.content_box);

    for child in &mut node.children {
        let LayoutChild::Node(child) = child else {
            continue;
        };

        if let Some(parent_content) = parent_content
            && child.style.display.outer == OuterDisplay::Block
            && child.style.spacing.margin_left == LengthOrAuto::Auto
            && child.style.spacing.margin_right == LengthOrAuto::Auto
            && let Some(child_box) = child.layout_box.iter().next()
            && child_box.border_box.width > parent_content.width
        {
            let shift_x = parent_content.x - child_box.border_box.x;
            if let ui_layout::LayoutBox::BlockBox(model) = &mut child.layout_box {
                model.border_box.x += shift_x;
                model.padding_box.x += shift_x;
                model.content_box.x += shift_x;
                model.children_box.x += shift_x;
            }
        }

        correct_oversized_auto_horizontal_margins(child);
    }
}

/// Resolve an auto grid track from the measured contents of an auto-sized
/// block item instead of letting that item consume all available width.
///
/// During the intrinsic grid pass `ui_layout` currently measures block flex
/// containers with their containing width. In a template such as
/// `1fr auto 1fr`, that makes the auto track take the entire grid and leaves
/// both fraction tracks at zero. The first layout still records the flex
/// contents' actual extent in `children_box`, so use that intrinsic width and
/// let a second layout resolve the tracks correctly.
pub fn constrain_auto_grid_track_items(node: &mut LayoutNode) -> bool {
    let mut changed = false;
    for child in &mut node.children {
        if let LayoutChild::Node(child) = child {
            changed |= constrain_auto_grid_track_items(child);
        }
    }

    if node.style.display.inner != InnerDisplay::Grid || node.style.grid_template_columns.is_empty()
    {
        return changed;
    }

    let mut item_index = 0usize;
    for child in &mut node.children {
        let LayoutChild::Node(child) = child else {
            continue;
        };
        if child.style.display.outer == OuterDisplay::None
            || child.style.position.kind.is_out_of_flow()
        {
            continue;
        }

        let auto_track = node
            .style
            .grid_template_columns
            .get(item_index)
            .is_some_and(|track| matches!(track, GridTrack::Breadth(LengthOrAuto::Auto)));
        item_index += 1;
        let self_aligned = child.style.spacing.margin_left == LengthOrAuto::Auto
            || child.style.spacing.margin_right == LengthOrAuto::Auto;
        if (!auto_track && !self_aligned) || child.style.size.width != LengthOrAuto::Auto {
            continue;
        }

        let Some(model) = child.layout_box.iter().next() else {
            continue;
        };
        let intrinsic_width = model.children_box.width.max(0.0);
        if intrinsic_width > 0.0 && intrinsic_width + 0.5 < model.content_box.width {
            child.style.size.width = LengthOrAuto::Length(Length::Px(intrinsic_width));
            changed = true;
        }
    }

    changed
}

/// Ensure text custom objects positioned as flex items produce their text-flow
/// cache entry. `ui_layout` measures and positions direct custom flex items,
/// but does not call their `layout` method, so render-time lookup otherwise
/// finds no spans for text directly inside an `inline-flex` element.
pub fn refresh_missing_text_layout_results(
    layout: &mut LayoutNode,
    info: &InfoNode,
    viewport: (f32, f32),
) {
    let containing = layout
        .layout_box
        .iter()
        .next()
        .map(|model| (model.content_box.width, model.content_box.height))
        .unwrap_or(viewport);

    for (layout_child, info_child) in layout.children.iter_mut().zip(&info.children) {
        match (layout_child, &info_child.kind) {
            (LayoutChild::Node(child), _) => {
                refresh_missing_text_layout_results(child, info_child, viewport);
            }
            (LayoutChild::Custom(custom), NodeKind::Text { text_id, .. })
                if TextFlowLayouter::get_result(*text_id).is_none() =>
            {
                let Some(box_model) = custom.result().map(|result| result.box_model.clone()) else {
                    continue;
                };
                let line_height = custom
                    .style()
                    .line_height
                    .resolve_with(Some(containing.0), viewport.0, viewport.1)
                    .unwrap_or(box_model.border_box.height);
                let _ = custom.layouter_mut().layout(&ui_layout::LayoutContext {
                    containing_block_width: Some(containing.0),
                    containing_block_height: Some(containing.1),
                    start_pos: (box_model.border_box.x, box_model.border_box.y),
                    available_inline_size: box_model.border_box.width.max(1.0),
                    line_height,
                    viewport_width: viewport.0,
                    viewport_height: viewport.1,
                });
            }
            _ => {}
        }
    }
}

/// Keep adjacent atomic inline boxes from overlapping their padding or
/// horizontal margins.
///
/// `ui_layout` currently advances past an inline flow-root using its content
/// width. CSS inline-blocks advance by their margin-box width instead.
pub fn correct_atomic_inline_spacing(node: &mut LayoutNode) {
    correct_atomic_inline_spacing_impl(node, None);
}

pub fn correct_atomic_inline_spacing_with_info(node: &mut LayoutNode, info: &InfoNode) {
    correct_atomic_inline_spacing_impl(node, Some(info));
}

fn correct_atomic_inline_spacing_impl(node: &mut LayoutNode, info: Option<&InfoNode>) {
    let containing_rect = node.layout_box.iter().next().map(|model| model.content_box);
    let containing_width = containing_rect.map(|rect| rect.width);
    let text_align = info
        .and_then(|info| match &info.kind {
            NodeKind::Container { style, .. } => Some(style.text_align),
            _ => None,
        })
        .unwrap_or_default();
    let wraps_inline_content = matches!(
        node.style.display.inner,
        InnerDisplay::Flow | InnerDisplay::FlowRoot
    );
    let mut previous: Option<(f32, f32)> = None;
    let mut line_y: Option<f32> = None;
    let mut line_start_x = 0.0;
    let mut line_bottom = 0.0;
    let mut line_margin_bottom = 0.0;
    let mut preceding_block_bottom: Option<(f32, f32)> = None;

    for (child_index, child) in node.children.iter_mut().enumerate() {
        let LayoutChild::Node(child) = child else {
            continue;
        };

        let child_info = info.and_then(|info| info.children.get(child_index));
        correct_atomic_inline_spacing_impl(child, child_info);

        let is_atomic_inline = child.style.display.outer == OuterDisplay::Inline
            && child.style.display.inner != InnerDisplay::Flow;

        if is_atomic_inline && let Some(model) = child.layout_box.iter().next() {
            let rect = model.border_box;
            let margin_left = fixed_nonnegative_px(&child.style.spacing.margin_left);
            let margin_right = fixed_nonnegative_px(&child.style.spacing.margin_right);
            let margin_top = fixed_nonnegative_px(&child.style.spacing.margin_top);
            let margin_bottom = fixed_nonnegative_px(&child.style.spacing.margin_bottom);

            if line_y.is_none_or(|y| (y - rect.y).abs() >= 0.5) {
                previous = None;
                line_y = Some(rect.y);
                line_start_x = rect.x;
                line_bottom = rect.bottom();
                line_margin_bottom = margin_bottom;
            }

            let mut desired_x = match previous {
                Some((right, previous_margin_right)) => right + previous_margin_right + margin_left,
                None => {
                    let margin_width = margin_left + rect.width + margin_right;
                    let aligned_x = containing_rect.map_or(rect.x, |containing| {
                        let free_space = (containing.width - margin_width).max(0.0);
                        containing.x
                            + match text_align {
                                TextAlign::Left => 0.0,
                                TextAlign::Center => free_space / 2.0,
                                TextAlign::Right => free_space,
                            }
                    });
                    aligned_x + margin_left
                }
            };
            // ui_layout positions atomic inline boxes at the line origin but
            // does not include their vertical margins in that position. The
            // margin box, rather than the border box, is what participates in
            // inline formatting (notably a full-width inline-block <main>
            // placed below a fixed header).
            let mut desired_y = rect.y + margin_top;
            if previous.is_none()
                && let Some((block_bottom, block_margin_bottom)) = preceding_block_bottom
            {
                desired_y = desired_y.max(block_bottom + block_margin_bottom + margin_top);
            }
            let exceeds_line = previous.is_some()
                && wraps_inline_content
                && containing_width
                    .is_some_and(|width| desired_x + rect.width + margin_right > width + 0.5);
            if exceeds_line {
                desired_x = line_start_x + margin_left;
                desired_y = line_bottom + line_margin_bottom + margin_top;
                line_y = Some(desired_y);
                line_bottom = desired_y + rect.height;
                line_margin_bottom = margin_bottom;
            }

            let shift_x = desired_x - rect.x;
            if shift_x.abs() >= 0.01 {
                shift_layout_box_x(&mut child.layout_box, shift_x);
            }
            let shift_y = desired_y - rect.y;
            if shift_y.abs() >= 0.01 {
                shift_layout_box_y(&mut child.layout_box, shift_y);
            }

            line_bottom = line_bottom.max(desired_y + rect.height);
            line_margin_bottom = line_margin_bottom.max(margin_bottom);
            previous = Some((desired_x + rect.width, margin_right));
        } else if matches!(child.layout_box, ui_layout::LayoutBox::BlockBox(_)) {
            previous = None;
            line_y = None;
            if !child.style.position.kind.is_out_of_flow()
                && let Some(model) = child.layout_box.iter().next()
            {
                preceding_block_bottom = Some((
                    model.border_box.bottom(),
                    fixed_nonnegative_px(&child.style.spacing.margin_bottom),
                ));
            }
        }
    }

    expand_auto_inline_width_to_children(node);
    expand_auto_flex_item_widths(node);
    correct_horizontal_flex_spacing(node);
    expand_auto_flex_width_to_children(node);
    expand_auto_flex_height_to_children(node);
    enforce_fixed_layout_height(node);
    correct_single_row_grid_alignment(node);
    correct_single_row_grid_inline_alignment(node);
    correct_vertical_block_spacing(node);
    expand_auto_flow_height_to_children(node);
}

fn expand_auto_inline_width_to_children(node: &mut LayoutNode) {
    if node.style.display.outer != OuterDisplay::Inline
        || node.style.size.width != LengthOrAuto::Auto
    {
        return;
    }
    let required_width = required_children_margin_box_width(node);
    grow_auto_layout_width(node, required_width);
}

fn correct_horizontal_flex_spacing(node: &mut LayoutNode) {
    if node.style.display.inner != InnerDisplay::Flex
        || node.style.flex_direction != FlexDirection::Row
    {
        return;
    }
    let column_gap = fixed_nonnegative_px(&node.style.column_gap);
    let mut previous_right: Option<f32> = None;
    for child in &mut node.children {
        let LayoutChild::Node(child) = child else {
            continue;
        };
        let Some(model) = child.layout_box.iter().next() else {
            continue;
        };
        let margin_left = fixed_nonnegative_px(&child.style.spacing.margin_left);
        let margin_right = fixed_nonnegative_px(&child.style.spacing.margin_right);
        let desired_x = previous_right
            .map(|right| right + column_gap + margin_left)
            .unwrap_or(model.border_box.x + margin_left);
        if model.border_box.x < desired_x {
            shift_layout_box_x(&mut child.layout_box, desired_x - model.border_box.x);
        }
        previous_right = Some(
            model
                .border_box
                .right()
                .max(desired_x + model.border_box.width)
                + margin_right,
        );
    }
}

fn expand_auto_flex_item_widths(node: &mut LayoutNode) {
    if node.style.display.inner != InnerDisplay::Flex
        || node.style.flex_direction != FlexDirection::Row
    {
        return;
    }
    for child in &mut node.children {
        let LayoutChild::Node(child) = child else {
            continue;
        };
        if child.style.size.width != LengthOrAuto::Auto {
            continue;
        }
        let required_width = required_children_margin_box_width(child);
        grow_auto_layout_width(child, required_width);
    }
}

fn expand_auto_flex_width_to_children(node: &mut LayoutNode) {
    if node.style.display.inner != InnerDisplay::Flex || node.style.size.width != LengthOrAuto::Auto
    {
        return;
    }
    let required_width = required_children_margin_box_width(node);
    grow_auto_layout_width(node, required_width);
}

fn expand_auto_flex_height_to_children(node: &mut LayoutNode) {
    if node.style.display.inner != InnerDisplay::Flex
        || node.style.size.height != LengthOrAuto::Auto
    {
        return;
    }
    let required_height = node
        .children
        .iter()
        .filter_map(LayoutChild::node)
        .filter_map(|child| {
            child.layout_box.iter().next().map(|model| {
                model.border_box.bottom() + fixed_nonnegative_px(&child.style.spacing.margin_bottom)
            })
        })
        .fold(0.0, f32::max);
    grow_auto_layout_height(node, required_height);
}

fn required_children_margin_box_width(node: &LayoutNode) -> f32 {
    node.children
        .iter()
        .filter_map(LayoutChild::node)
        .filter_map(|child| {
            child.layout_box.iter().next().map(|model| {
                model.border_box.right() + fixed_nonnegative_px(&child.style.spacing.margin_right)
            })
        })
        .fold(0.0, f32::max)
}

fn grow_auto_layout_width(node: &mut LayoutNode, required_width: f32) {
    let Some(model) = node.layout_box.iter().next() else {
        return;
    };
    let extra = required_width - model.content_box.width;
    if extra <= 0.0 {
        return;
    }

    match &mut node.layout_box {
        ui_layout::LayoutBox::BlockBox(model) => {
            model.content_box.width += extra;
            model.padding_box.width += extra;
            model.border_box.width += extra;
            model.children_box.width = model.children_box.width.max(required_width);
        }
        ui_layout::LayoutBox::InlineBox(inline) => {
            inline.box_model.content_box.width += extra;
            inline.box_model.padding_box.width += extra;
            inline.box_model.border_box.width += extra;
            inline.box_model.children_box.width =
                inline.box_model.children_box.width.max(required_width);
            if let Some(last) = inline.line_spans.last_mut() {
                last.x_range.end += extra;
            }
        }
        ui_layout::LayoutBox::None => {}
    }
}

fn grow_auto_layout_height(node: &mut LayoutNode, required_height: f32) {
    let Some(model) = node.layout_box.iter().next() else {
        return;
    };
    let extra = required_height - model.content_box.height;
    if extra <= 0.0 {
        return;
    }

    match &mut node.layout_box {
        ui_layout::LayoutBox::BlockBox(model) => {
            model.content_box.height += extra;
            model.padding_box.height += extra;
            model.border_box.height += extra;
            model.children_box.height = model.children_box.height.max(required_height);
        }
        ui_layout::LayoutBox::InlineBox(inline) => {
            inline.box_model.content_box.height += extra;
            inline.box_model.padding_box.height += extra;
            inline.box_model.border_box.height += extra;
            inline.box_model.children_box.height =
                inline.box_model.children_box.height.max(required_height);
        }
        ui_layout::LayoutBox::None => {}
    }
}

fn enforce_fixed_layout_height(node: &mut LayoutNode) {
    let declared = match (&node.style.size.height, &node.style.size.min_height) {
        (LengthOrAuto::Length(Length::Px(height)), LengthOrAuto::Length(Length::Px(minimum))) => {
            Some(height.max(*minimum))
        }
        (LengthOrAuto::Length(Length::Px(height)), _) => Some(*height),
        (_, LengthOrAuto::Length(Length::Px(minimum))) => Some(*minimum),
        _ => None,
    };
    let Some(declared) = declared else {
        return;
    };
    let Some(model) = node.layout_box.iter().next() else {
        return;
    };
    let current = if node.style.box_sizing == BoxSizing::BorderBox {
        model.border_box.height
    } else {
        model.content_box.height
    };
    let extra = declared - current;
    if extra <= 0.0 {
        return;
    }

    grow_layout_height(&mut node.layout_box, extra);
    if node.style.position.kind.is_out_of_flow()
        && node.style.position.top == LengthOrAuto::Auto
        && node.style.position.bottom != LengthOrAuto::Auto
    {
        shift_layout_box_y(&mut node.layout_box, -extra);
    }
}

fn correct_single_row_grid_alignment(node: &mut LayoutNode) {
    if node.style.display.inner != InnerDisplay::Grid {
        return;
    }
    let Some(content_box) = node.layout_box.iter().next().map(|model| model.content_box) else {
        return;
    };

    let row_origins: Vec<f32> = node
        .children
        .iter()
        .filter_map(LayoutChild::node)
        .filter(|child| {
            child.style.display.outer != OuterDisplay::None
                && !child.style.position.kind.is_out_of_flow()
        })
        .filter_map(|child| {
            child
                .layout_box
                .iter()
                .next()
                .map(|model| model.border_box.y)
        })
        .collect();
    let Some(first_row) = row_origins.first().copied() else {
        return;
    };
    if row_origins
        .iter()
        .any(|origin| (origin - first_row).abs() > 0.5)
    {
        return;
    }

    for child in &mut node.children {
        let LayoutChild::Node(child) = child else {
            continue;
        };
        if child.style.display.outer == OuterDisplay::None
            || child.style.position.kind.is_out_of_flow()
        {
            continue;
        }
        let alignment = child
            .style
            .item_style
            .align_self
            .unwrap_or(node.style.align_items);
        if matches!(alignment, AlignItems::Start | AlignItems::Stretch) {
            continue;
        }
        let Some(model) = child.layout_box.iter().next() else {
            continue;
        };
        let margin_top = fixed_nonnegative_px(&child.style.spacing.margin_top);
        let margin_bottom = fixed_nonnegative_px(&child.style.spacing.margin_bottom);
        let free_space =
            (content_box.height - model.border_box.height - margin_top - margin_bottom).max(0.0);
        let offset = match alignment {
            AlignItems::Center => free_space / 2.0,
            AlignItems::End => free_space,
            AlignItems::Start | AlignItems::Stretch => 0.0,
        };
        let desired_y = margin_top + offset;
        let shift_y = desired_y - model.border_box.y;
        shift_layout_box_y(&mut child.layout_box, shift_y);
    }
}

fn correct_single_row_grid_inline_alignment(node: &mut LayoutNode) {
    if node.style.display.inner != InnerDisplay::Grid {
        return;
    }
    let Some(content_box) = node.layout_box.iter().next().map(|model| model.content_box) else {
        return;
    };
    let column_gap = fixed_nonnegative_px(&node.style.column_gap);
    let item_indices: Vec<usize> = node
        .children
        .iter()
        .enumerate()
        .filter_map(|(index, child)| {
            child.node().and_then(|child| {
                (child.style.display.outer != OuterDisplay::None
                    && !child.style.position.kind.is_out_of_flow())
                .then_some(index)
            })
        })
        .collect();
    if item_indices.len() > node.style.grid_template_columns.len() {
        return;
    }

    for (position, index) in item_indices.iter().copied().enumerate() {
        let next_x = item_indices
            .get(position + 1)
            .and_then(|next| node.children[*next].node())
            .and_then(|next| next.layout_box.iter().next())
            .map(|model| model.border_box.x - column_gap)
            .unwrap_or(content_box.width);
        let child = node.children[index].node_mut().expect("grid item");
        let left_auto = child.style.spacing.margin_left == LengthOrAuto::Auto;
        let right_auto = child.style.spacing.margin_right == LengthOrAuto::Auto;
        if !left_auto && !right_auto {
            continue;
        }
        let Some(model) = child.layout_box.iter().next() else {
            continue;
        };
        let track_start = model.border_box.x;
        let free_space = (next_x - track_start - model.border_box.width).max(0.0);
        let offset = match (left_auto, right_auto) {
            (true, true) => free_space / 2.0,
            (true, false) => free_space,
            _ => 0.0,
        };
        shift_layout_box_x(&mut child.layout_box, offset);
    }
}

fn correct_vertical_block_spacing(node: &mut LayoutNode) {
    if node.style.display.inner != InnerDisplay::Flow {
        return;
    }
    let mut previous_bottom: Option<f32> = None;
    for child in &mut node.children {
        let LayoutChild::Node(child) = child else {
            continue;
        };
        if child.style.position.kind.is_out_of_flow()
            || child.style.display.outer != OuterDisplay::Block
        {
            continue;
        }
        let Some(model) = child.layout_box.iter().next() else {
            continue;
        };
        let margin_top = fixed_nonnegative_px(&child.style.spacing.margin_top);
        let margin_bottom = fixed_nonnegative_px(&child.style.spacing.margin_bottom);
        let desired_y = previous_bottom
            .map(|bottom| bottom + margin_top)
            .unwrap_or(model.border_box.y);
        if model.border_box.y < desired_y {
            shift_layout_box_y(&mut child.layout_box, desired_y - model.border_box.y);
        }
        previous_bottom = Some(
            model
                .border_box
                .bottom()
                .max(desired_y + model.border_box.height)
                + margin_bottom,
        );
    }
}

fn expand_auto_flow_height_to_children(node: &mut LayoutNode) {
    if node.style.display.inner != InnerDisplay::Flow
        || node.style.size.height != LengthOrAuto::Auto
    {
        return;
    }
    let required_height = node
        .children
        .iter()
        .filter_map(LayoutChild::node)
        .filter(|child| !child.style.position.kind.is_out_of_flow())
        .filter_map(|child| {
            child.layout_box.iter().next().map(|model| {
                model.border_box.bottom() + fixed_nonnegative_px(&child.style.spacing.margin_bottom)
            })
        })
        .fold(0.0, f32::max);
    grow_auto_layout_height(node, required_height);
}

fn grow_layout_height(layout_box: &mut ui_layout::LayoutBox, extra: f32) {
    match layout_box {
        ui_layout::LayoutBox::BlockBox(model) => {
            model.content_box.height += extra;
            model.padding_box.height += extra;
            model.border_box.height += extra;
            model.children_box.height += extra;
        }
        ui_layout::LayoutBox::InlineBox(inline) => {
            inline.box_model.content_box.height += extra;
            inline.box_model.padding_box.height += extra;
            inline.box_model.border_box.height += extra;
            inline.box_model.children_box.height += extra;
        }
        ui_layout::LayoutBox::None => {}
    }
}

fn shift_layout_box_x(layout_box: &mut ui_layout::LayoutBox, shift_x: f32) {
    let shift_model = |model: &mut ui_layout::BoxModel| {
        model.border_box.x += shift_x;
        model.padding_box.x += shift_x;
        model.content_box.x += shift_x;
        model.children_box.x += shift_x;
    };
    match layout_box {
        ui_layout::LayoutBox::None => {}
        ui_layout::LayoutBox::BlockBox(model) => shift_model(model),
        ui_layout::LayoutBox::InlineBox(inline) => {
            shift_model(&mut inline.box_model);
            for span in &mut inline.line_spans {
                span.line_pos.0 += shift_x;
            }
        }
    }
}

fn shift_layout_box_y(layout_box: &mut ui_layout::LayoutBox, shift_y: f32) {
    let shift_model = |model: &mut ui_layout::BoxModel| {
        model.border_box.y += shift_y;
        model.padding_box.y += shift_y;
        model.content_box.y += shift_y;
        model.children_box.y += shift_y;
    };
    match layout_box {
        ui_layout::LayoutBox::None => {}
        ui_layout::LayoutBox::BlockBox(model) => shift_model(model),
        ui_layout::LayoutBox::InlineBox(inline) => {
            shift_model(&mut inline.box_model);
            for span in &mut inline.line_spans {
                span.line_pos.1 += shift_y;
            }
        }
    }
}

fn fixed_nonnegative_px(value: &LengthOrAuto) -> f32 {
    match value {
        LengthOrAuto::Length(Length::Px(value)) => value.max(0.0),
        _ => 0.0,
    }
}

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
    let _t = std::time::Instant::now();
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

    let layouter = TextFlowLayouter::new(text.clone(), text_flow_style, clusters);
    let kind = NodeKind::Text {
        text,
        style: text_style,
        flow_style: text_flow_style,
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
    rule_set: &RuleSet,
    chain: &ElementChain,
) -> (Properties, Properties) {
    let mut properties = HashMap::new();
    let mut custom_properties = HashMap::new();

    let element = match chain.first() {
        Some(el) => el,
        None => return (properties, custom_properties),
    };

    let candidates_iter = rule_set.query_candidates(element);

    for decl in candidates_iter {
        let matches_sel = decl.selector.matches(chain);
        if !matches_sel {
            continue;
        }

        let target = if decl.name.starts_with("--") {
            &mut custom_properties
        } else {
            &mut properties
        };

        let should_replace = match target.get(&decl.name) {
            Some(current) => decl.outranks(current),
            None => true,
        };

        if should_replace {
            target.insert(decl.name.clone(), decl.clone());
        }
    }

    (properties, custom_properties)
}

fn apply_attribute_dimensions(
    html_node: &HtmlNodeType,
    style: &mut Style,
    container_style: &mut ContainerStyle,
    text_style: &mut TextStyle,
    text_flow_style: &mut TextFlowStyle,
    overflow: &mut Overflow,
    color_scheme: ColorScheme,
) {
    fn apply_attribute_size(
        attr: &str,
        html_node: &HtmlNodeType,
        style: &mut Style,
        container_style: &mut ContainerStyle,
        text_style: &mut TextStyle,
        text_flow_style: &mut TextFlowStyle,
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
                text_flow_style,
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
        text_flow_style,
        overflow,
        color_scheme,
    );

    apply_attribute_size(
        "height",
        html_node,
        style,
        container_style,
        text_style,
        text_flow_style,
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
    text_flow_style: &TextFlowStyle,
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
        CssValue::Length(_, _) => Some((
            1.0,
            1.0,
            resolve_css_len_auto("flex", value, text_flow_style)?,
        )),
        CssValue::List(values) => match values.as_slice() {
            [CssValue::Number(grow), CssValue::Number(shrink)]
                if *grow >= 0.0 && *shrink >= 0.0 =>
            {
                Some((*grow, *shrink, zero_basis))
            }
            [CssValue::Number(grow), basis] if *grow >= 0.0 => Some((
                *grow,
                1.0,
                resolve_css_len_auto("flex", basis, text_flow_style)?,
            )),
            [CssValue::Number(grow), CssValue::Number(shrink), basis]
                if *grow >= 0.0 && *shrink >= 0.0 =>
            {
                Some((
                    *grow,
                    *shrink,
                    resolve_css_len_auto("flex", basis, text_flow_style)?,
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
    text_flow_style: &mut TextFlowStyle,
    overflow: &mut Overflow,
    color_scheme: ColorScheme,
) -> Option<()> {
    fn expand_box<T: Clone, F>(
        name: &str,
        value: &CssValue,
        text_flow_style: &TextFlowStyle,
        resolve: &impl Fn(&str, &CssValue, &TextFlowStyle) -> Option<T>,
        mut set: F,
    ) -> Option<()>
    where
        F: FnMut(T, T, T, T),
    {
        let resolve = |v: &CssValue| -> Option<T> { resolve(name, v, text_flow_style) };

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
        text_flow_style: &TextFlowStyle,
    ) -> Option<[Length; 4]> {
        let vals: Vec<Length> = values
            .iter()
            .map(|v| resolve_css_len(name, v, text_flow_style))
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
        text_flow_style: &TextFlowStyle,
    ) -> Option<(CornerRadius, CornerRadius, CornerRadius, CornerRadius)> {
        let (horiz, vert) = split_radius_lists(value);
        let h = expand_radius_axis(name, &horiz, text_flow_style)?;
        let v = if vert.is_empty() {
            h.clone()
        } else {
            expand_radius_axis(name, &vert, text_flow_style)?
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
        text_flow_style: &TextFlowStyle,
    ) -> Option<CornerRadius> {
        let (horiz, vert) = split_radius_lists(value);
        let x = expand_radius_axis(name, &horiz, text_flow_style)?;
        let y = if vert.is_empty() {
            x.clone()
        } else {
            expand_radius_axis(name, &vert, text_flow_style)?
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
        text_flow_style: &TextFlowStyle,
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
                && let Some(l) = resolve_css_len(name, token, text_flow_style)
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

        ("z-index", CssValue::Number(value))
            if value.is_finite() && value.fract().abs() < f32::EPSILON =>
        {
            container_style.z_index = Some(*value as i32);
        }

        ("z-index", CssValue::Keyword(value))
            if matches!(
                value.to_ascii_lowercase().as_str(),
                "auto" | "initial" | "unset"
            ) =>
        {
            container_style.z_index = None;
        }

        ("visibility", CssValue::Keyword(value)) => {
            container_style.visibility = match value.to_ascii_lowercase().as_str() {
                "visible" => Visibility::Visible,
                "hidden" => Visibility::Hidden,
                "collapse" => Visibility::Collapse,
                _ => return None,
            };
        }

        ("float", CssValue::Keyword(value)) => {
            container_style.css_float = match value.to_ascii_lowercase().as_str() {
                "left" => CssFloat::Left,
                "right" => CssFloat::Right,
                "none" | "initial" | "unset" => CssFloat::None,
                _ => return None,
            };
        }

        /* ======================
         * Color / Text
         * ====================== */
        ("background-color", _) => {
            let color = match value {
                CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("inherit") => text_style.color,
                CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("currentColor") => {
                    text_style.color
                }
                CssValue::Keyword(kw)
                    if kw.eq_ignore_ascii_case("initial") || kw.eq_ignore_ascii_case("unset") =>
                {
                    Color(0, 0, 0, 0)
                }
                _ => resolve_css_color(name, value, color_scheme)?,
            };
            match &mut container_style.background {
                Background::Image {
                    color: image_color, ..
                } => *image_color = color,
                _ => container_style.background = Background::Color(color),
            };
        }

        ("background", _) => {
            container_style.background =
                parse_background_shorthand(name, value, text_style, text_flow_style, color_scheme)?;
            apply_background_shorthand_geometry(value, text_flow_style.font_size, container_style);
        }

        ("background-image", _) => {
            let existing_color = match &container_style.background {
                Background::Color(color) | Background::Image { color, .. } => *color,
                Background::Gradient(_) => Color(0, 0, 0, 0),
            };
            let parsed =
                parse_background_shorthand(name, value, text_style, text_flow_style, color_scheme)?;
            container_style.background = match parsed {
                Background::Image { source, image, .. } => Background::Image {
                    source,
                    image,
                    color: existing_color,
                },
                Background::Color(Color(_, _, _, 0)) => Background::Color(existing_color),
                other => other,
            };
        }

        ("background-repeat", _) => {
            container_style.background_repeat = parse_background_repeat(value)?;
        }

        ("background-size", _) => {
            container_style.background_size =
                parse_background_size(value, text_flow_style.font_size)?;
        }

        ("background-position", _) => {
            container_style.background_position =
                parse_background_position(value, text_flow_style.font_size)?;
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

        // `color-scheme` is resolved separately above.
        ("color-scheme", _) => {}

        ("font-size", _) => {
            let len = resolve_css_len(name, value, text_flow_style)?;
            let px = resolve_font_size_px(&len, text_flow_style.font_size)?;
            text_flow_style.font_size = px;
        }

        ("line-height", CssValue::Number(factor)) => {
            text_flow_style.line_height = LineHeight::Number(*factor);
        }
        ("line-height", CssValue::Keyword(v)) if v == "normal" => {
            text_flow_style.line_height = LineHeight::Normal;
        }
        ("line-height", _) => {
            let len = resolve_css_len(name, value, text_flow_style)?;
            text_flow_style.line_height =
                LineHeight::Px(length_to_px(&len, text_flow_style.font_size));
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
                    text_flow_style.vertical_align = VerticalAlign::Sub;
                }
                "super" | "sup" => {
                    text_flow_style.vertical_align = VerticalAlign::Super;
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
            text_flow_style.text_align = TextAlign::Left;
        }
        ("text-align", CssValue::Keyword(v)) if v == "center" => {
            text_flow_style.text_align = TextAlign::Center;
        }
        ("text-align", CssValue::Keyword(v)) if v == "right" => {
            text_flow_style.text_align = TextAlign::Right;
        }

        ("white-space", CssValue::Keyword(v)) => {
            text_flow_style.white_space = match v.as_str() {
                "normal" => WhiteSpace::Normal,
                "nowrap" => WhiteSpace::Nowrap,
                "pre" => WhiteSpace::Pre,
                "pre-wrap" => WhiteSpace::PreWrap,
                "pre-line" => WhiteSpace::PreLine,
                "break-spaces" => WhiteSpace::BreakSpaces,
                _ => text_flow_style.white_space,
            };
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
                text_flow_style,
                &|_, cv, tfs| match cv {
                    CssValue::Keyword(s) if s == "auto" => Some(ui_layout::LengthOrAuto::Auto),
                    _ => resolve_css_len(name, cv, tfs).map(ui_layout::LengthOrAuto::Length),
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
            style.spacing.margin_top = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("margin-right", _) => {
            style.spacing.margin_right = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("margin-bottom", _) => {
            style.spacing.margin_bottom = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("margin-left", _) => {
            style.spacing.margin_left = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("margin-inline", _) => {
            let values = one_or_two_values(value)?;
            style.spacing.margin_left = resolve_css_len_auto(name, values.0, text_flow_style)?;
            style.spacing.margin_right = resolve_css_len_auto(name, values.1, text_flow_style)?;
        }
        ("margin-inline-start", _) => {
            style.spacing.margin_left = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("margin-inline-end", _) => {
            style.spacing.margin_right = resolve_css_len_auto(name, value, text_flow_style)?;
        }

        ("border", v) => {
            let (maybe_width, maybe_style, maybe_color) = if let CssValue::Keyword(k) = v
                && (k.eq_ignore_ascii_case("inset") || k.eq_ignore_ascii_case("initial"))
            {
                (Some(Length::Px(0.0)), None, None)
            } else {
                parse_border_shorthand(name, v, text_flow_style, color_scheme)?
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
                parse_border_shorthand(name, value, text_flow_style, color_scheme)?;
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
                parse_border_shorthand(name, value, text_flow_style, color_scheme)?;
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
                parse_border_shorthand(name, value, text_flow_style, color_scheme)?;
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
                parse_border_shorthand(name, value, text_flow_style, color_scheme)?;
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
            let (tl, tr, br, bl) = parse_border_radius_shorthand(name, v, text_flow_style)?;
            container_style.border_radius = BorderRadius {
                top_left: tl,
                top_right: tr,
                bottom_right: br,
                bottom_left: bl,
            };
        }
        ("border-top-left-radius", v) => {
            container_style.border_radius.top_left = parse_corner_radius(name, v, text_flow_style)?;
        }
        ("border-top-right-radius", v) => {
            container_style.border_radius.top_right =
                parse_corner_radius(name, v, text_flow_style)?;
        }
        ("border-bottom-right-radius", v) => {
            container_style.border_radius.bottom_right =
                parse_corner_radius(name, v, text_flow_style)?;
        }
        ("border-bottom-left-radius", v) => {
            container_style.border_radius.bottom_left =
                parse_corner_radius(name, v, text_flow_style)?;
        }

        ("padding", v) => {
            expand_box(
                name,
                v,
                text_flow_style,
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
            style.spacing.padding_top = resolve_css_len(name, value, text_flow_style)?;
        }
        ("padding-right", _) => {
            style.spacing.padding_right = resolve_css_len(name, value, text_flow_style)?;
        }
        ("padding-bottom", _) => {
            style.spacing.padding_bottom = resolve_css_len(name, value, text_flow_style)?;
        }
        ("padding-left", _) => {
            style.spacing.padding_left = resolve_css_len(name, value, text_flow_style)?;
        }
        ("padding-inline", _) => {
            let values = one_or_two_values(value)?;
            style.spacing.padding_left = resolve_css_len(name, values.0, text_flow_style)?;
            style.spacing.padding_right = resolve_css_len(name, values.1, text_flow_style)?;
        }
        ("padding-inline-start", _) => {
            style.spacing.padding_left = resolve_css_len(name, value, text_flow_style)?;
        }
        ("padding-inline-end", _) => {
            style.spacing.padding_right = resolve_css_len(name, value, text_flow_style)?;
        }

        /* ======================
         * Size
         * ====================== */
        ("width", _) => {
            style.size.width = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("height", _) => {
            style.size.height = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("min-width", _) => {
            style.size.min_width = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("min-height", _) => {
            style.size.min_height = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("max-width", _) => {
            style.size.max_width = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("max-height", _) => {
            style.size.max_height = resolve_css_len_auto(name, value, text_flow_style)?;
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
                    "sticky" => Position::Sticky,
                    _ => return None,
                },
                _ => return None,
            };
        }
        ("top", _) => {
            style.position.top = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("right", _) => {
            style.position.right = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("bottom", _) => {
            style.position.bottom = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("left", _) => {
            style.position.left = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("inset", v) => {
            expand_box(
                name,
                v,
                text_flow_style,
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
                let gap = resolve_css_len_auto(name, l.next()?, text_flow_style)?;
                style.row_gap = gap;
                let gap = resolve_css_len_auto(name, l.next()?, text_flow_style)?;
                style.column_gap = gap;
            }
            CssValue::Length(_, _) => {
                let gap = resolve_css_len_auto(name, value, text_flow_style)?;
                style.row_gap = gap.clone();
                style.column_gap = gap;
            }
            _ => {}
        },

        ("flex", _) => {
            let (grow, shrink, basis) = resolve_flex_shorthand(value, text_flow_style)?;
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
            style.item_style.flex_basis = resolve_css_len_auto(name, value, text_flow_style)?;
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

        ("justify-self", CssValue::Keyword(v)) => match v.as_str() {
            "center" => {
                style.spacing.margin_left = LengthOrAuto::Auto;
                style.spacing.margin_right = LengthOrAuto::Auto;
            }
            "flex-end" | "end" => {
                style.spacing.margin_left = LengthOrAuto::Auto;
                style.spacing.margin_right = LengthOrAuto::Length(Length::Px(0.0));
            }
            "flex-start" | "start" => {
                style.spacing.margin_left = LengthOrAuto::Length(Length::Px(0.0));
                style.spacing.margin_right = LengthOrAuto::Auto;
            }
            "auto" | "normal" | "stretch" => {}
            _ => return None,
        },

        ("column-gap", _) => {
            style.column_gap = resolve_css_len_auto(name, value, text_flow_style)?;
        }

        ("row-gap", _) => {
            style.row_gap = resolve_css_len_auto(name, value, text_flow_style)?;
        }

        /* ======================
         * Grid
         * ====================== */
        ("grid-template-columns", _) => {
            style.grid_template_columns = parse_grid_tracks(name, value, text_flow_style)?;
        }

        ("grid-template-rows", _) => {
            style.grid_template_rows = parse_grid_tracks(name, value, text_flow_style)?;
        }

        ("grid-template-areas", _) => {
            style.grid_template_areas = parse_grid_template_areas(value)?;
        }

        ("grid-area", CssValue::Keyword(area)) => {
            style.grid_area = Some(area.to_string());
        }

        ("grid-column", _) => {
            style.grid_column = parse_grid_placement(value)?;
        }

        ("grid-row", _) => {
            style.grid_row = parse_grid_placement(value)?;
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
    text_flow_style: &TextFlowStyle,
    color_scheme: ColorScheme,
) -> Option<Background> {
    let items: Vec<&CssValue> = match value {
        CssValue::List(values) => values.iter().collect(),
        _ => vec![value],
    };

    let mut maybe_color: Option<Color> = None;
    let mut maybe_gradient: Option<Gradient> = None;
    let mut maybe_image: Option<String> = None;

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
            if kw.eq_ignore_ascii_case("none")
                || kw.eq_ignore_ascii_case("initial")
                || kw.eq_ignore_ascii_case("unset")
            {
                maybe_color = Some(Color(0, 0, 0, 0));
                continue;
            }
        }

        if let CssValue::Number(0.0) = v {
            maybe_color = Some(Color(0, 0, 0, 0));
            continue;
        }

        // gradient
        if let CssValue::Function(fn_name, args) = v
            && matches!(
                fn_name.as_str(),
                "linear-gradient"
                    | "repeating-linear-gradient"
                    | "radial-gradient"
                    | "repeating-radial-gradient"
                    | "conic-gradient"
                    | "repeating-conic-gradient"
            )
        {
            maybe_gradient = Some(parse_gradient(
                fn_name,
                args,
                text_style,
                text_flow_style,
                color_scheme,
            )?);
            continue;
        }

        if let CssValue::Function(fn_name, args) = v
            && fn_name.eq_ignore_ascii_case("url")
        {
            maybe_image = args.iter().find_map(|value| match value {
                CssValue::String(source) => Some(source.clone()),
                CssValue::Keyword(source) => Some(source.to_string()),
                _ => None,
            });
            continue;
        }

        // Only color-shaped values should reach the color resolver. A
        // background shorthand also contains image, repeat, attachment,
        // position, size and box tokens, none of which are color errors.
        let is_color_value = matches!(v, CssValue::Color(_))
            || matches!(
                v,
                CssValue::Function(function, _)
                    if matches!(
                        function.as_str(),
                        "rgb" | "rgba" | "hsl" | "hsla" | "light-dark" | "color-mix"
                    )
            )
            || matches!(
                v,
                CssValue::Keyword(keyword)
                    if !matches!(
                        keyword.to_ascii_lowercase().as_str(),
                        "none"
                            | "repeat"
                            | "repeat-x"
                            | "repeat-y"
                            | "no-repeat"
                            | "space"
                            | "round"
                            | "scroll"
                            | "fixed"
                            | "local"
                            | "left"
                            | "right"
                            | "top"
                            | "bottom"
                            | "center"
                            | "cover"
                            | "contain"
                            | "auto"
                            | "border-box"
                            | "padding-box"
                            | "content-box"
                            | "/"
                    )
            );
        if is_color_value && let Some(c) = resolve_css_color(name, v, color_scheme) {
            maybe_color = Some(c);
        }
    }

    if let Some(g) = maybe_gradient {
        return Some(Background::Gradient(g));
    }
    if let Some(source) = maybe_image {
        return Some(Background::Image {
            source,
            image: None,
            color: maybe_color.unwrap_or(Color(0, 0, 0, 0)),
        });
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
    text_flow_style: &TextFlowStyle,
    color_scheme: ColorScheme,
) -> Option<Gradient> {
    match fn_name {
        "linear-gradient" | "repeating-linear-gradient" => {
            parse_linear_gradient(args, text_style, text_flow_style, color_scheme)
        }
        "radial-gradient" | "repeating-radial-gradient" => {
            parse_radial_gradient(args, text_style, text_flow_style, color_scheme)
        }
        "conic-gradient" | "repeating-conic-gradient" => {
            parse_conic_gradient(args, text_style, text_flow_style, color_scheme)
        }
        _ => None,
    }
}

fn parse_linear_gradient(
    args: &[CssValue],
    text_style: &TextStyle,
    text_flow_style: &TextFlowStyle,
    color_scheme: ColorScheme,
) -> Option<Gradient> {
    if args.is_empty() {
        return None;
    }

    let (skip, angle) = parse_linear_direction(args);
    let angle = angle.unwrap_or(180.0);
    let stops = parse_color_stops(&args[skip..], text_style, text_flow_style, color_scheme)?;

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
    text_style: &TextStyle,
    text_flow_style: &TextFlowStyle,
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

    let stops = parse_color_stops(&args[idx..], text_style, text_flow_style, color_scheme)?;
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

fn parse_color_stops(
    args: &[CssValue],
    text_style: &TextStyle,
    text_flow_style: &TextFlowStyle,
    color_scheme: ColorScheme,
) -> Option<Vec<ColorStop>> {
    let mut stops = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let color = match &args[i] {
            CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("currentColor") => text_style.color,
            _ => resolve_css_color("gradient", &args[i], color_scheme)?,
        };
        i += 1;

        // Consume up to two position lengths (a double-position stop is
        // equivalent to two stops at the same color).
        let mut positions: Vec<f32> = Vec::new();
        while positions.len() < 2 && i < args.len() {
            if let CssValue::Length(_v, Unit::Px) = &args[i] {
                // Absolute lengths are not resolvable here; treat as auto.
                i += 1;
                continue;
            }
            match resolve_gradient_position(&args[i], text_flow_style) {
                Some(position) => {
                    positions.push(position.clamp(0.0, 1.0));
                    i += 1;
                }
                None => break,
            }
        }

        if positions.is_empty() {
            stops.push(ColorStop {
                color,
                position: None,
            });
        } else {
            for p in positions {
                stops.push(ColorStop {
                    color,
                    position: Some(p),
                });
            }
        }
    }

    Some(stops)
}

fn parse_conic_gradient(
    args: &[CssValue],
    text_style: &TextStyle,
    text_flow_style: &TextFlowStyle,
    color_scheme: ColorScheme,
) -> Option<Gradient> {
    if args.is_empty() {
        return None;
    }

    let mut angle = 0.0f32;
    let mut position = (0.5f32, 0.5f32);
    let mut idx = 0;

    // Optional `from <angle>`
    if idx + 1 < args.len()
        && matches!(&args[idx], CssValue::Keyword(k) if k.eq_ignore_ascii_case("from"))
        && let CssValue::Length(v, Unit::Deg) = &args[idx + 1]
    {
        angle = *v;
        idx += 2;
    }

    // Optional `at <position>`
    if idx + 1 < args.len()
        && matches!(&args[idx], CssValue::Keyword(k) if k.eq_ignore_ascii_case("at"))
    {
        idx += 1;
        if idx < args.len()
            && let CssValue::Keyword(k) = &args[idx]
        {
            match k.as_str() {
                "center" => position = (0.5, 0.5),
                "top" => position = (0.5, 0.0),
                "bottom" => position = (0.5, 1.0),
                "left" => position = (0.0, 0.5),
                "right" => position = (1.0, 0.5),
                _ => {}
            }
            idx += 1;
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

    let stops = parse_color_stops(&args[idx..], text_style, text_flow_style, color_scheme)?;
    if stops.is_empty() {
        return None;
    }
    Some(Gradient {
        kind: GradientKind::Conic { angle, position },
        stops,
    })
}

/// Resolve a gradient stop position into a normalized fraction of the
/// gradient length (100% == 1.0). `None` means the value is not resolvable
/// at this stage and should be treated as an auto position.
fn resolve_gradient_position(value: &CssValue, text_flow_style: &TextFlowStyle) -> Option<f32> {
    match value {
        CssValue::Length(v, Unit::Percent) => Some(*v / 100.0),
        CssValue::Length(v, Unit::Deg) => Some(*v / 360.0),
        CssValue::Number(0.0) => Some(0.0),
        CssValue::Function(fn_name, args) if fn_name == "calc" => {
            let value = CssValue::Function(fn_name.clone(), args.clone());
            let length = resolve_css_len("gradient", &value, text_flow_style)?;
            length_to_fraction(&length)
        }
        _ => None,
    }
}

/// Convert a resolved [`Length`] into a normalized gradient fraction. Percent
/// values are relative to the gradient length (100% == 1.0). Absolute lengths
/// (px/vw/vh) cannot be resolved without the gradient box and yield `None`.
fn length_to_fraction(length: &Length) -> Option<f32> {
    match length {
        Length::Percent(v) => Some(*v / 100.0),
        Length::Add(a, b) => Some(length_to_fraction(a)? + length_to_fraction(b)?),
        Length::Sub(a, b) => Some(length_to_fraction(a)? - length_to_fraction(b)?),
        Length::Mul(a, factor) => Some(length_to_fraction(a)? * factor),
        Length::Div(a, factor) => {
            if *factor == 0.0 {
                None
            } else {
                Some(length_to_fraction(a)? / factor)
            }
        }
        Length::Min(a, b) => Some(length_to_fraction(a)?.min(length_to_fraction(b)?)),
        Length::Max(a, b) => Some(length_to_fraction(a)?.max(length_to_fraction(b)?)),
        Length::Clamp { min, val, max } => {
            Some(length_to_fraction(val)?.clamp(length_to_fraction(min)?, length_to_fraction(max)?))
        }
        Length::Px(_) | Length::Vw(_) | Length::Vh(_) => None,
    }
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
    text_flow_style: &TextFlowStyle,
) -> Option<LengthOrAuto> {
    match &css_len {
        CssValue::Keyword(s) if s == "auto" => Some(LengthOrAuto::Auto),
        _ => resolve_css_len(name, css_len, text_flow_style).map(|l| l.into()),
    }
}

fn one_or_two_values(value: &CssValue) -> Option<(&CssValue, &CssValue)> {
    match value {
        CssValue::List(values) if values.len() == 2 => Some((&values[0], &values[1])),
        CssValue::List(values) if values.len() == 1 => Some((&values[0], &values[0])),
        CssValue::List(_) => None,
        value => Some((value, value)),
    }
}

fn resolve_font_size_px(length: &Length, inherited_size: f32) -> Option<f32> {
    match length {
        Length::Px(value) => Some(*value),
        Length::Percent(value) => Some(*value * inherited_size / 100.0),
        Length::Clamp { min, val, max } => {
            let minimum = resolve_font_size_px(min, inherited_size)?;
            let maximum = resolve_font_size_px(max, inherited_size)?;
            let preferred = resolve_font_size_px(val, inherited_size).unwrap_or(maximum);
            Some(preferred.clamp(minimum, maximum))
        }
        Length::Min(left, right) => Some(
            resolve_font_size_px(left, inherited_size)?
                .min(resolve_font_size_px(right, inherited_size)?),
        ),
        Length::Max(left, right) => Some(
            resolve_font_size_px(left, inherited_size)?
                .max(resolve_font_size_px(right, inherited_size)?),
        ),
        Length::Add(left, right) => Some(
            resolve_font_size_px(left, inherited_size)?
                + resolve_font_size_px(right, inherited_size)?,
        ),
        Length::Sub(left, right) => Some(
            resolve_font_size_px(left, inherited_size)?
                - resolve_font_size_px(right, inherited_size)?,
        ),
        Length::Mul(value, factor) => Some(resolve_font_size_px(value, inherited_size)? * factor),
        Length::Div(value, factor) if *factor != 0.0 => {
            Some(resolve_font_size_px(value, inherited_size)? / factor)
        }
        Length::Vw(_) | Length::Vh(_) | Length::Div(_, _) => None,
    }
}

/// calc() の評価結果。型 (number / length) を保持する。
#[derive(Debug, Clone, PartialEq)]
enum CalcValue {
    Number(f32),
    Length(Length),
}

fn resolve_length(
    name: &str,
    v: f32,
    unit: &Unit,
    text_flow_style: &TextFlowStyle,
) -> Option<Length> {
    match unit {
        Unit::Em => Some(Length::Px(text_flow_style.font_size * v)),
        Unit::Rem => Some(Length::Px(16.0 * v)), // Stub
        Unit::Percent => Some(Length::Percent(v)),
        Unit::Px => Some(Length::Px(v)),
        Unit::Vw => Some(Length::Vw(v)),
        Unit::Vh => Some(Length::Vh(v)),
        Unit::Deg => {
            log::error!(target: "Layouter", "Unexpected deg unit for `{}` (expected length)", name);
            None
        }
        Unit::Fr => {
            log::error!(target: "Layouter", "Unexpected fr unit for `{}` (expected length)", name);
            None
        }
    }
}

fn calc_combine(name: &str, op: &CssValue, left: CalcValue, right: CalcValue) -> Option<CalcValue> {
    match op {
        CssValue::Keyword(o) if o == "+" => match (left, right) {
            (CalcValue::Number(x), CalcValue::Number(y)) => Some(CalcValue::Number(x + y)),
            (CalcValue::Length(x), CalcValue::Length(y)) => {
                Some(CalcValue::Length(Length::Add(Box::new(x), Box::new(y))))
            }
            _ => {
                log::error!(
                    target: "Layouter",
                    "Cannot add number and length in calc() for `{}`",
                    name
                );
                None
            }
        },
        CssValue::Keyword(o) if o == "-" => match (left, right) {
            (CalcValue::Number(x), CalcValue::Number(y)) => Some(CalcValue::Number(x - y)),
            (CalcValue::Length(x), CalcValue::Length(y)) => {
                Some(CalcValue::Length(Length::Sub(Box::new(x), Box::new(y))))
            }
            _ => {
                log::error!(
                    target: "Layouter",
                    "Cannot subtract number and length in calc() for `{}`",
                    name
                );
                None
            }
        },
        CssValue::Keyword(o) if o == "*" => match (left, right) {
            (CalcValue::Number(x), CalcValue::Number(y)) => Some(CalcValue::Number(x * y)),
            (CalcValue::Number(x), CalcValue::Length(y)) => {
                Some(CalcValue::Length(Length::Mul(Box::new(y), x)))
            }
            (CalcValue::Length(x), CalcValue::Number(y)) => {
                Some(CalcValue::Length(Length::Mul(Box::new(x), y)))
            }
            _ => {
                log::error!(
                    target: "Layouter",
                    "Cannot multiply two lengths in calc() for `{}`",
                    name
                );
                None
            }
        },
        CssValue::Keyword(o) if o == "/" => match (left, right) {
            (CalcValue::Number(_), CalcValue::Number(0.0))
            | (CalcValue::Length(_), CalcValue::Number(0.0)) => {
                log::error!(
                    target: "Layouter",
                    "Division by zero in calc() for `{}`",
                    name
                );
                None
            }
            (CalcValue::Number(x), CalcValue::Number(y)) => Some(CalcValue::Number(x / y)),
            (CalcValue::Length(x), CalcValue::Number(y)) => {
                Some(CalcValue::Length(Length::Div(Box::new(x), y)))
            }
            _ => {
                log::error!(
                    target: "Layouter",
                    "Cannot divide by length in calc() for `{}`",
                    name
                );
                None
            }
        },
        _ => {
            log::error!(target: "Layouter", "Unknown operator in calc() for `{}`: {:?}{:?}{:?}", name, left, op, right);
            None
        }
    }
}

/// CssValue を number / length の型情報付きで評価する。
fn resolve_calc_value(
    name: &str,
    value: &CssValue,
    text_flow_style: &TextFlowStyle,
) -> Option<CalcValue> {
    match value {
        CssValue::Length(v, unit) => {
            resolve_length(name, *v, unit, text_flow_style).map(CalcValue::Length)
        }
        CssValue::Number(n) => Some(CalcValue::Number(*n)),
        CssValue::Function(fn_name, args) if fn_name == "calc" && !args.is_empty() => {
            let mut iter = args.iter();
            let mut result = resolve_calc_value(name, iter.next().unwrap(), text_flow_style)?;

            while let (Some(op), Some(val)) = (iter.next(), iter.next()) {
                let rhs = resolve_calc_value(name, val, text_flow_style)?;
                result = calc_combine(name, op, result, rhs)?;
            }

            Some(result)
        }
        CssValue::Function(fn_name, args)
            if (fn_name == "min" || fn_name == "max") && args.len() >= 2 =>
        {
            let mut resolved: Vec<Length> = Vec::with_capacity(args.len());
            for arg in args {
                resolved.push(match resolve_calc_value(name, arg, text_flow_style)? {
                    CalcValue::Length(l) => l,
                    CalcValue::Number(0.0) => Length::Px(0.0),
                    CalcValue::Number(n) => {
                        log::error!(
                            target: "Layouter",
                            "Invalid operand for {fn_name}() in `{}` (expected length): {}",
                            name,
                            n
                        );
                        return None;
                    }
                });
            }
            let mut result = resolved.remove(resolved.len() - 1);
            for arg in resolved.into_iter().rev() {
                result = if fn_name == "min" {
                    Length::Min(Box::new(arg), Box::new(result))
                } else {
                    Length::Max(Box::new(arg), Box::new(result))
                };
            }
            Some(CalcValue::Length(result))
        }
        CssValue::Function(fn_name, args) if fn_name == "clamp" && args.len() == 3 => {
            Some(CalcValue::Length(Length::Clamp {
                min: Box::new(resolve_css_len(name, &args[0], text_flow_style)?),
                val: Box::new(resolve_css_len(name, &args[1], text_flow_style)?),
                max: Box::new(resolve_css_len(name, &args[2], text_flow_style)?),
            }))
        }
        _ => {
            log::error!(
                target: "Layouter",
                "Invalid lenth value for `{}`: {:?}",
                name,
                value
            );
            None
        }
    }
}

/// Resolve CssValue to Length.
fn resolve_css_len(
    name: &str,
    css_len: &CssValue,
    text_flow_style: &TextFlowStyle,
) -> Option<Length> {
    match &css_len {
        CssValue::Length(v, unit) => resolve_length(name, *v, unit, text_flow_style),
        CssValue::Number(0.0) => Some(Length::Px(0.0)),
        CssValue::Keyword(_) => None,
        CssValue::Function(_, _) => match resolve_calc_value(name, css_len, text_flow_style)? {
            CalcValue::Length(l) => Some(l),
            CalcValue::Number(0.0) => Some(Length::Px(0.0)),
            CalcValue::Number(n) => {
                log::error!(
                    target: "Layouter",
                    "calc() resolved to a number ({}) for `{}` (expected length)",
                    n,
                    name
                );
                None
            }
        },
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
    text_flow_style: &TextFlowStyle,
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
        .map(|value| parse_grid_track(name, value, text_flow_style))
        .collect()
}

const GRID_SPAN_TO_END: usize = usize::MAX;

fn parse_grid_placement(value: &CssValue) -> Option<GridPlacement> {
    let values: Vec<&CssValue> = match value {
        CssValue::List(values) => values.iter().collect(),
        value => vec![value],
    };
    if matches!(values.as_slice(), [CssValue::Keyword(keyword)] if keyword == "auto") {
        return Some(GridPlacement::default());
    }

    let slash = values
        .iter()
        .position(|value| matches!(value, CssValue::Keyword(keyword) if keyword == "/"));
    let (start_values, end_values) = slash.map_or((values.as_slice(), &[][..]), |slash| {
        (&values[..slash], &values[slash + 1..])
    });
    let start = parse_positive_grid_line(start_values)?;
    if end_values.is_empty() {
        return Some(GridPlacement {
            start: Some(start),
            span: 1,
        });
    }
    if matches!(end_values, [CssValue::Number(end)] if *end == -1.0) {
        return Some(GridPlacement {
            start: Some(start),
            span: GRID_SPAN_TO_END,
        });
    }
    // TODO: Resolve every negative CSS grid line against the explicit grid, not only -1.
    let end = parse_positive_grid_line(end_values)?;
    (end > start).then_some(GridPlacement {
        start: Some(start),
        span: end - start,
    })
}

fn parse_positive_grid_line(values: &[&CssValue]) -> Option<usize> {
    let [CssValue::Number(line)] = values else {
        return None;
    };
    (*line >= 1.0 && line.fract().abs() < f32::EPSILON).then_some(*line as usize)
}

fn parse_grid_track(
    name: &str,
    value: &CssValue,
    text_flow_style: &TextFlowStyle,
) -> Option<GridTrack> {
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
                Box::new(parse_grid_track(name, minimum, text_flow_style)?),
                Box::new(parse_grid_track(name, maximum, text_flow_style)?),
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
                .map(|value| parse_grid_track(name, value, text_flow_style))
                .collect::<Option<Vec<_>>>()?;
            (!pattern.is_empty()).then_some(GridTrack::Repeat(repeat, pattern))
        }
        _ => resolve_css_len(name, value, text_flow_style)
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

/// Mix two or more colors according to the `color-mix()` interpolation
/// rules: weights are normalized, alpha is premultiplied, and the
/// interpolation happens in the requested color space.
fn mix_colors(colors: &[Color], weights: &[f32], space: &str) -> Color {
    let n = colors.len().min(weights.len());
    let mut acc = colors[0];
    let mut acc_w = weights[0];
    for i in 1..n {
        if weights[i] <= 0.0 {
            continue;
        }
        acc = match space {
            "lch" => mix_two_lch(acc, colors[i], acc_w, weights[i]),
            _ => mix_two_srgb(acc, colors[i], acc_w, weights[i]),
        };
        acc_w += weights[i];
    }
    acc
}

/// Interpolate between two colors in sRGB with premultiplied alpha.
fn mix_two_srgb(a: Color, b: Color, wa: f32, wb: f32) -> Color {
    let total = wa + wb;
    if total <= 0.0 {
        return Color(0, 0, 0, 0);
    }
    let f = wb / total;
    let al = a.to_linear_f32_array();
    let bl = b.to_linear_f32_array();
    let mut c = [0.0f32; 4];
    for i in 0..4 {
        c[i] = al[i] * al[3] * (1.0 - f) + bl[i] * bl[3] * f;
    }
    let alpha = al[3] * (1.0 - f) + bl[3] * f;
    if alpha > 0.0 {
        for i in 0..3 {
            c[i] /= alpha;
        }
    }
    c[3] = alpha;
    Color::from_linear_f32_array(c)
}

/// Interpolate between two colors in LCH with premultiplied alpha.
fn mix_two_lch(a: Color, b: Color, wa: f32, wb: f32) -> Color {
    let total = wa + wb;
    if total <= 0.0 {
        return Color(0, 0, 0, 0);
    }
    let f = wb / total;
    let a_alpha = a.3 as f32 / 255.0;
    let b_alpha = b.3 as f32 / 255.0;
    let (al, ac, ah) = rgb_to_lch(a);
    let (bl, bc, bh) = rgb_to_lch(b);
    let lm = al * a_alpha * (1.0 - f) + bl * b_alpha * f;
    let cm = ac * a_alpha * (1.0 - f) + bc * b_alpha * f;
    let hm = lerp_hue(ah, bh, f);
    let alpha = a_alpha * (1.0 - f) + b_alpha * f;
    let (l, c) = if alpha > 0.0 {
        (lm / alpha, cm / alpha)
    } else {
        (lm, cm)
    };
    lch_to_color(l, c, hm, alpha)
}

/// Interpolate a hue angle along the shortest arc.
fn lerp_hue(a: f32, b: f32, t: f32) -> f32 {
    let mut d = b - a;
    if d > 180.0 {
        d -= 360.0;
    } else if d < -180.0 {
        d += 360.0;
    }
    (a + d * t).rem_euclid(360.0)
}

const D65_WHITE: (f32, f32, f32) = (0.95047, 1.0, 1.08883);

fn lab_f(t: f32) -> f32 {
    const EPS: f32 = 6.0 / 29.0;
    if t > EPS * EPS * EPS {
        t.cbrt()
    } else {
        t / (3.0 * EPS * EPS) + 4.0 / 29.0
    }
}

fn lab_f_inv(t: f32) -> f32 {
    const EPS: f32 = 6.0 / 29.0;
    if t > EPS {
        t * t * t
    } else {
        3.0 * EPS * EPS * (t - 4.0 / 29.0)
    }
}

/// Convert sRGB (0..1 channels) to CIE XYZ (D65).
fn srgb_to_xyz(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let r = Color::srgb_to_linear(r);
    let g = Color::srgb_to_linear(g);
    let b = Color::srgb_to_linear(b);
    let x = 0.4124564 * r + 0.3575761 * g + 0.1804375 * b;
    let y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b;
    let z = 0.0193339 * r + 0.1191920 * g + 0.9503041 * b;
    (x, y, z)
}

/// Convert CIE XYZ (D65) to sRGB (0..1 channels).
fn xyz_to_srgb(x: f32, y: f32, z: f32) -> (f32, f32, f32) {
    let r = 3.2404542 * x - 1.5371385 * y - 0.4985314 * z;
    let g = -0.9692660 * x + 1.8760108 * y + 0.0415560 * z;
    let b = 0.0556434 * x - 0.2040259 * y + 1.0572252 * z;
    (
        Color::linear_to_srgb(r),
        Color::linear_to_srgb(g),
        Color::linear_to_srgb(b),
    )
}

fn xyz_to_lab(x: f32, y: f32, z: f32) -> (f32, f32, f32) {
    let (wx, wy, wz) = D65_WHITE;
    let fx = lab_f(x / wx);
    let fy = lab_f(y / wy);
    let fz = lab_f(z / wz);
    let l = 116.0 * fy - 16.0;
    let a = 500.0 * (fx - fy);
    let b = 200.0 * (fy - fz);
    (l, a, b)
}

fn lab_to_xyz(l: f32, a: f32, b: f32) -> (f32, f32, f32) {
    let (wx, wy, wz) = D65_WHITE;
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    (wx * lab_f_inv(fx), wy * lab_f_inv(fy), wz * lab_f_inv(fz))
}

/// Convert an sRGB color to LCH (L 0..100, C >= 0, H 0..360).
fn rgb_to_lch(c: Color) -> (f32, f32, f32) {
    let (x, y, z) = srgb_to_xyz(c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0);
    let (l, a, b) = xyz_to_lab(x, y, z);
    let chroma = (a * a + b * b).sqrt();
    let hue = b.atan2(a).to_degrees().rem_euclid(360.0);
    (l, chroma, hue)
}

/// Convert LCH (L 0..100, C >= 0, H 0..360) back to an sRGB color.
fn lch_to_color(l: f32, chroma: f32, hue: f32, alpha: f32) -> Color {
    let hr = hue.to_radians();
    let a = chroma * hr.cos();
    let b = chroma * hr.sin();
    let (x, y, z) = lab_to_xyz(l, a, b);
    let (r, g, b) = xyz_to_srgb(x, y, z);
    Color(
        (r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (b.clamp(0.0, 1.0) * 255.0).round() as u8,
        (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

/// Parse `color-mix(in <space>, <color> [<percentage>], ...)`.
fn parse_color_mix(args: &[CssValue], name: &str, color_scheme: ColorScheme) -> Option<Color> {
    // args: [Keyword("in"), Keyword("<space>"), <color>..., ...]
    if !matches!(args.first(), Some(CssValue::Keyword(k)) if k.eq_ignore_ascii_case("in")) {
        return None;
    }
    let space = match args.get(1) {
        Some(CssValue::Keyword(k)) => k.to_ascii_lowercase(),
        _ => return None,
    };

    let mut colors: Vec<Color> = Vec::new();
    let mut weights: Vec<Option<f32>> = Vec::new();
    let mut i = 2;
    while i < args.len() {
        let color = resolve_css_color(name, &args[i], color_scheme)?;
        i += 1;
        let mut weight = None;
        if i < args.len() {
            match &args[i] {
                CssValue::Number(n) => {
                    weight = Some(*n);
                    i += 1;
                }
                CssValue::Length(p, Unit::Percent) => {
                    weight = Some(*p);
                    i += 1;
                }
                _ => {}
            }
        }
        colors.push(color);
        weights.push(weight);
    }

    if colors.len() < 2 {
        return None;
    }

    // Resolve missing weights to the remaining percentage.
    let specified_sum: f32 = weights.iter().flatten().sum();
    let missing = weights.iter().filter(|w| w.is_none()).count();
    let remainder = (100.0 - specified_sum).max(0.0);
    let mut resolved: Vec<f32> = Vec::with_capacity(colors.len());
    for weight in &weights {
        match weight {
            Some(v) => resolved.push(*v),
            None if missing > 0 => resolved.push(remainder / missing as f32),
            None => resolved.push(0.0),
        }
    }
    let total: f32 = resolved.iter().sum();
    let normalized: Vec<f32> = if total > 0.0 {
        resolved.iter().map(|v| v / total).collect()
    } else {
        vec![1.0 / colors.len() as f32; colors.len()]
    };

    Some(mix_colors(&colors, &normalized, &space))
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
            let mut channels = Vec::new();
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
                            channels.push(arg);
                        }
                    }
                    CssValue::Length(percent, Unit::Percent) if after_slash => {
                        alpha = Some(*percent / 100.0);
                    }
                    CssValue::Length(_, Unit::Percent | Unit::Deg) if !after_slash => {
                        channels.push(arg);
                    }
                    _ => return None,
                }
            }

            // Legacy hsla(h, s, l, a) puts alpha in the fourth comma-separated
            // channel instead of after a slash.
            if channels.len() == 4 && alpha.is_none() {
                alpha = match channels.pop()? {
                    CssValue::Number(value) => Some(*value),
                    CssValue::Length(value, Unit::Percent) => Some(*value / 100.0),
                    _ => return None,
                };
            }
            let [hue, saturation, lightness] = channels.as_slice() else {
                return None;
            };
            let hue = match hue {
                CssValue::Number(value) | CssValue::Length(value, Unit::Deg) => {
                    value.rem_euclid(360.0)
                }
                _ => return None,
            };
            let percentage = |value: &CssValue| match value {
                CssValue::Length(value, Unit::Percent) => Some(*value / 100.0),
                CssValue::Number(value) if (0.0..=1.0).contains(value) => Some(*value),
                _ => None,
            };
            let saturation = percentage(saturation)?.clamp(0.0, 1.0);
            let lightness = percentage(lightness)?.clamp(0.0, 1.0);

            let alpha = alpha.unwrap_or(1.0).clamp(0.0, 1.0);
            let (r, g, b, a) = hsla_to_rgba(hue, saturation, lightness, alpha);

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

        // color-mix(in <space>, <color> [<percentage>], <color> [<percentage>])
        CssValue::Function(func, args) if func == "color-mix" => {
            parse_color_mix(args, name, color_scheme)
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
        let mut text_flow_style = TextFlowStyle::default();
        let mut overflow = Overflow::default();
        let parsed = apply_declaration(
            name,
            &value,
            &mut style,
            &mut container_style,
            &mut text_style,
            &mut text_flow_style,
            &mut overflow,
            ColorScheme::Light,
        );
        assert!(parsed.is_some());
        style
    }

    fn apply_container_property(name: &str, value: CssValue) -> ContainerStyle {
        let mut style = Style::default();
        let mut container_style = ContainerStyle::default();
        let mut text_style = TextStyle::default();
        let mut text_flow_style = TextFlowStyle::default();
        let mut overflow = Overflow::default();
        let parsed = apply_declaration(
            name,
            &value,
            &mut style,
            &mut container_style,
            &mut text_style,
            &mut text_flow_style,
            &mut overflow,
            ColorScheme::Light,
        );
        assert!(parsed.is_some());
        container_style
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
    fn z_index_accepts_integers_and_auto() {
        assert_eq!(
            apply_container_property("z-index", CssValue::Number(999999.0)).z_index,
            Some(999999)
        );
        assert_eq!(
            apply_container_property("z-index", CssValue::Keyword("auto".into())).z_index,
            None
        );
    }

    #[test]
    fn float_keywords_are_preserved_for_layout_blockification() {
        assert_eq!(
            apply_container_property("float", CssValue::Keyword("left".into())).css_float,
            CssFloat::Left
        );
        assert_eq!(
            apply_container_property("float", CssValue::Keyword("right".into())).css_float,
            CssFloat::Right
        );
        assert_eq!(
            apply_container_property("float", CssValue::Keyword("none".into())).css_float,
            CssFloat::None
        );
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
    fn nested_calc_operands_resolve_with_type_checking() {
        // calc(calc(1 - 0) * 10px) => Length::Mul(Px(10), 1.0)
        let style = apply_layout_property(
            "margin-top",
            CssValue::Function(
                "calc".into(),
                vec![
                    CssValue::Function(
                        "calc".into(),
                        vec![
                            CssValue::Number(1.0),
                            CssValue::Keyword("-".into()),
                            CssValue::Number(0.0),
                        ],
                    ),
                    CssValue::Keyword("*".into()),
                    CssValue::Length(10.0, Unit::Px),
                ],
            ),
        );
        assert_eq!(
            style.spacing.margin_top,
            LengthOrAuto::Length(Length::Mul(Box::new(Length::Px(10.0)), 1.0))
        );
    }

    #[test]
    fn calc_rejects_mixed_number_length_arithmetic() {
        let text_flow_style = TextFlowStyle::default();
        // 10px + 5 is invalid: cannot add a number to a length
        let add = CssValue::Function(
            "calc".into(),
            vec![
                CssValue::Length(10.0, Unit::Px),
                CssValue::Keyword("+".into()),
                CssValue::Number(5.0),
            ],
        );
        assert_eq!(resolve_css_len("margin-top", &add, &text_flow_style), None);
        // 10px * 5px is invalid: cannot multiply two lengths
        let mul = CssValue::Function(
            "calc".into(),
            vec![
                CssValue::Length(10.0, Unit::Px),
                CssValue::Keyword("*".into()),
                CssValue::Length(5.0, Unit::Px),
            ],
        );
        assert_eq!(resolve_css_len("margin-top", &mul, &text_flow_style), None);
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
        let mut text_flow_style = TextFlowStyle::default();
        let mut overflow = Overflow::default();
        let parsed = apply_declaration(
            "overflow",
            &value,
            &mut style,
            &mut container_style,
            &mut text_style,
            &mut text_flow_style,
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
        let mut text_flow_style = TextFlowStyle::default();
        let mut overflow = Overflow::default();

        assert!(
            apply_declaration(
                "overflow-x",
                &CssValue::Keyword("scroll".into()),
                &mut style,
                &mut container_style,
                &mut text_style,
                &mut text_flow_style,
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
                &mut text_flow_style,
                &mut overflow,
                ColorScheme::Light,
            )
            .is_some()
        );
        assert_eq!(overflow, Overflow { x: true, y: true });
    }

    #[test]
    fn logical_inline_margins_apply_to_both_physical_sides() {
        let mut style = Style::default();
        let mut container_style = ContainerStyle::default();
        let mut text_style = TextStyle::default();
        let mut text_flow_style = TextFlowStyle::default();
        let mut overflow = Overflow::default();

        assert!(
            apply_declaration(
                "margin-inline",
                &CssValue::Keyword("auto".into()),
                &mut style,
                &mut container_style,
                &mut text_style,
                &mut text_flow_style,
                &mut overflow,
                ColorScheme::Light,
            )
            .is_some()
        );
        assert_eq!(style.spacing.margin_left, LengthOrAuto::Auto);
        assert_eq!(style.spacing.margin_right, LengthOrAuto::Auto);
    }

    #[test]
    fn clamp_font_size_uses_pixel_bound_for_viewport_preference() {
        let mut style = Style::default();
        let mut container_style = ContainerStyle::default();
        let mut text_style = TextStyle::default();
        let mut text_flow_style = TextFlowStyle::default();
        let mut overflow = Overflow::default();
        let clamp = CssValue::Function(
            "clamp".into(),
            vec![
                CssValue::Length(60.0, Unit::Px),
                CssValue::Length(8.4, Unit::Vw),
                CssValue::Length(100.0, Unit::Px),
            ],
        );

        assert!(
            apply_declaration(
                "font-size",
                &clamp,
                &mut style,
                &mut container_style,
                &mut text_style,
                &mut text_flow_style,
                &mut overflow,
                ColorScheme::Light,
            )
            .is_some()
        );
        assert_eq!(text_flow_style.font_size, 100.0);
    }

    #[test]
    fn unsupported_overflow_value_is_rejected() {
        let mut style = Style::default();
        let mut container_style = ContainerStyle::default();
        let mut text_style = TextStyle::default();
        let mut text_flow_style = TextFlowStyle::default();
        let mut overflow = Overflow::default();
        assert!(
            apply_declaration(
                "overflow",
                &CssValue::Number(1.0),
                &mut style,
                &mut container_style,
                &mut text_style,
                &mut text_flow_style,
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
            InheritedCss::default(),
            ElementChain::default(),
            ColorScheme::Light,
            ScriptingMode::default(),
        )
    }

    fn text_content(info: &InfoNode) -> String {
        let mut text = match &info.kind {
            NodeKind::Text { text, .. } => text.clone(),
            _ => String::new(),
        };
        for child in &info.children {
            text.push_str(&text_content(child));
        }
        text
    }

    #[test]
    fn noscript_content_is_absent_from_layout_when_scripting_is_enabled() {
        let info = layout_for(
            "<html><body><p>before</p><noscript><p>fallback</p></noscript><p>after</p></body></html>",
            "",
        );
        let text = text_content(&info);
        assert!(text.contains("before"));
        assert!(text.contains("after"));
        assert!(!text.contains("fallback"));
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
    fn floated_carousel_slides_shrink_to_fixed_descendant_width() {
        let html = r#"
            <html><body>
                <div class="track">
                    <div class="slide"><div class="card"></div></div>
                    <div class="slide"><div class="card"></div></div>
                </div>
            </body></html>
        "#;
        let css = r#"
            .track { width: 1000px; }
            .slide { float: left; padding-right: 30px; }
            .card { width: 144px; height: 100px; }
        "#;
        let (mut layout, _) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 1200.0, 600.0);

        fn floated_children(node: &LayoutNode) -> Option<Vec<&LayoutNode>> {
            let children: Vec<_> = node
                .children
                .iter()
                .filter_map(LayoutChild::node)
                .filter(|child| {
                    child.style.size.auto_behavior == AutoSizeBehavior::ShrinkToFit
                        && child.style.display.inner == InnerDisplay::FlowRoot
                })
                .collect();
            if children.len() == 2 {
                return Some(children);
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(floated_children)
        }

        let slides = floated_children(&layout).expect("two floated slides");
        let first = slides[0].layout_box.iter().next().unwrap();
        let second = slides[1].layout_box.iter().next().unwrap();
        assert_eq!(first.content_box.width, 144.0);
        assert_eq!(first.border_box.width, 174.0);
        assert_eq!(second.border_box.x, 174.0);
        assert_eq!(second.border_box.y, 0.0);
    }

    #[test]
    fn oversized_block_with_auto_horizontal_margins_starts_at_parent_edge() {
        let html = r#"
            <html><body>
                <div class="parent"><div class="wide"></div></div>
            </body></html>
        "#;
        let css = r#"
            .parent { width: 300px; }
            .wide { width: 500px; height: 20px; margin-left: auto; margin-right: auto; }
        "#;
        let (mut layout, _) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);
        correct_oversized_auto_horizontal_margins(&mut layout);

        fn wide_box(node: &LayoutNode) -> Option<ui_layout::Rect> {
            if node.style.size.width == LengthOrAuto::Length(Length::Px(500.0)) {
                return node.layout_box.iter().next().map(|model| model.border_box);
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(wide_box)
        }

        let wide = wide_box(&layout).expect("wide child");
        assert_eq!(wide.x, 0.0);
        assert_eq!(wide.width, 500.0);
    }

    #[test]
    fn adjacent_inline_blocks_advance_past_padding_and_margins() {
        let html = r#"
            <html><body><div class="row"><a>First</a><a>Second</a></div></body></html>
        "#;
        let css = r#"
            a { display: inline-block; padding: 0 12px; margin-right: 12px; }
        "#;
        let (mut layout, _) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

        fn inline_blocks(node: &LayoutNode) -> Option<Vec<ui_layout::Rect>> {
            let boxes: Vec<_> = node
                .children
                .iter()
                .filter_map(LayoutChild::node)
                .filter(|child| child.style.display.inner == InnerDisplay::FlowRoot)
                .filter_map(|child| child.layout_box.iter().next().map(|model| model.border_box))
                .collect();
            if boxes.len() == 2 {
                return Some(boxes);
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(inline_blocks)
        }

        correct_atomic_inline_spacing(&mut layout);

        let boxes = inline_blocks(&layout).expect("two inline blocks");
        assert!(boxes[1].x >= boxes[0].right() + 12.0);
    }

    #[test]
    fn atomic_inline_block_starts_below_its_top_margin() {
        let html = r#"
            <html><body><main><div>Content</div></main></body></html>
        "#;
        let css = r#"
            main { display: inline-block; width: 100%; margin-top: 50px; }
        "#;
        let (mut layout, _) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);
        correct_atomic_inline_spacing(&mut layout);

        fn main_box(node: &LayoutNode) -> Option<ui_layout::Rect> {
            if node.style.display
                == (Display {
                    outer: OuterDisplay::Inline,
                    inner: InnerDisplay::FlowRoot,
                })
                && node.style.spacing.margin_top == LengthOrAuto::Length(Length::Px(50.0))
            {
                return node.layout_box.iter().next().map(|model| model.border_box);
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(main_box)
        }

        let main = main_box(&layout).expect("main inline block");
        assert_eq!(main.y, 50.0);
    }

    #[test]
    fn atomic_inline_after_block_starts_below_the_block_margin() {
        let html = r#"
            <html><body><div class="copy">
                <p class="overline">Status</p><h2>Heading</h2><p class="summary">Summary</p><a>Continue</a>
            </div></body></html>
        "#;
        let css = r#"
            .copy { width: 600px; text-align: center; }
            .copy .overline { height: 25px; margin: 0 0 18px; }
            .copy h2 { height: 58px; margin: 0 0 24px; }
            .copy .summary { width: 560px; height: 80px; margin: 0 0 28px; }
            .copy a { display: inline-flex; width: 200px; height: 30px; }
        "#;
        let (mut layout, info) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);
        correct_atomic_inline_spacing_with_info(&mut layout, &info);

        fn box_with_width(node: &LayoutNode, width: f32) -> Option<ui_layout::Rect> {
            if node.style.size.width == LengthOrAuto::Length(Length::Px(width)) {
                return node.layout_box.iter().next().map(|model| model.border_box);
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(|child| box_with_width(child, width))
        }

        let summary = box_with_width(&layout, 560.0).expect("summary");
        let action = box_with_width(&layout, 200.0).expect("inline action");
        assert_eq!(action.y, summary.bottom() + 28.0);
        assert_eq!(action.x, 200.0);
    }

    #[test]
    fn indentation_before_block_does_not_create_anonymous_line() {
        let html = "<html><body><div>\n    <section></section>\n</div></body></html>";
        let css = "section { display: block; width: 100px; height: 20px; }";
        let (mut layout, _) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

        fn section_box(node: &LayoutNode) -> Option<ui_layout::Rect> {
            if node.style.size.width == LengthOrAuto::Length(Length::Px(100.0)) {
                return node.layout_box.iter().next().map(|model| model.border_box);
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(section_box)
        }

        assert_eq!(section_box(&layout).expect("section").y, 0.0);
    }

    #[test]
    fn auto_flex_height_includes_child_vertical_margins() {
        let html = "<html><body><div class='row'><span></span></div></body></html>";
        let css = r#"
            .row { display: flex; }
            span { display: inline-block; width: 20px; height: 30px; margin: 10px 0; }
        "#;
        let (mut layout, _) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);
        correct_atomic_inline_spacing(&mut layout);

        fn flex_box(node: &LayoutNode) -> Option<ui_layout::Rect> {
            if node.style.display.inner == InnerDisplay::Flex {
                return node.layout_box.iter().next().map(|model| model.border_box);
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(flex_box)
        }

        assert!(flex_box(&layout).expect("flex row").height >= 50.0);
    }

    #[test]
    fn grid_min_height_pushes_later_block_flow_content() {
        let html = "<html><body><header></header><main></main></body></html>";
        let css = r#"
            header { display: grid; min-height: 48px; }
            main { display: block; height: 20px; }
        "#;
        let (mut layout, _) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);
        correct_atomic_inline_spacing(&mut layout);

        fn children(node: &LayoutNode) -> Option<Vec<ui_layout::Rect>> {
            let boxes: Vec<_> = node
                .children
                .iter()
                .filter_map(LayoutChild::node)
                .filter(|child| child.style.display.outer == OuterDisplay::Block)
                .filter_map(|child| child.layout_box.iter().next().map(|model| model.border_box))
                .collect();
            if boxes.len() == 2 {
                return Some(boxes);
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(children)
        }

        let boxes = children(&layout).expect("header and main");
        assert!(boxes[0].height >= 48.0);
        assert!(boxes[1].y >= boxes[0].bottom());
    }

    #[test]
    fn grid_auto_track_can_measure_flex_contents() {
        let html = "<html><body><div class='grid'><a>A</a><nav><span></span><span></span></nav><a>B</a></div></body></html>";
        let css = r#"
            .grid { display: grid; width: 1024px; grid-template-columns: 1fr auto 1fr; }
            nav { display: flex; gap: 10px; }
            nav span { display: block; width: 100px; height: 10px; }
        "#;
        let (mut layout, _) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 1280.0, 600.0);
        assert!(constrain_auto_grid_track_items(&mut layout));
        ui_layout::LayoutEngine::layout(&mut layout, 1280.0, 600.0);

        fn grid(node: &LayoutNode) -> Option<&LayoutNode> {
            if node.style.display.inner == InnerDisplay::Grid {
                return Some(node);
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(grid)
        }
        let grid = grid(&layout).expect("grid");
        let items: Vec<_> = grid.children.iter().filter_map(LayoutChild::node).collect();
        assert_eq!(items[0].style.display.outer, OuterDisplay::Block);
        assert_eq!(items[2].style.display.outer, OuterDisplay::Block);
        let middle = items[1].layout_box.iter().next().expect("middle");
        assert!((middle.content_box.width - 210.0).abs() < 0.5);
        assert!(items[0].layout_box.width_box() > 400.0);
        assert!(items[2].layout_box.width_box() > 400.0);
    }

    #[test]
    fn negative_grid_end_line_spans_to_the_last_explicit_track() {
        let html = "<html><body><div class='grid'><article class='large'></article><article></article></div></body></html>";
        let css = r#"
            .grid { display: grid; width: 600px; grid-template-columns: repeat(2, 1fr); }
            .large { grid-column: 1 / -1; height: 20px; }
        "#;
        let (mut layout, _) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

        fn grid(layout: &LayoutNode) -> Option<&LayoutNode> {
            if layout.style.display.inner == InnerDisplay::Grid {
                return Some(layout);
            }
            layout
                .children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(grid)
        }

        let grid = grid(&layout).expect("grid");
        let large = grid
            .children
            .iter()
            .filter_map(LayoutChild::node)
            .next()
            .expect("large grid item");
        assert_eq!(large.style.grid_column.start, Some(1));
        assert_eq!(large.style.grid_column.span, 2);
        assert!((large.layout_box.width_box() - 600.0).abs() < 0.5);
    }

    #[test]
    fn single_row_grid_centers_items_after_min_height_growth() {
        let html = "<html><body><div class='grid'><span></span></div></body></html>";
        let css = r#"
            .grid { display: grid; min-height: 48px; align-items: center; }
            span { display: block; width: 20px; height: 10px; }
        "#;
        let (mut layout, _) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);
        correct_atomic_inline_spacing(&mut layout);

        fn grid_item(node: &LayoutNode) -> Option<ui_layout::Rect> {
            if node.style.display.inner == InnerDisplay::Grid {
                return node
                    .children
                    .iter()
                    .filter_map(LayoutChild::node)
                    .find_map(|child| {
                        child.layout_box.iter().next().map(|model| model.border_box)
                    });
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(grid_item)
        }

        let item = grid_item(&layout).expect("grid item");
        assert!((item.y - 19.0).abs() < 0.5, "item={item:?}");
    }

    #[test]
    fn grid_justify_self_end_uses_the_end_of_its_track() {
        let html = "<html><body><div class='grid'><a>A</a><nav><span></span></nav><a class='end'>B</a></div></body></html>";
        let css = r#"
            .grid { display: grid; width: 300px; margin-left: 50px; grid-template-columns: 1fr auto 1fr; }
            nav { display: flex; }
            nav span { display: block; width: 100px; height: 10px; }
            .end { justify-self: end; }
        "#;
        let (mut layout, _) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);
        assert!(constrain_auto_grid_track_items(&mut layout));
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);
        correct_atomic_inline_spacing(&mut layout);

        fn end_item(node: &LayoutNode) -> Option<ui_layout::Rect> {
            if node.style.spacing.margin_left == LengthOrAuto::Auto {
                return node.layout_box.iter().next().map(|model| model.border_box);
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(end_item)
        }

        let item = end_item(&layout).expect("end-aligned item");
        assert!((item.right() - 300.0).abs() < 0.5, "item={item:?}");
    }

    #[test]
    fn inline_flex_lays_out_direct_text_with_inherited_style() {
        let html = "<html><body><a class='action'>目指すこと<span>›</span></a></body></html>";
        let css = r#"
            .action { display: inline-flex; gap: 5px; color: #0066cc; font-size: 19px; }
            .action span { font-size: 22px; }
        "#;
        let (mut layout, info) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);
        refresh_missing_text_layout_results(&mut layout, &info, (800.0, 600.0));

        fn inline_flex(layout: &LayoutNode) -> Option<&LayoutNode> {
            if layout.style.display.inner == InnerDisplay::Flex {
                return Some(layout);
            }
            layout
                .children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(inline_flex)
        }
        let flex = inline_flex(&layout).expect("inline flex");
        let span = flex
            .children
            .iter()
            .filter_map(LayoutChild::node)
            .next()
            .expect("span flex item");
        assert_eq!(span.style.display.outer, OuterDisplay::Block);

        let label_style = text_style_for(&info, "目指すこと");
        assert_eq!(text_flow_style_for(&info, "目指すこと").font_size, 19.0);
        assert_eq!(label_style.color, Color(0, 102, 204, 255));

        fn text_id_for(info: &InfoNode, content: &str) -> Option<usize> {
            if let NodeKind::Text { text, text_id, .. } = &info.kind
                && text == content
            {
                return Some(*text_id);
            }
            info.children
                .iter()
                .find_map(|child| text_id_for(child, content))
        }

        let label_id = text_id_for(&info, "目指すこと").expect("label text id");
        let result = TextFlowLayouter::get_result(label_id).expect("label layout result");
        assert_eq!(result.line_texts, vec!["目指すこと"]);
        assert!(result.spans[0].line_pos.0 >= 0.0);
    }

    #[test]
    fn flex_navigation_blockifies_and_spaces_inline_links() {
        let html = "<html><body><nav><a>目指す</a><a>違い</a><a>開発</a></nav></body></html>";
        let css = "nav { display: flex; gap: 30px; } a { display: inline; }";
        let (mut layout, _) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);
        correct_atomic_inline_spacing(&mut layout);

        fn navigation(layout: &LayoutNode) -> Option<&LayoutNode> {
            let links: Vec<_> = layout
                .children
                .iter()
                .filter_map(LayoutChild::node)
                .filter(|child| child.style.display.outer == OuterDisplay::Block)
                .collect();
            if layout.style.display.inner == InnerDisplay::Flex && links.len() == 3 {
                return Some(layout);
            }
            layout
                .children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(navigation)
        }

        let nav = navigation(&layout).expect("flex navigation");
        let links: Vec<_> = nav
            .children
            .iter()
            .filter_map(LayoutChild::node)
            .filter_map(|child| child.layout_box.iter().next())
            .collect();
        assert_eq!(links.len(), 3);
        assert!(links[1].border_box.x >= links[0].border_box.right() + 29.5);
        assert!(links[2].border_box.x >= links[1].border_box.right() + 29.5);
    }

    #[test]
    fn bottom_anchored_grid_repositions_after_min_height_growth() {
        let html = "<html><body><div class='parent'><div class='dialog'></div></div></body></html>";
        let css = r#"
            .parent { position: relative; width: 300px; height: 200px; }
            .dialog { position: absolute; right: 20px; bottom: -10px; display: grid; width: 80px; min-height: 100px; }
        "#;
        let (mut layout, _) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);
        correct_atomic_inline_spacing(&mut layout);

        fn dialog(node: &LayoutNode) -> Option<ui_layout::Rect> {
            if node.style.position.kind == Position::Absolute {
                return node.layout_box.iter().next().map(|model| model.border_box);
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(dialog)
        }

        let dialog = dialog(&layout).expect("dialog");
        assert!(dialog.height >= 100.0);
        assert!((dialog.y - 110.0).abs() < 0.5, "dialog={dialog:?}");
    }

    #[test]
    fn border_box_button_keeps_declared_size_with_padding() {
        let html = "<html><body><button>Search</button></body></html>";
        let css = "button { display: inline-block; box-sizing: border-box; width: 40px; height: 40px; padding: 12px 16px; }";
        let (mut layout, _) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

        fn button_box(node: &LayoutNode) -> Option<ui_layout::Rect> {
            if node.style.box_sizing == BoxSizing::BorderBox
                && node.style.size.width == LengthOrAuto::Length(Length::Px(40.0))
            {
                return node.layout_box.iter().next().map(|model| model.border_box);
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(button_box)
        }

        let button = button_box(&layout).expect("border-box button");
        assert_eq!((button.width, button.height), (40.0, 40.0));
    }

    #[test]
    fn full_width_inline_blocks_wrap_onto_separate_lines() {
        let html = r#"
            <html><body><main><section>First</section><section>Second</section></main></body></html>
        "#;
        let css = r#"
            main { width: 300px; }
            section { display: inline-block; width: 100%; height: 40px; margin-bottom: 10px; }
        "#;
        let (mut layout, _) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);
        correct_atomic_inline_spacing(&mut layout);

        fn sections(node: &LayoutNode) -> Option<Vec<ui_layout::Rect>> {
            let boxes: Vec<_> = node
                .children
                .iter()
                .filter_map(LayoutChild::node)
                .filter(|child| {
                    child.style.display
                        == (Display {
                            outer: OuterDisplay::Inline,
                            inner: InnerDisplay::FlowRoot,
                        })
                })
                .filter_map(|child| child.layout_box.iter().next().map(|model| model.border_box))
                .collect();
            if boxes.len() == 2 {
                return Some(boxes);
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(sections)
        }

        let boxes = sections(&layout).expect("two inline blocks");
        assert!((boxes[0].x - boxes[1].x).abs() < 0.5);
        assert!(boxes[1].y >= boxes[0].bottom() + 10.0);
    }

    #[test]
    fn auto_flex_container_expands_to_corrected_inline_margin_boxes() {
        let html = r#"
            <html><body><div class="column"><div class="row"><a>First</a><a>Second</a></div></div></body></html>
        "#;
        let css = r#"
            .column { display: flex; flex-direction: column; align-items: center; }
            .row { display: flex; }
            a { display: inline-block; padding: 0 12px; margin-right: 12px; }
        "#;
        let (mut layout, _) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);
        correct_atomic_inline_spacing(&mut layout);

        fn flex_row(node: &LayoutNode) -> Option<&LayoutNode> {
            let atomic_children = node
                .children
                .iter()
                .filter_map(LayoutChild::node)
                .filter(|child| child.style.display.inner == InnerDisplay::FlowRoot)
                .count();
            if node.style.display.inner == InnerDisplay::Flex && atomic_children == 2 {
                return Some(node);
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(flex_row)
        }

        let row = flex_row(&layout).expect("flex row");
        let row_box = row.layout_box.iter().next().unwrap();
        let last = row
            .children
            .iter()
            .filter_map(LayoutChild::node)
            .last()
            .unwrap();
        let last_box = last.layout_box.iter().next().unwrap();
        let required_right = last_box.border_box.right() + 12.0;
        assert!(row_box.content_box.right() >= required_right);
    }

    #[test]
    fn inline_flex_item_wraps_padded_atomic_child_without_overlap() {
        let html = r#"
            <html><body><div class="bar">
                <a><div class="button">One</div></a><a><div class="button">Two</div></a>
            </div></body></html>
        "#;
        let css = r#"
            .bar { display: flex; }
            a { display: inline; }
            .button { display: inline-block; margin: 0 8px; padding: 8px 24px; }
        "#;
        let (mut layout, _) = layout_and_info_for(html, css);
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);
        correct_atomic_inline_spacing(&mut layout);

        fn flex_links(node: &LayoutNode) -> Option<Vec<&LayoutNode>> {
            let links: Vec<_> = node
                .children
                .iter()
                .filter_map(LayoutChild::node)
                .filter(|child| child.style.display.outer == OuterDisplay::Block)
                .collect();
            if node.style.display.inner == InnerDisplay::Flex && links.len() == 2 {
                return Some(links);
            }
            node.children
                .iter()
                .filter_map(LayoutChild::node)
                .find_map(flex_links)
        }

        let links = flex_links(&layout).expect("two inline flex items");
        let first = links[0].layout_box.iter().next().unwrap();
        let second = links[1].layout_box.iter().next().unwrap();
        let first_button = links[0]
            .children
            .iter()
            .filter_map(LayoutChild::node)
            .next()
            .unwrap()
            .layout_box
            .iter()
            .next()
            .unwrap();
        assert!(first.content_box.width >= first_button.border_box.right() + 8.0);
        assert!(second.border_box.x >= first.border_box.right());
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

    #[test]
    fn inline_image_keeps_intrinsic_dimensions_after_layout() {
        let dom = HtmlParser::new(
            r#"<html><body><div><img src="profile.png" alt="profile"></div></body></html>"#,
        )
        .parse();
        let mut images = HashMap::new();
        images.insert(
            "profile.png".to_string(),
            Image::from_rgba(460, 460, vec![255; 460 * 460 * 4]).unwrap(),
        );
        let stylesheet = CssParser::new("img { display: inline-block; }")
            .parse()
            .unwrap();
        let resolved_styles = CssResolver::resolve(&stylesheet);
        let (mut layout, _) = build_layout_and_info_with_images(
            &dom.root,
            &resolved_styles,
            Arc::new(FallbackTextMeasurer),
            InheritedCss::default(),
            ElementChain::default(),
            ColorScheme::Light,
            ScriptingMode::default(),
            &images,
        );
        ui_layout::LayoutEngine::layout(&mut layout, 800.0, 600.0);

        fn image_layout_size(node: &LayoutNode) -> Option<(f32, f32)> {
            if node.style.size.auto_behavior == AutoSizeBehavior::ShrinkToFit {
                return node
                    .layout_box
                    .iter()
                    .next()
                    .map(|box_model| (box_model.content_box.width, box_model.content_box.height));
            }
            node.children.iter().find_map(|child| match child {
                LayoutChild::Node(child) => image_layout_size(child),
                _ => None,
            })
        }

        assert_eq!(image_layout_size(&layout), Some((460.0, 460.0)));
    }

    /// Returns the body element's layout children, unwrapping the document
    /// and `<html>` wrapper nodes.
    fn body_layout_children<'a>(mut node: &'a LayoutNode) -> &'a [LayoutChild] {
        while let [LayoutChild::Node(child)] = node.children.as_slice() {
            node = child;
        }
        &node.children
    }

    /// Recursively counts whitespace-only text nodes in the `InfoNode` tree,
    /// which mirrors the DOM (whitespace stays a separate node, unlike the
    /// layout where it may merge into an adjacent run). This is the reliable
    /// way to assert whether a whitespace node was dropped.
    fn count_whitespace_text_info(info: &InfoNode) -> usize {
        let mut n = 0;
        if let NodeKind::Text { text, .. } = &info.kind {
            if text.chars().all(is_css_whitespace) {
                n += 1;
            }
        }
        n + info
            .children
            .iter()
            .map(count_whitespace_text_info)
            .sum::<usize>()
    }

    #[test]
    fn whitespace_text_between_block_siblings_is_dropped() {
        let html = "<html><body><div>a</div>\n  <div>b</div></body></html>";
        let (layout, _) = layout_and_info_for(html, "");

        let children = body_layout_children(&layout);
        assert_eq!(children.len(), 2, "whitespace text must be dropped");
        assert!(
            children.iter().all(|child| child.node().is_some()),
            "only the two block divs remain"
        );
    }

    #[test]
    fn whitespace_text_adjacent_to_inline_siblings_is_kept() {
        let html = "<html><body><span>a</span> <span>b</span></body></html>";
        let (layout, _) = layout_and_info_for(html, "span { display: inline; }");

        let children = body_layout_children(&layout);
        assert_eq!(children.len(), 3);
        assert!(
            matches!(&children[1], LayoutChild::Custom(_)),
            "space between inline spans must be kept"
        );
    }

    #[test]
    fn whitespace_text_next_to_block_is_dropped() {
        // With the "side" rule, whitespace adjacent to a block on either side
        // is dropped even when the other sibling is inline.
        let html = "<html><body><div>a</div> <span>b</span></body></html>";
        let (_, info) = layout_and_info_for(html, "span { display: inline; }");

        assert_eq!(
            count_whitespace_text_info(&info),
            0,
            "whitespace next to a block must be dropped"
        );
    }

    #[test]
    fn whitespace_text_trailing_after_block_is_dropped() {
        // Trailing whitespace after a single block is adjacent to that block,
        // so it must be dropped.
        let html = "<html><body><div>a</div> \n</body></html>";
        let (_, info) = layout_and_info_for(html, "");

        assert_eq!(
            count_whitespace_text_info(&info),
            0,
            "trailing whitespace after a single block must be dropped"
        );
    }

    #[test]
    fn whitespace_text_around_br_between_block_siblings_is_dropped() {
        let html = "<html><body><div>a</div> \n<br>\n <div>b</div></body></html>";
        let (layout, _) = layout_and_info_for(html, "");

        let children = body_layout_children(&layout);
        assert_eq!(children.len(), 3, "only the two divs and the <br> remain");
        assert!(matches!(&children[0], LayoutChild::Node(_)));
        assert!(
            matches!(&children[1], LayoutChild::Fragment(_)),
            "<br> itself must be kept"
        );
        assert!(matches!(&children[2], LayoutChild::Node(_)));
    }

    #[test]
    fn whitespace_text_after_br_is_dropped() {
        let html = "<html><body><p>a <br> </p></body></html>";
        let (layout, _) = layout_and_info_for(html, "");

        let children = body_layout_children(&layout);
        assert_eq!(children.len(), 2, "whitespace after <br> must be dropped");
        assert!(matches!(&children[0], LayoutChild::Custom(_)));
        assert!(matches!(&children[1], LayoutChild::Fragment(_)));
    }

    #[test]
    fn whitespace_text_before_br_is_dropped() {
        let html = "<html><body><p> <br>b</p></body></html>";
        let (layout, _) = layout_and_info_for(html, "");

        let children = body_layout_children(&layout);
        assert_eq!(children.len(), 2, "whitespace before <br> must be dropped");
        assert!(matches!(&children[0], LayoutChild::Fragment(_)));
        assert!(matches!(&children[1], LayoutChild::Custom(_)));
    }

    #[test]
    fn whitespace_crossing_display_none_between_blocks_is_dropped() {
        // A `display:none` element does not participate in layout, so the
        // whitespace on either side of it is still adjacent to a block once
        // the none element is skipped.
        let html =
            "<html><body><div>a</div> <span class=\"hidden\">x</span> <div>b</div></body></html>";
        let (layout, _) = layout_and_info_for(html, ".hidden { display: none; }");

        let children = body_layout_children(&layout);
        // The none span stays in the tree; only the two whitespace text nodes
        // are dropped (3 children: div, none span, div).
        assert_eq!(children.len(), 3, "two whitespace nodes must be dropped");
        assert!(
            children.iter().all(|child| child.node().is_some()),
            "no stray whitespace inline boxes remain"
        );
    }

    #[test]
    fn whitespace_crossing_display_none_between_inlines_is_kept() {
        // Crossing a `display:none` element reveals inline spans on both
        // sides, so the whitespace is not adjacent to any block or `<br>` and
        // must be kept.
        let html = "<html><body><span>a</span> <span class=\"hidden\">x</span> <span>b</span></body></html>";
        let (_, info) =
            layout_and_info_for(html, "span { display: inline; } .hidden { display: none; }");

        // `count_whitespace_text_info` inspects the DOM-shaped info tree, so the
        // kept whitespace survives as its own node regardless of anonymous-block
        // wrapping in the layout.
        assert_eq!(
            count_whitespace_text_info(&info),
            2,
            "whitespace across a none element between inlines must be kept"
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

    fn text_flow_style_for(info: &InfoNode, content: &str) -> TextFlowStyle {
        fn walk(node: &InfoNode, content: &str) -> Option<TextFlowStyle> {
            if let NodeKind::Text {
                flow_style,
                text: actual,
                ..
            } = &node.kind
                && actual == content
            {
                return Some(*flow_style);
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
    fn custom_properties_cascade_across_rules_and_inherit() {
        let html = r#"
            <html><body>
                <section><p>root theme</p></section>
                <section class="alternate"><p>alternate theme</p></section>
            </body></html>
        "#;
        let css = r#"
            :root { --scratch-accent: #855cd6; }
            p { color: var(--scratch-accent); }
            .alternate { --scratch-accent: #4c97ff; }
        "#;
        let info = layout_for(html, css);

        assert_eq!(
            text_style_for(&info, "root theme").color,
            Color(133, 92, 214, 255)
        );
        assert_eq!(
            text_style_for(&info, "alternate theme").color,
            Color(76, 151, 255, 255)
        );
    }

    #[test]
    fn inline_declaration_can_use_inherited_custom_property() {
        let html = r#"
            <html><body style="--accent: #ff6680">
                <p style="color: var(--accent)">inline theme</p>
            </body></html>
        "#;
        let info = layout_for(html, "");

        assert_eq!(
            text_style_for(&info, "inline theme").color,
            Color(255, 102, 128, 255)
        );
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

    #[test]
    fn hsl_percentage_background_is_resolved() {
        let info = layout_for(
            r#"<html><body><div id="view">x</div></body></html>"#,
            "#view { background-color: hsl(0, 0%, 99%); }",
        );

        fn find_background(node: &InfoNode) -> Option<Color> {
            if let NodeKind::Container { style, .. } = &node.kind
                && let Background::Color(color) = style.background
                && color.3 > 0
            {
                return Some(color);
            }
            node.children.iter().find_map(find_background)
        }
        assert_eq!(find_background(&info), Some(Color(252, 252, 252, 255)));
    }

    fn resolve_color(value: CssValue) -> Color {
        resolve_css_color("test", &value, ColorScheme::Light).expect("color resolves")
    }

    #[test]
    fn hsla_accepts_percentage_channels_and_alpha() {
        assert_eq!(
            resolve_color(CssValue::Function(
                "hsla".into(),
                vec![
                    CssValue::Length(120.0, Unit::Deg),
                    CssValue::Length(100.0, Unit::Percent),
                    CssValue::Length(25.0, Unit::Percent),
                    CssValue::Keyword("/".into()),
                    CssValue::Length(50.0, Unit::Percent),
                ],
            )),
            Color(0, 128, 0, 128)
        );
    }

    #[test]
    fn color_mix_in_srgb_blends_weights() {
        let mixed = resolve_color(CssValue::Function(
            "color-mix".into(),
            vec![
                CssValue::Keyword("in".into()),
                CssValue::Keyword("srgb".into()),
                CssValue::Keyword("red".into()),
                CssValue::Keyword("blue".into()),
            ],
        ));
        // 50/50 of red and blue in linear sRGB.
        assert_eq!(mixed, Color(188, 0, 188, 255));
    }

    #[test]
    fn color_mix_with_percentages_and_missing_weight() {
        let mixed = resolve_color(CssValue::Function(
            "color-mix".into(),
            vec![
                CssValue::Keyword("in".into()),
                CssValue::Keyword("srgb".into()),
                CssValue::Keyword("red".into()),
                CssValue::Length(25.0, Unit::Percent),
                CssValue::Keyword("blue".into()),
            ],
        ));
        // red 25% + blue (missing weight takes the remaining 75%).
        assert_eq!(mixed, Color(137, 0, 225, 255));
    }

    #[test]
    fn color_mix_in_lch_produces_purple() {
        let mixed = resolve_color(CssValue::Function(
            "color-mix".into(),
            vec![
                CssValue::Keyword("in".into()),
                CssValue::Keyword("lch".into()),
                CssValue::Keyword("red".into()),
                CssValue::Keyword("blue".into()),
            ],
        ));
        // Mixing red and blue in LCH stays on the purple hue arc.
        assert!(
            mixed.0 > 0 && mixed.2 > 0,
            "purple has red and blue: {mixed:?}"
        );
        assert!(mixed.1 < 50, "not green: {mixed:?}");
        assert_eq!(mixed.3, 255, "alpha is preserved");
        assert_ne!(mixed, Color(255, 0, 0, 255));
        assert_ne!(mixed, Color(0, 0, 255, 255));
    }

    #[test]
    fn color_mix_alpha_is_premultiplied() {
        let mixed = resolve_color(CssValue::Function(
            "color-mix".into(),
            vec![
                CssValue::Keyword("in".into()),
                CssValue::Keyword("srgb".into()),
                CssValue::Keyword("transparent".into()),
                CssValue::Keyword("blue".into()),
            ],
        ));
        // transparent is (0,0,0,0); mixing with opaque blue gives half alpha.
        assert_eq!(mixed.3, 128);
    }

    #[test]
    fn conic_gradient_parses_stops_and_kind() {
        let args = vec![
            CssValue::Keyword("red".into()),
            CssValue::Length(0.0, Unit::Deg),
            CssValue::Keyword("red".into()),
            CssValue::Length(0.0, Unit::Deg),
            CssValue::Length(1.0, Unit::Deg),
            CssValue::Keyword("red".into()),
            CssValue::Length(2.0, Unit::Deg),
        ];
        let gradient = parse_gradient(
            "conic-gradient",
            &args,
            &TextStyle::default(),
            &TextFlowStyle::default(),
            ColorScheme::Light,
        )
        .expect("conic gradient parses");

        assert!(matches!(
            gradient.kind,
            GradientKind::Conic {
                angle: 0.0,
                position: (0.5, 0.5)
            }
        ));
        assert_eq!(gradient.stops.len(), 4);
        for (stop, expected) in gradient
            .stops
            .iter()
            .zip([0.0f32, 0.0, 1.0 / 360.0, 2.0 / 360.0])
        {
            assert_eq!(stop.position, Some(expected));
            assert_eq!(stop.color, Color(255, 0, 0, 255));
        }
    }

    #[test]
    fn conic_gradient_background_shorthand() {
        let container = apply_container_property(
            "background",
            CssValue::Function(
                "conic-gradient".into(),
                vec![
                    CssValue::Keyword("red".into()),
                    CssValue::Length(0.0, Unit::Deg),
                    CssValue::Keyword("blue".into()),
                    CssValue::Length(180.0, Unit::Deg),
                ],
            ),
        );
        let Background::Gradient(gradient) = container.background else {
            panic!("expected gradient background");
        };
        assert!(matches!(
            gradient.kind,
            GradientKind::Conic {
                angle: 0.0,
                position: (0.5, 0.5)
            }
        ));
        assert_eq!(gradient.stops.len(), 2);
    }

    #[test]
    fn background_image_shorthand_keeps_its_fallback_color() {
        let container = apply_container_property(
            "background",
            CssValue::List(vec![
                CssValue::Function(
                    "url".into(),
                    vec![CssValue::String("/images/caret.svg".into())],
                ),
                CssValue::Keyword("no-repeat".into()),
                CssValue::Keyword("right".into()),
                CssValue::Keyword("center".into()),
                CssValue::Keyword("white".into()),
            ]),
        );
        assert!(matches!(
            container.background,
            Background::Image {
                source,
                image: None,
                color: Color(255, 255, 255, 255)
            } if source == "/images/caret.svg"
        ));
    }

    #[test]
    fn background_none_resolves_to_transparent() {
        let container = apply_container_property("background", CssValue::Keyword("none".into()));
        assert_eq!(container.background, Background::default());
    }

    #[test]
    fn background_image_longhand_preserves_background_color() {
        let mut style = Style::default();
        let mut container = ContainerStyle::default();
        let mut text = TextStyle::default();
        let mut text_flow = TextFlowStyle::default();
        let mut overflow = Overflow::default();
        for (name, value) in [
            (
                "background-color",
                CssValue::Keyword("rebeccapurple".into()),
            ),
            (
                "background-image",
                CssValue::Function(
                    "url".into(),
                    vec![CssValue::String("/images/hero.svg".into())],
                ),
            ),
        ] {
            apply_declaration(
                name,
                &value,
                &mut style,
                &mut container,
                &mut text,
                &mut text_flow,
                &mut overflow,
                ColorScheme::Light,
            )
            .expect("background declaration is accepted");
        }
        assert!(matches!(
            container.background,
            Background::Image {
                source,
                color: Color(102, 51, 153, 255),
                ..
            } if source == "/images/hero.svg"
        ));
    }

    #[test]
    fn scratch_background_geometry_longhands_are_parsed() {
        let mut style = Style::default();
        let mut container = ContainerStyle::default();
        let mut text = TextStyle::default();
        let mut text_flow = TextFlowStyle::default();
        let mut overflow = Overflow::default();
        for (name, value) in [
            ("background-repeat", CssValue::Keyword("no-repeat".into())),
            (
                "background-size",
                CssValue::List(vec![
                    CssValue::Length(624.0, Unit::Px),
                    CssValue::Length(325.0, Unit::Px),
                ]),
            ),
            ("background-position", CssValue::Keyword("right".into())),
        ] {
            apply_declaration(
                name,
                &value,
                &mut style,
                &mut container,
                &mut text,
                &mut text_flow,
                &mut overflow,
                ColorScheme::Light,
            )
            .expect("Scratch background declaration is accepted");
        }
        assert_eq!(container.background_repeat, BackgroundRepeat::NoRepeat);
        assert_eq!(
            container.background_size,
            BackgroundSize::Explicit {
                width: BackgroundDimension::Length(624.0),
                height: BackgroundDimension::Length(325.0),
            }
        );
        assert_eq!(
            container.background_position,
            BackgroundPosition {
                x: BackgroundPositionAxis::End(BackgroundOffset::Zero),
                y: BackgroundPositionAxis::Center(BackgroundOffset::Zero),
            }
        );
    }

    #[test]
    fn scratch_responsive_background_position_is_parsed() {
        let container = apply_container_property(
            "background-position",
            CssValue::List(vec![
                CssValue::Keyword("bottom".into()),
                CssValue::Length(32.0, Unit::Px),
                CssValue::Keyword("right".into()),
                CssValue::Length(50.0, Unit::Percent),
            ]),
        );
        assert_eq!(
            container.background_position,
            BackgroundPosition {
                x: BackgroundPositionAxis::End(BackgroundOffset::Percent(0.5)),
                y: BackgroundPositionAxis::End(BackgroundOffset::Length(32.0)),
            }
        );

        let container =
            apply_container_property("background-size", CssValue::Length(40.0, Unit::Rem));
        assert_eq!(
            container.background_size,
            BackgroundSize::Explicit {
                width: BackgroundDimension::Length(640.0),
                height: BackgroundDimension::Auto,
            }
        );
    }

    #[test]
    fn linear_gradient_calc_position_resolves() {
        let args = vec![
            CssValue::Keyword("red".into()),
            CssValue::Length(0.0, Unit::Percent),
            CssValue::Keyword("blue".into()),
            CssValue::Function(
                "calc".into(),
                vec![
                    CssValue::Length(50.0, Unit::Percent),
                    CssValue::Keyword("+".into()),
                    CssValue::Length(10.0, Unit::Percent),
                ],
            ),
            CssValue::Keyword("green".into()),
            CssValue::Length(100.0, Unit::Percent),
        ];
        let gradient = parse_gradient(
            "linear-gradient",
            &args,
            &TextStyle::default(),
            &TextFlowStyle::default(),
            ColorScheme::Light,
        )
        .expect("gradient parses");
        assert_eq!(gradient.stops[0].position, Some(0.0));
        assert_eq!(gradient.stops[1].position, Some(0.6));
        assert_eq!(gradient.stops[2].position, Some(1.0));
    }

    #[test]
    fn linear_gradient_calc_negative_position_clamps() {
        let args = vec![
            CssValue::Keyword("red".into()),
            CssValue::Function(
                "calc".into(),
                vec![
                    CssValue::Length(50.0, Unit::Percent),
                    CssValue::Keyword("*".into()),
                    CssValue::Number(-1.0),
                ],
            ),
            CssValue::Keyword("blue".into()),
            CssValue::Length(100.0, Unit::Percent),
        ];
        let gradient = parse_gradient(
            "linear-gradient",
            &args,
            &TextStyle::default(),
            &TextFlowStyle::default(),
            ColorScheme::Light,
        )
        .expect("gradient parses");
        assert_eq!(gradient.stops[0].position, Some(0.0));
    }

    #[test]
    fn linear_gradient_calc_zero_position() {
        let args = vec![
            CssValue::Keyword("red".into()),
            CssValue::Function("calc".into(), vec![CssValue::Length(0.0, Unit::Percent)]),
            CssValue::Keyword("blue".into()),
            CssValue::Length(100.0, Unit::Percent),
        ];
        let gradient = parse_gradient(
            "linear-gradient",
            &args,
            &TextStyle::default(),
            &TextFlowStyle::default(),
            ColorScheme::Light,
        )
        .expect("gradient parses");
        assert_eq!(gradient.stops[0].position, Some(0.0));
    }

    #[test]
    fn repeating_gradient_parses_through_background_shorthand() {
        let container = apply_container_property(
            "background",
            CssValue::Function(
                "repeating-linear-gradient".into(),
                vec![
                    CssValue::Length(90.0, Unit::Deg),
                    CssValue::Keyword("red".into()),
                    CssValue::Length(0.0, Unit::Percent),
                    CssValue::Length(10.0, Unit::Percent),
                    CssValue::Keyword("blue".into()),
                    CssValue::Length(10.0, Unit::Percent),
                    CssValue::Length(20.0, Unit::Percent),
                ],
            ),
        );
        let Background::Gradient(gradient) = container.background else {
            panic!("expected gradient background");
        };
        assert!(matches!(
            gradient.kind,
            GradientKind::Linear { angle: 90.0 }
        ));
        assert_eq!(gradient.stops.len(), 4);
    }

    #[test]
    fn gradient_current_color_stop_resolves_to_text_color() {
        let args = vec![
            CssValue::Keyword("currentColor".into()),
            CssValue::Keyword("white".into()),
            CssValue::Keyword("black".into()),
        ];
        let mut text_style = TextStyle::default();
        text_style.color = Color(255, 0, 0, 255);
        let gradient = parse_gradient(
            "linear-gradient",
            &args,
            &text_style,
            &TextFlowStyle::default(),
            ColorScheme::Light,
        )
        .expect("gradient parses");
        assert_eq!(gradient.stops[0].color, Color(255, 0, 0, 255));
    }

    #[test]
    fn normalize_whitespace_collapses_css_whitespace_only() {
        let normal = |s| normalize_whitespace(s, WhiteSpace::Normal);
        assert_eq!(normal("a  b\tc\nd"), "a b c d");
        assert_eq!(normal("a\u{a0}b"), "a\u{a0}b");
        assert_eq!(normal("\u{2007}\u{2007}"), "\u{2007}\u{2007}");
        assert_eq!(normal(" a\n\t b "), " a b ");
    }

    #[test]
    fn normalize_whitespace_normalizes_segment_breaks() {
        let normal = |s| normalize_whitespace(s, WhiteSpace::Normal);
        let pre = |s| normalize_whitespace(s, WhiteSpace::Pre);
        assert_eq!(normal("a\r\nb"), "a b");
        assert_eq!(normal("a\rb"), "a b");
        assert_eq!(normal("a\u{c}b"), "a b");
        assert_eq!(pre("a\r\nb"), "a\nb");
        assert_eq!(pre("a\rb"), "a\nb");
        assert_eq!(pre("a\u{c}b"), "a\nb");
    }

    #[test]
    fn normalize_whitespace_pre_line_drops_trailing_newline() {
        let pre_line = |s| normalize_whitespace(s, WhiteSpace::PreLine);
        assert_eq!(pre_line("a\nb\n"), "a\nb");
        assert_eq!(pre_line("a\nb\n\n"), "a\nb");
        assert_eq!(pre_line("\n"), "");
        assert_eq!(pre_line("a  b\nc\n"), "a b\nc");
    }

    #[test]
    fn normalize_whitespace_pre_wrap_preserves_newlines() {
        assert_eq!(
            normalize_whitespace("a\nb\n", WhiteSpace::PreWrap),
            "a\nb\n"
        );
        assert_eq!(
            normalize_whitespace("a  b\tc", WhiteSpace::BreakSpaces),
            "a  b\tc"
        );
        assert_eq!(normalize_whitespace("a\n\nb", WhiteSpace::Nowrap), "a b");
    }
}
