//! Read-only inspection queries for the DevTools bridge.
//!
//! Answers `__orinium_devtools` requests against the live document tree,
//! using stable node ids from [`DomIdRegistry`].

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::{Rc, Weak};

use serde_json::{Value, json};

use crate::engine::css::values::CssValue;
use crate::engine::html::HtmlNodeType;
use crate::engine::layouter::css_resolver::{MediaEnvironment, RuleSet};
use crate::engine::layouter::dom_snapshot::NodeId;
use crate::engine::layouter::style_inspect::{MatchedRule, collect_matched_rules};
use crate::engine::layouter::types::InfoNode;
use crate::engine::tree::{NodeRef, TreeNode};
use ui_layout::{Length, LengthOrAuto, Rect};

use super::WebView;

/// Stable inspector-facing DOM ids over the live document tree.
///
/// Ids are assigned lazily as nodes are first visited and stay valid while
/// the underlying node lives; nodes dropped from the tree only leave stale
/// weak entries behind. Navigation clears the registry entirely.
#[derive(Default)]
pub(super) struct DomIdRegistry {
    /// Id per live node address.
    ids: HashMap<usize, u64>,
    /// Node per id, weak so dropped subtrees do not leak here.
    nodes: HashMap<u64, Weak<RefCell<TreeNode<HtmlNodeType>>>>,
    next_id: u64,
}

impl DomIdRegistry {
    /// Drops every registration; used when the page navigates away.
    pub(super) fn clear(&mut self) {
        self.ids.clear();
        self.nodes.clear();
        self.next_id = 0;
    }

    /// Returns the id previously assigned to `node`, assigning a fresh one
    /// if this is its first visit.
    fn intern(&mut self, node: &NodeRef<HtmlNodeType>) -> u64 {
        let address = Rc::as_ptr(node) as usize;
        if let Some(&id) = self.ids.get(&address) {
            return id;
        }
        self.next_id += 1;
        let id = self.next_id;
        self.ids.insert(address, id);
        self.nodes.insert(id, Rc::downgrade(node));
        id
    }

    /// Returns the node registered under `id`, if it is still alive.
    fn node(&self, id: u64) -> Option<NodeRef<HtmlNodeType>> {
        self.nodes.get(&id).and_then(Weak::upgrade)
    }

    /// Registers every node in `node`'s subtree and serializes it.
    fn serialize_subtree(&mut self, node: &NodeRef<HtmlNodeType>) -> Value {
        let id = self.intern(node);
        let borrowed = node.borrow();
        match &borrowed.value {
            HtmlNodeType::Document => json!({
                "id": id,
                "type": "document",
                "children": Self::serialize_children(self, &borrowed),
            }),
            HtmlNodeType::Element {
                tag_name,
                attributes,
            } => json!({
                "id": id,
                "type": "element",
                "tag": tag_name,
                "attributes": attributes
                    .iter()
                    .map(|attr| json!([attr.name, attr.value]))
                    .collect::<Vec<_>>(),
                "children": Self::serialize_children(self, &borrowed),
            }),
            HtmlNodeType::Text(text) => json!({
                "id": id,
                "type": "text",
                "text": text,
            }),
            HtmlNodeType::Comment(comment) => json!({
                "id": id,
                "type": "comment",
                "text": comment,
            }),
            HtmlNodeType::Doctype { .. } => json!({
                "id": id,
                "type": "doctype",
            }),
            HtmlNodeType::InvalidNode(..) => json!({
                "id": id,
                "type": "invalid",
            }),
        }
    }

    fn serialize_children(
        registry: &mut DomIdRegistry,
        borrowed: &std::cell::Ref<'_, TreeNode<HtmlNodeType>>,
    ) -> Vec<Value> {
        // Collect child references first so each node's borrow is released
        // before its subtree is serialized recursively.
        let children = borrowed.children().to_vec();
        children
            .iter()
            .map(|child| registry.serialize_subtree(child))
            .collect()
    }
}

