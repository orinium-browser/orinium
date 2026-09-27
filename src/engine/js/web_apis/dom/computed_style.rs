//! Headless computed-style resolution for `getComputedStyle`.
//!
//! The GUI webview pushes a serialized cascade snapshot into
//! [`JsHost::computed_styles`] after every committed layout, which is the only
//! path that has a laid-out box tree. Headless callers (`examples/acid3.rs`,
//! tests, and any read that happens before the first layout) never commit a
//! layout, so every property used to read back as `""`.
//!
//! [`Resolver`] resolves the cascade directly from the live JS-side tree by
//! reusing the layouter's matcher, cascade ordering, and media filtering, so a
//! headless read agrees with the same page once it is laid out.

use crate::engine::css::parser::Parser as CssParser;
use crate::engine::html::HtmlNodeType;
use crate::engine::layouter::css_resolver::{
    CssResolver, MediaEnvironment, ResolvedStyles, RuleSet, StyleOrigin, append_resolved_styles,
};
use crate::engine::layouter::dom_snapshot::DomSnapshot;
use crate::engine::layouter::style_inspect::collect_matched_rules;
use crate::engine::tree::NodeRef;
use std::collections::HashMap;
use std::rc::Rc;

/// The user-agent stylesheet, applied at `UserAgent` origin so author rules
/// win as they do in the laid-out GUI path.
const USER_AGENT_CSS: &str = include_str!("../../../../../resource/user-agent.css");

/// Identity of an already-resolved element result.
///
/// `dom_version` is the host's single mutation counter, which every JS DOM
/// mutation bumps regardless of which tree it touched, so it invalidates entries
/// for iframes as well as the main document. `css_generation` changes only when
/// the set of `<style>` texts changes, so ordinary DOM mutations do not force
/// the stylesheet to be re-parsed.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct CacheKey {
    pub(crate) dom_version: u64,
    pub(crate) css_generation: u64,
    pub(crate) viewport: (f32, f32),
}

/// The per-tree half of the cascade: a snapshot for selector matching, the
/// stylesheets it was collected alongside, plus the viewport its media queries
/// were evaluated against.
///
/// The sheets are held here so [`collect_style_text`] — a walk of the whole
/// tree — runs when the snapshot is rebuilt rather than on every property read.
struct TreeSnapshot {
    dom_version: u64,
    viewport: (f32, f32),
    snapshot: DomSnapshot,
    sheets: Vec<String>,
}

/// The stylesheet half of the cascade, which only depends on the collected
/// `<style>` texts and the viewport.
struct Rules {
    css_generation: u64,
    viewport: (f32, f32),
    sheets: Vec<String>,
    rules: RuleSet,
}

/// Resolves computed declarations, caching the per-tree snapshot and the rule
/// set.
///
/// A cold resolution is O(tree size) and re-parses every `<style>` element,
/// which is far too slow to redo for each property of each element a script
/// inspects, so the two are cached independently: a DOM mutation only
/// invalidates the snapshot, and re-parsing the CSS only happens once the
/// stylesheets themselves change.
#[derive(Default)]
pub(crate) struct Resolver {
    trees: HashMap<usize, TreeSnapshot>,
    rules: Option<Rules>,
    css_generation: u64,
    elements: HashMap<u64, (CacheKey, Vec<(String, String)>)>,
}

impl Resolver {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Dropped wholesale on mutation, so a script that changes the DOM and then
    /// reads back never sees a pre-mutation value.
    pub(crate) fn invalidate(&mut self) {
        self.trees.clear();
        self.rules = None;
        self.elements.clear();
    }

    /// Resolved declarations for the node at `target` as `(css-name, value)`
    /// pairs in cascade order, with the winner of each property last.
    ///
    /// `root` is the tree that node belongs to (the main document or an iframe's
    /// own tree) and `target` must be a descendant of it.
    pub(crate) fn declarations(
        &mut self,
        root: &NodeRef<HtmlNodeType>,
        target: &NodeRef<HtmlNodeType>,
        dom_id: u64,
        dom_version: u64,
        environment: &MediaEnvironment,
    ) -> Vec<(String, String)> {
        let sheets = self
            .settle_snapshot(root, dom_version, environment)
            .map(|tree| tree.sheets.clone());
        let Some(sheets) = sheets else {
            return Vec::new();
        };
        let css_generation = self.settle_css(sheets, environment);
        let key = CacheKey {
            dom_version,
            css_generation,
            viewport: environment.viewport(),
        };
        if let Some((cached_key, declarations)) = self.elements.get(&dom_id)
            && *cached_key == key
        {
            return declarations.clone();
        }

        let root_key = Rc::as_ptr(root) as usize;
        let Some(target_index) = preorder_index(root, target) else {
            return Vec::new();
        };
        let rules = &self.rules.as_ref().expect("settle_css ran").rules;
        let snapshot = &self.trees[&root_key].snapshot;
        let inline_style_attr = snapshot
            .node(target_index)
            .kind
            .get_attr("style")
            .map(str::to_string);
        let inline_style_attr = inline_style_attr.as_deref();
        let declarations: Vec<(String, String)> =
            collect_matched_rules(snapshot, target_index, rules, inline_style_attr)
                .into_iter()
                .flat_map(|rule| rule.declarations)
                .filter(|declaration| declaration.applied)
                .map(|declaration| (declaration.name, declaration.value.to_string()))
                .collect();
        self.elements.insert(dom_id, (key, declarations.clone()));
        declarations
    }

    /// Reuses the cached rule set when the stylesheets are unchanged, and
    /// returns the generation the resulting rules belong to.
    fn settle_css(&mut self, sheets: Vec<String>, environment: &MediaEnvironment) -> u64 {
        let viewport = environment.viewport();
        let current = self
            .rules
            .as_ref()
            .filter(|rules| rules.sheets == sheets && rules.viewport == viewport);
        if let Some(rules) = current {
            return rules.css_generation;
        }
        self.css_generation += 1;
        let generation = self.css_generation;
        self.rules = Some(Rules {
            css_generation: generation,
            viewport,
            rules: build_rule_set(&sheets, environment),
            sheets,
        });
        // A new stylesheet invalidates every element result resolved so far.
        self.elements.clear();
        generation
    }
    /// Reuses the cached snapshot for `root` when neither the tree nor the
    /// viewport changed, and otherwise rebuilds it along with the stylesheets
    /// the tree currently carries.
    fn settle_snapshot(
        &mut self,
        root: &NodeRef<HtmlNodeType>,
        dom_version: u64,
        environment: &MediaEnvironment,
    ) -> Option<&TreeSnapshot> {
        let viewport = environment.viewport();
        let root_key = Rc::as_ptr(root) as usize;
        let reusable = self
            .trees
            .get(&root_key)
            .is_some_and(|tree| tree.dom_version == dom_version && tree.viewport == viewport);
        if !reusable {
            self.trees.insert(
                root_key,
                TreeSnapshot {
                    dom_version,
                    viewport,
                    // Node ids are not needed to match selectors, and labelling
                    // them would cost a lookup per node on every rebuild.
                    snapshot: DomSnapshot::from_mirror(root, &HashMap::new()),
                    sheets: collect_style_text(root),
                },
            );
        }
        self.trees.get(&root_key)
    }
}

/// Index of `target` in `root`'s pre-order walk.
///
/// [`DomSnapshot::from_mirror`] stores nodes in pre-order with `id == index`,
/// so the same walk locates a live node in the snapshot. Children are pushed
/// reversed so they are visited left to right.
fn preorder_index(root: &NodeRef<HtmlNodeType>, target: &NodeRef<HtmlNodeType>) -> Option<u32> {
    let mut stack = vec![Rc::clone(root)];
    let mut next = 0u32;
    while let Some(node) = stack.pop() {
        let index = next;
        next += 1;
        if Rc::ptr_eq(&node, target) {
            return Some(index);
        }
        let mut children = node.borrow().children().to_vec();
        children.reverse();
        stack.extend(children);
    }
    None
}