/// Maps an inspector dom id to the matching snapshot node of the current
/// layout, returning the node id plus its inline `style` attribute.
///
/// The snapshot's live references (`layout_dom_refs`, indexed by snapshot
/// node id) bridge the two id spaces: the registry hands out ids over the
/// live tree while the cascade replay walks the snapshot the layout was
/// actually built from.
fn style_inspection_target(
    webview: &WebView,
    dom_id: u64,
) -> Result<(NodeId, Option<String>), String> {
    let info = webview
        .docment_info
        .as_ref()
        .ok_or_else(|| "no document".to_string())?;
    let node = webview
        .inspector_ids
        .borrow()
        .node(dom_id)
        .ok_or_else(|| format!("unknown domId: {dom_id}"))?;
    let address = Rc::as_ptr(&node) as usize;

    let cache = webview
        .snapshot_cache
        .as_ref()
        .ok_or_else(|| "no layout snapshot yet".to_string())?;
    if cache.dom_version != info.dom.version() {
        return Err("layout snapshot is stale".to_string());
    }

    let node_id = webview
        .layout_dom_refs
        .iter()
        .position(|weak| {
            weak.upgrade()
                .is_some_and(|live| Rc::as_ptr(&live) as usize == address)
        })
        .ok_or_else(|| format!("domId {dom_id} is not part of the current layout"))?
        as NodeId;

    let kind = &cache.snapshot.node(node_id).kind;
    if kind.tag_name().is_none() {
        return Err(format!("domId {dom_id} is not an element"));
    }
    Ok((node_id, kind.get_attr("style").map(str::to_string)))
}

/// One style-inspection answer: every matched rule plus the computed map of
/// winning properties.
type StyleInspection = (Vec<MatchedRule>, Vec<(String, CssValue)>);

/// Replays the cascade for `dom_id`: every matched rule plus the computed
/// (winning) property map.
fn inspect_styles(webview: &WebView, dom_id: u64) -> Result<StyleInspection, String> {
    let (node_id, inline_style) = style_inspection_target(webview, dom_id)?;
    let cache = webview.snapshot_cache.as_ref().expect("checked above");

    let media_environment = MediaEnvironment::new(webview.viewport, webview.system_color_scheme);
    let rule_set = RuleSet::from_declarations(&webview.resolved_styles, &media_environment);
    let rules = collect_matched_rules(&cache.snapshot, node_id, &rule_set, inline_style.as_deref());

    // Computed view: the applied declaration per property. Custom property
    // inheritance is not replayed here yet, so var()-dependent winners keep
    // their raw value.
    let mut computed = BTreeMap::new();
    for rule in &rules {
        for declaration in &rule.declarations {
            if declaration.applied {
                computed.insert(declaration.name.clone(), declaration.value.clone());
            }
        }
    }

    Ok((rules, computed.into_iter().collect()))
}

fn serialize_origin(origin: crate::engine::layouter::css_resolver::StyleOrigin) -> &'static str {
    match origin {
        crate::engine::layouter::css_resolver::StyleOrigin::UserAgent => "user-agent",
        crate::engine::layouter::css_resolver::StyleOrigin::Author => "author",
    }
}

// ── Box model / layout geometry ─────────────────────────────────────────────

/// A layout node located by dom id, together with its page-space boxes.
struct LayoutTarget<'a> {
    layout: &'a ui_layout::LayoutNode,
    info: &'a InfoNode,
    border_box: Rect,
    padding_box: Rect,
    content_box: Rect,
}

/// Finds the layout node whose paired [`InfoNode`] carries `dom_id`, walking
/// the (layout, info) trees in lockstep and accumulating parent content
/// origins exactly like the JS metrics collector, so the returned boxes are
/// page-space absolute.
fn find_layout_target<'a>(
    layout: &'a ui_layout::LayoutNode,
    info: &'a InfoNode,
    dom_id: NodeId,
    origin: (f32, f32),
) -> Option<LayoutTarget<'a>> {
    let first = layout.layout_box.iter().next()?;
    let child_origin = (
        origin.0 + first.content_box.x,
        origin.1 + first.content_box.y,
    );

    if info.dom_id == Some(dom_id) {
        let offset = |box_rect: &ui_layout::Rect| Rect {
            x: origin.0 + box_rect.x,
            y: origin.1 + box_rect.y,
            width: box_rect.width,
            height: box_rect.height,
        };
        return Some(LayoutTarget {
            layout,
            info,
            border_box: offset(&first.border_box),
            padding_box: offset(&first.padding_box),
            content_box: offset(&first.content_box),
        });
    }

    for (child_layout, child_info) in layout.children.iter().zip(&info.children) {
        if let Some(child_layout) = child_layout.node()
            && let Some(found) = find_layout_target(child_layout, child_info, dom_id, child_origin)
        {
            return Some(found);
        }
    }
    None
}

/// Resolves `dom_id` to its laid-out node, running the box pass first if the
/// pending result has not been painted yet (the builder hands back trees whose
/// boxes are only filled in by [`ui_layout::LayoutEngine::layout`] at draw
/// time, so an inspection between builds must lay out once itself).
fn layout_target(webview: &mut WebView, dom_id: u64) -> Result<(NodeId, LayoutTarget<'_>), String> {
    let (node_id, _) = style_inspection_target(webview, dom_id)?;
    let (layout, _) = webview
        .layout_and_info
        .as_mut()
        .ok_or_else(|| "no layout yet".to_string())?;
    if matches!(layout.layout_box, ui_layout::LayoutBox::None) {
        ui_layout::LayoutEngine::layout(layout, webview.viewport.0, webview.viewport.1);
        if crate::engine::layouter::constrain_auto_grid_track_items(layout) {
            ui_layout::LayoutEngine::layout(layout, webview.viewport.0, webview.viewport.1);
        }
    }
    let (layout, info) = webview.layout_and_info.as_ref().expect("borrowed above");
    find_layout_target(layout, info, node_id, (0.0, 0.0))
        .map(|target| (node_id, target))
        .ok_or_else(|| format!("domId {dom_id} has no layout box"))
}