/// Builds the cascade for the user-agent sheet plus the stylesheets collected by
/// the caller, in document order.
/// The user-agent sheet's resolved declarations.
///
/// The sheet is compiled in, so it never changes; parsing it once keeps a
/// stylesheet edit from redoing that work.
fn user_agent_declarations() -> &'static ResolvedStyles {
    static CACHE: std::sync::OnceLock<ResolvedStyles> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        let mut resolved = Vec::new();
        if let Ok(stylesheet) = CssParser::new(USER_AGENT_CSS).parse() {
            append_resolved_styles(
                &mut resolved,
                CssResolver::resolve_with_origin(&stylesheet, StyleOrigin::UserAgent),
            );
        }
        resolved
    })
}

fn build_rule_set(sheets: &[String], environment: &MediaEnvironment) -> RuleSet {
    let mut resolved = user_agent_declarations().clone();
    for css in sheets {
        if let Ok(stylesheet) = CssParser::new(css).parse() {
            append_resolved_styles(&mut resolved, CssResolver::resolve(&stylesheet));
        }
    }
    RuleSet::from_declarations(&resolved, environment)
}

/// Collects the CSS text of every `<style>` element under `root` in document
/// order. `<style>` content is a raw text child, so only direct text children
/// contribute.
fn collect_style_text(root: &NodeRef<HtmlNodeType>) -> Vec<String> {
    let mut sheets = Vec::new();
    let mut stack = vec![Rc::clone(root)];
    while let Some(node) = stack.pop() {
        let is_style = {
            let node = node.borrow();
            node.value
                .tag_name()
                .is_some_and(|tag| tag.eq_ignore_ascii_case("style"))
        };
        if is_style {
            let node = node.borrow();
            let css = node
                .children()
                .iter()
                .filter_map(|child| match &child.borrow().value {
                    HtmlNodeType::Text(text) => Some(text.clone()),
                    _ => None,
                })
                .collect::<String>();
            if !css.trim().is_empty() {
                sheets.push(css);
            }
            continue;
        }
        // Reversed so the pop order preserves document order.
        let mut children = node.borrow().children().to_vec();
        children.reverse();
        stack.extend(children);
    }
    sheets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::html::Parser as HtmlParser;
    use crate::engine::layouter::types::ColorScheme;

    fn environment() -> MediaEnvironment {
        MediaEnvironment::new((1024.0, 768.0), ColorScheme::Light)
    }

    fn parse(html: &str) -> NodeRef<HtmlNodeType> {
        HtmlParser::new(html).parse().root
    }

    /// First element under `root` with the given tag name.
    fn by_tag(root: &NodeRef<HtmlNodeType>, wanted: &str) -> NodeRef<HtmlNodeType> {
        let mut found = None;
        fn walk(
            node: &NodeRef<HtmlNodeType>,
            wanted: &str,
            found: &mut Option<NodeRef<HtmlNodeType>>,
        ) {
            if found.is_some() {
                return;
            }
            if node
                .borrow()
                .value
                .tag_name()
                .is_some_and(|tag| tag.eq_ignore_ascii_case(wanted))
            {
                *found = Some(Rc::clone(node));
                return;
            }
            for child in node.borrow().children().to_vec() {
                walk(&child, wanted, found);
            }
        }
        walk(root, wanted, &mut found);
        found.expect("element with that tag is present")
    }

    /// The winning value for `name`, mirroring how the `CSSStyleDeclaration`
    /// wrapper resolves a property against a serialized declaration list.
    fn value_of(declarations: &[(String, String)], name: &str) -> String {
        declarations
            .iter()
            .rev()
            .find(|(property, _)| property == name)
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    }

    fn declarations_for(
        resolver: &mut Resolver,
        root: &NodeRef<HtmlNodeType>,
        node: &NodeRef<HtmlNodeType>,
        dom_version: u64,
    ) -> Vec<(String, String)> {
        resolver.declarations(root, node, 1, dom_version, &environment())
    }

    #[test]
    fn resolves_author_rule_from_a_style_element() {
        let root = parse(
            "<html><head><style>p { z-index: 3; }</style></head>\
             <body><p id=\"target\">x</p></body></html>",
        );
        let mut resolver = Resolver::new();
        let paragraph = by_tag(&root, "p");
        let declarations = declarations_for(&mut resolver, &root, &paragraph, 1);
        assert_eq!(value_of(&declarations, "z-index"), "3");
    }

    #[test]
    fn specificity_beats_source_order() {
        let root = parse(
            "<html><head><style>p { color: red; } #target { color: lime; }</style></head>\
             <body><p id=\"target\">x</p></body></html>",
        );
        let mut resolver = Resolver::new();
        let paragraph = by_tag(&root, "p");
        let declarations = declarations_for(&mut resolver, &root, &paragraph, 1);
        assert_eq!(value_of(&declarations, "color"), "lime");
    }

    #[test]
    fn inline_style_attribute_wins_over_selector() {
        let root = parse(
            "<html><head><style>p { color: red; }</style></head>\
             <body><p id=\"target\" style=\"color: blue\">x</p></body></html>",
        );
        let mut resolver = Resolver::new();
        let paragraph = by_tag(&root, "p");
        let declarations = declarations_for(&mut resolver, &root, &paragraph, 1);
        assert_eq!(value_of(&declarations, "color"), "blue");
    }

    #[test]
    fn later_style_sheet_wins_at_equal_specificity() {
        let root = parse(
            "<html><head><style>p { color: red; }</style>\
             <style>p { color: lime; }</style></head>\
             <body><p id=\"target\">x</p></body></html>",
        );
        let mut resolver = Resolver::new();
        let paragraph = by_tag(&root, "p");
        let declarations = declarations_for(&mut resolver, &root, &paragraph, 1);
        assert_eq!(value_of(&declarations, "color"), "lime");
    }

    #[test]
    fn author_rule_beats_user_agent_rule() {
        let root = parse(
            "<html><head><style>input { display: inline; }</style></head>\
             <body><input></body></html>",
        );
        let mut resolver = Resolver::new();
        let input = by_tag(&root, "input");
        let declarations = declarations_for(&mut resolver, &root, &input, 1);
        assert_eq!(value_of(&declarations, "display"), "inline");
    }

    #[test]
    fn unmatched_element_reports_no_declarations() {
        let root = parse(
            "<html><head><style>div { color: red; }</style></head>\
             <body><p>x</p></body></html>",
        );
        let mut resolver = Resolver::new();
        let paragraph = by_tag(&root, "p");
        let declarations = declarations_for(&mut resolver, &root, &paragraph, 1);
        assert!(value_of(&declarations, "color").is_empty());
    }

    #[test]
    fn descendant_selector_matches_through_intermediate_ancestors() {
        let root = parse(
            "<html><head><style>body .box span { color: teal; }</style></head>\
             <body><div class=\"box\"><span id=\"target\">x</span></div></body></html>",
        );
        let mut resolver = Resolver::new();
        let span = by_tag(&root, "span");
        let declarations = declarations_for(&mut resolver, &root, &span, 1);
        assert_eq!(value_of(&declarations, "color"), "teal");
    }

    #[test]
    fn repeated_reads_hit_the_cache_and_agree() {
        let root = parse(
            "<html><head><style>p { color: red; }</style></head>\
             <body><p id=\"target\">x</p><p>y</p></body></html>",
        );
        let mut resolver = Resolver::new();
        let first = by_tag(&root, "p");
        let declarations = declarations_for(&mut resolver, &root, &first, 1);
        let again = declarations_for(&mut resolver, &root, &first, 1);
        assert_eq!(declarations, again);
    }

    #[test]
    fn a_new_version_re_reads_the_tree_instead_of_a_stale_value() {
        let root = parse(
            "<html><head><style>p { color: red; }</style></head>\
             <body><p id=\"target\">x</p></body></html>",
        );
        let mut resolver = Resolver::new();
        let paragraph = by_tag(&root, "p");
        let before = declarations_for(&mut resolver, &root, &paragraph, 1);
        assert_eq!(value_of(&before, "color"), "red");

        // Rewrite the stylesheet and bump the version the way a mutation does.
        let style = by_tag(&root, "style");
        let text_node = Rc::clone(&style.borrow().children()[0]);
        text_node.borrow_mut().value = HtmlNodeType::Text("p { color: lime; }".to_string());
        let after = declarations_for(&mut resolver, &root, &paragraph, 2);
        assert_eq!(value_of(&after, "color"), "lime");
    }
}