/// Renders a length as DevTools-style text: bare numbers for px, `auto` for
/// auto margins, unit-suffixed otherwise.
fn length_text(value: &LengthOrAuto) -> String {
    match value {
        LengthOrAuto::Auto => "auto".to_string(),
        LengthOrAuto::Length(length) => length_text_inner(length),
    }
}

fn length_text_inner(length: &Length) -> String {
    match length {
        Length::Px(value) => format!("{value}"),
        Length::Percent(value) => format!("{value}%"),
        Length::Vw(value) => format!("{value}vw"),
        Length::Vh(value) => format!("{value}vh"),
        other => format!("{other:?}"),
    }
}

/// Ring thicknesses between two nested boxes: `[top, right, bottom, left]`.
fn ring_between(outer: &Rect, inner: &Rect) -> [f32; 4] {
    [
        inner.y - outer.y,
        outer.right() - inner.right(),
        outer.bottom() - inner.bottom(),
        inner.x - outer.x,
    ]
}

fn serialize_box_model(target: &LayoutTarget<'_>) -> Value {
    let spacing = &target.layout.style.spacing;
    json!({
        "margin": [
            length_text(&spacing.margin_top),
            length_text(&spacing.margin_right),
            length_text(&spacing.margin_bottom),
            length_text(&spacing.margin_left),
        ],
        // Border and padding rings come from the laid-out geometry so the
        // diagram always matches what is on screen.
        "border": ring_between(&target.border_box, &target.padding_box),
        "padding": ring_between(&target.padding_box, &target.content_box),
        "content": [target.content_box.width, target.content_box.height],
        "position": [target.border_box.x, target.border_box.y],
        "size": [target.border_box.width, target.border_box.height],
    })
}

/// Curated layout summary for a node: display/position kind, declared size,
/// scroll state and child count.
fn serialize_layout_info(info: &InfoNode, target: &LayoutTarget<'_>) -> Value {
    let style = &target.layout.style;
    let size = &style.size;
    let scroll = info.kind.scroll_offsets();
    json!({
        "display": format!("{:?}", style.display.inner),
        "position": format!("{:?}", style.position.kind),
        "width": length_text(&size.width),
        "height": length_text(&size.height),
        "children": target.layout.children.len(),
        "scroll": [scroll.0, scroll.1],
    })
}

/// Answers a DevTools inspection query against this page's state.
pub(super) fn handle(webview: &mut WebView, method: &str, params: &str) -> Result<Value, String> {
    let Some(info) = webview.docment_info.as_ref() else {
        return Err("no document".to_string());
    };
    match method {
        "getVersion" => Ok(json!({
            // The DOM version resets when a script mutation commits a rebuilt
            // tree, so pair it with the monotonic layout version to detect
            // every kind of change from the frontend's polling loop.
            "domVersion": info.dom.version(),
            "layoutVersion": webview.layout_applied_version,
        })),
        "getDocument" => {
            let root = Rc::clone(&info.dom.root);
            Ok(webview.inspector_ids.borrow_mut().serialize_subtree(&root))
        }
        "getAttributes" => {
            let parsed: Value = serde_json::from_str(params).map_err(|e| e.to_string())?;
            let dom_id = parsed
                .get("domId")
                .and_then(Value::as_u64)
                .ok_or_else(|| "params must include a numeric domId".to_string())?;
            let node = webview
                .inspector_ids
                .borrow()
                .node(dom_id)
                .ok_or_else(|| format!("unknown domId: {dom_id}"))?;
            let borrowed = node.borrow();
            let HtmlNodeType::Element { attributes, .. } = &borrowed.value else {
                return Err(format!("domId {dom_id} is not an element"));
            };
            Ok(json!({
                "id": dom_id,
                "tag": borrowed.value.tag_name(),
                "attributes": attributes
                    .iter()
                    .map(|attr| json!([attr.name, attr.value]))
                    .collect::<Vec<_>>(),
            }))
        }
        "getMatchedRules" | "getComputedStyle" => {
            let parsed: Value = serde_json::from_str(params).map_err(|e| e.to_string())?;
            let dom_id = parsed
                .get("domId")
                .and_then(Value::as_u64)
                .ok_or_else(|| "params must include a numeric domId".to_string())?;
            let (rules, computed) = inspect_styles(webview, dom_id)?;

            if method == "getComputedStyle" {
                return Ok(json!({
                    "id": dom_id,
                    "properties": computed
                        .iter()
                        .map(|(name, value)| json!({ "name": name, "value": value.to_string() }))
                        .collect::<Vec<_>>(),
                }));
            }

            Ok(json!({
                "id": dom_id,
                "rules": rules
                    .iter()
                    .map(|rule| {
                        json!({
                            "selector": rule.selector,
                            "origin": serialize_origin(rule.origin),
                            "inline": rule.inline,
                            "declarations": rule
                                .declarations
                                .iter()
                                .map(|declaration| json!({
                                    "name": declaration.name,
                                    "value": declaration.value.to_string(),
                                    "important": declaration.important,
                                    "applied": declaration.applied,
                                }))
                                .collect::<Vec<_>>(),
                        })
                    })
                    .collect::<Vec<_>>(),
            }))
        }
        "getBoxModel" | "getLayoutInfo" => {
            let parsed: Value = serde_json::from_str(params).map_err(|e| e.to_string())?;
            let dom_id = parsed
                .get("domId")
                .and_then(Value::as_u64)
                .ok_or_else(|| "params must include a numeric domId".to_string())?;
            let (node_id, target) = layout_target(webview, dom_id)?;
            if method == "getLayoutInfo" {
                return Ok(json!({
                    "id": node_id,
                    "info": serialize_layout_info(target.info, &target),
                }));
            }
            Ok(json!({
                "id": dom_id,
                "model": serialize_box_model(&target),
            }))
        }
        _ => Err(format!("unknown method: {method}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::html::parser::Parser as HtmlParser;

    fn parse(html: &str) -> NodeRef<HtmlNodeType> {
        Rc::clone(&HtmlParser::new(html).parse().root)
    }

    fn find_by_tag<'a>(node: &'a Value, tag: &str) -> Option<&'a Value> {
        if node.get("tag").and_then(Value::as_str) == Some(tag) {
            return Some(node);
        }
        node.get("children")
            .and_then(Value::as_array)
            .and_then(|children| children.iter().find_map(|child| find_by_tag(child, tag)))
    }

    #[test]
    fn serialization_assigns_stable_unique_ids() {
        let root = parse("<div><p>hello</p><p>world</p></div>");
        let mut registry = DomIdRegistry::default();

        let document = registry.serialize_subtree(&root);
        let again = registry.serialize_subtree(&root);

        assert_eq!(document, again);
        assert_eq!(document["type"], "document");

        fn collect_ids(value: &Value, ids: &mut Vec<u64>) {
            if let Some(id) = value.get("id").and_then(Value::as_u64) {
                ids.push(id);
            }
            for child in value
                .get("children")
                .and_then(Value::as_array)
                .unwrap_or(&Vec::new())
            {
                collect_ids(child, ids);
            }
        }
        let mut ids = Vec::new();
        collect_ids(&document, &mut ids);
        let unique: std::collections::HashSet<u64> = ids.iter().copied().collect();
        assert_eq!(unique.len(), ids.len(), "ids must be unique");
    }

    #[test]
    fn serialization_includes_tags_attributes_and_text() {
        let root = parse(r#"<body><a href="https://example.test">link</a></body>"#);
        let mut registry = DomIdRegistry::default();

        let document = registry.serialize_subtree(&root);
        let anchor = find_by_tag(&document, "a").expect("anchor in tree");
        assert_eq!(anchor["type"], "element");
        assert_eq!(anchor["attributes"][0][0], "href");
        assert_eq!(anchor["attributes"][0][1], "https://example.test");
        assert_eq!(anchor["children"][0]["type"], "text");
        assert_eq!(anchor["children"][0]["text"], "link");
    }

    #[test]
    fn attributes_lookup_resolves_interned_elements_only() {
        let root = parse(r#"<div id="target" class="box"></div>"#);
        let mut registry = DomIdRegistry::default();
        let document = registry.serialize_subtree(&root);
        let div = find_by_tag(&document, "div").expect("div in tree");
        let div_id = div["id"].as_u64().unwrap();

        let text_node_id = div["id"].as_u64().unwrap() + 1000;
        assert!(registry.node(text_node_id).is_none());

        let node = registry.node(div_id).unwrap();
        let borrowed = node.borrow();
        let HtmlNodeType::Element { attributes, .. } = &borrowed.value else {
            panic!("expected an element");
        };
        assert_eq!(attributes.len(), 2);
        assert_eq!(
            attributes
                .iter()
                .find(|attr| attr.name == "class")
                .map(|attr| attr.value.as_str()),
            Some("box")
        );
    }
}
