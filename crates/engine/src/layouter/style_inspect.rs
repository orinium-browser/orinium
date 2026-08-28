//! Read-only CSS cascade inspection for the DevTools Styles panel.
//!
//! Reports matching rules, including overridden declarations.

use std::collections::HashMap;

use super::builder::{element_info, element_sibling_infos};
use super::css_resolver::{ResolvedDeclaration, RuleSet, StyleOrigin, resolve_inline_style};
use super::dom_snapshot::{DomSnapshot, NodeId};
use crate::css::matcher::ElementChain;
use crate::css::values::CssValue;

/// One property declaration as shown in the Styles panel.
#[derive(Debug, Clone)]
pub struct InspectedDeclaration {
    pub name: String,
    pub value: CssValue,
    pub important: bool,
    /// Whether this declaration is the cascade winner for its property.
    pub applied: bool,
}

/// A rule (or the inline `style` attribute) whose declarations were cascaded.
#[derive(Debug, Clone)]
pub struct MatchedRule {
    /// Rendered selector text, or `"element.style"` for inline styles.
    pub selector: String,
    pub origin: StyleOrigin,
    /// Whether this rule is the synthetic inline-style entry.
    pub inline: bool,
    pub declarations: Vec<InspectedDeclaration>,
}

/// Total ordering mirroring the builder's cascade semantics: `!important`
/// first, then origin (`Author` ranks above `UserAgent`), then inline styles
/// over non-important stylesheet rules, then specificity and source order.
///
/// Inline entries use source orders starting at [`INLINE_ORDER_BASE`] so that
/// repeated inline declarations resolve last-wins while still ranking above
/// every normal stylesheet declaration.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CascadeKey {
    important: bool,
    origin: StyleOrigin,
    inline: bool,
    specificity: (u32, u32, u32),
    order: usize,
}

const INLINE_ORDER_BASE: usize = usize::MAX / 2;

impl CascadeKey {
    fn from_declaration(declaration: &ResolvedDeclaration) -> Self {
        Self {
            important: declaration.important,
            origin: declaration.origin,
            inline: false,
            specificity: declaration.specificity,
            order: declaration.order,
        }
    }
}

struct Pending {
    name: String,
    key: CascadeKey,
    rule: usize,
    declaration: usize,
}

/// Collects every rule matching `target` and flags winning declarations.
///
/// `inline_style_attr` is the target's raw `style` attribute value, taken
/// from the same snapshot the layout was built from; pass `None` when the
/// element has no attribute. `target` must be an element node.
pub fn collect_matched_rules(
    snapshot: &DomSnapshot,
    target: NodeId,
    rule_set: &RuleSet<'_>,
    inline_style_attr: Option<&str>,
) -> Vec<MatchedRule> {
    let Some(chain) = chain_for_node(snapshot, target) else {
        return Vec::new();
    };
    let Some(element) = chain.first() else {
        return Vec::new();
    };

    // The synthetic inline entry leads the output like browser DevTools do.
    let mut rules: Vec<MatchedRule> = Vec::new();
    let mut pendings: Vec<Pending> = Vec::new();

    if let Some(style_attr) = inline_style_attr {
        let inline_declarations = resolve_inline_style(style_attr);
        if !inline_declarations.is_empty() {
            let mut declarations = Vec::with_capacity(inline_declarations.len());
            for (offset, (name, value, important)) in inline_declarations.into_iter().enumerate() {
                pendings.push(Pending {
                    name: name.clone(),
                    key: CascadeKey {
                        important,
                        origin: StyleOrigin::Author,
                        inline: true,
                        specificity: (u32::MAX, 0, 0),
                        order: INLINE_ORDER_BASE + offset,
                    },
                    rule: 0,
                    declaration: declarations.len(),
                });
                declarations.push(InspectedDeclaration {
                    name,
                    value,
                    important,
                    applied: false,
                });
            }
            rules.push(MatchedRule {
                selector: "element.style".to_string(),
                origin: StyleOrigin::Author,
                inline: true,
                declarations,
            });
        }
    }

    let mut rule_index: HashMap<(String, StyleOrigin), usize> = HashMap::new();
    for declaration in rule_set.query_candidates(element) {
        if !declaration.selector.matches(&chain) {
            continue;
        }
        let selector = declaration.selector.to_string();
        let key = (selector, declaration.origin);
        let rule = *rule_index.entry(key).or_insert_with(|| {
            rules.push(MatchedRule {
                selector: declaration.selector.to_string(),
                origin: declaration.origin,
                inline: false,
                declarations: Vec::new(),
            });
            rules.len() - 1
        });
        pendings.push(Pending {
            name: declaration.name.clone(),
            key: CascadeKey::from_declaration(declaration),
            rule,
            declaration: rules[rule].declarations.len(),
        });
        rules[rule].declarations.push(InspectedDeclaration {
            name: declaration.name.clone(),
            value: declaration.value.clone(),
            important: declaration.important,
            applied: false,
        });
    }

    mark_cascade_winners(&mut rules, &pendings);
    rules
}

/// Flags the highest-ranking declaration per property name as applied.
fn mark_cascade_winners(rules: &mut [MatchedRule], pendings: &[Pending]) {
    let mut winners: HashMap<&str, (CascadeKey, usize)> = HashMap::new();
    for (index, pending) in pendings.iter().enumerate() {
        match winners.get(pending.name.as_str()) {
            Some((existing_key, _)) if *existing_key >= pending.key => {}
            _ => {
                winners.insert(&pending.name, (pending.key.clone(), index));
            }
        }
    }
    for (_, (_, index)) in winners {
        let pending = &pendings[index];
        rules[pending.rule].declarations[pending.declaration].applied = true;
    }
}

/// Rebuilds the [`ElementChain`] the builder would have walked with when
/// styling `target`: ancestor elements outermost-first, each carrying the
/// sibling indexes its structural pseudo-classes rely on.
fn chain_for_node(snapshot: &DomSnapshot, target: NodeId) -> Option<ElementChain> {
    let root = *snapshot.roots().first()?;
    let mut path = Vec::new();
    find_path(snapshot, root, target, &mut path)?;

    let mut elements = Vec::with_capacity(path.len());
    for (depth, &node_id) in path.iter().enumerate() {
        let info = if depth == 0 {
            element_info(&snapshot.node(node_id).kind)
        } else {
            let siblings = snapshot.children(path[depth - 1]);
            let position = siblings.iter().position(|&sibling| sibling == node_id)?;
            element_sibling_infos(snapshot, siblings)[position].clone()
        };
        elements.push(info);
    }

    // `from_vec` expects innermost-first; non-element ancestors (document,
    // text) simply do not contribute chain links, matching the builder.
    elements.reverse();
    Some(ElementChain::from_vec(
        elements.into_iter().flatten().collect(),
    ))
}

fn find_path(
    snapshot: &DomSnapshot,
    current: NodeId,
    target: NodeId,
    path: &mut Vec<NodeId>,
) -> Option<()> {
    path.push(current);
    if current == target {
        return Some(());
    }
    for &child in snapshot.children(current) {
        if find_path(snapshot, child, target, path).is_some() {
            return Some(());
        }
    }
    path.pop();
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::parser::Parser as CssParser;
    use crate::html::parser::Parser as HtmlParser;
    use crate::layouter::css_resolver::{CssResolver, MediaEnvironment, ResolvedStyles};
    use crate::layouter::types::ColorScheme;

    /// Leaks a tiny fixture so `RuleSet` can borrow it for `'static`; keeps
    /// test plumbing free of self-referential lifetimes.
    fn leak(resolved: ResolvedStyles) -> &'static ResolvedStyles {
        Box::leak(Box::new(resolved))
    }

    fn resolved_for(css: &str) -> ResolvedStyles {
        CssResolver::resolve(&CssParser::new(css).parse().unwrap())
    }

    fn resolved_for_origin(css: &str, origin: StyleOrigin) -> ResolvedStyles {
        CssResolver::resolve_with_origin(&CssParser::new(css).parse().unwrap(), origin)
    }

    fn media() -> MediaEnvironment {
        MediaEnvironment::new((800.0, 600.0), ColorScheme::Light)
    }

    fn snapshot_for(html: &str) -> DomSnapshot {
        let dom = HtmlParser::new(html).parse();
        DomSnapshot::from_tree(&dom.root).0
    }

    fn node_with_tag(snapshot: &DomSnapshot, tag: &str) -> NodeId {
        nodes_with_tag(snapshot, tag)
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("no <{tag}> in fixture"))
    }

    /// Pre-order positions of every element with `tag`; text nodes sit
    /// between element ids, so callers must not assume consecutive ids.
    fn nodes_with_tag(snapshot: &DomSnapshot, tag: &str) -> Vec<NodeId> {
        snapshot
            .nodes()
            .iter()
            .enumerate()
            .filter(|(_, node)| node.kind.tag_name() == Some(tag))
            .map(|(index, _)| index as NodeId)
            .collect()
    }

    fn declaration<'a>(rule: &'a MatchedRule, name: &str) -> &'a InspectedDeclaration {
        rule.declarations
            .iter()
            .find(|declaration| declaration.name == name)
            .unwrap_or_else(|| panic!("no {name} declared by {}", rule.selector))
    }

    #[test]
    fn later_specific_rule_overrides_and_flags_the_loser() {
        let snapshot = snapshot_for("<body><p class=\"box\">t</p></body>");
        let styles = leak(resolved_for("p { color: red; } .box { color: blue; }"));
        let rules = RuleSet::from_declarations(styles, &media());
        let target = node_with_tag(&snapshot, "p");

        let matched = collect_matched_rules(&snapshot, target, &rules, None);

        let tag_rule = matched.iter().find(|rule| rule.selector == "p").unwrap();
        let class_rule = matched.iter().find(|rule| rule.selector == ".box").unwrap();
        assert!(!declaration(tag_rule, "color").applied);
        assert!(declaration(class_rule, "color").applied);
    }

    #[test]
    fn chain_reconstruction_carries_sibling_indexes() {
        let snapshot = snapshot_for("<ul><li>a</li><li>b</li><span>s</span></ul>");

        fn indexes(snapshot: &DomSnapshot, id: NodeId) -> (usize, usize) {
            let chain = chain_for_node(snapshot, id).expect("chain");
            let element = chain.first().unwrap();
            (element.element_index, element.element_count)
        }

        let lis = nodes_with_tag(&snapshot, "li");
        assert_eq!(lis.len(), 2);
        assert_eq!(indexes(&snapshot, lis[0]), (1, 3));
        assert_eq!(indexes(&snapshot, lis[1]), (2, 3));

        // The chain reaches ancestors: first() is the node itself.
        let chain = chain_for_node(&snapshot, lis[1]).unwrap();
        assert_eq!(chain.first().unwrap().tag_name, "li");
    }

    #[test]
    fn structural_pseudo_classes_use_reconstructed_sibling_indexes() {
        let snapshot = snapshot_for("<ul><li>a</li><li>b</li></ul>");
        let styles = leak(resolved_for("li:nth-child(2) { color: green; }"));
        let rules = RuleSet::from_declarations(styles, &media());
        let lis = nodes_with_tag(&snapshot, "li");

        let matched = collect_matched_rules(&snapshot, lis[1], &rules, None);
        assert!(
            matched.iter().any(
                |rule| rule.selector == "li:nth-child(2)" && declaration(rule, "color").applied
            ),
            "second li must match :nth-child(2)"
        );

        let matched = collect_matched_rules(&snapshot, lis[0], &rules, None);
        assert!(matched.is_empty(), "first li must not match :nth-child(2)");
    }

    #[test]
    fn descendant_combinator_selectors_match_through_ancestors() {
        let snapshot = snapshot_for("<div class=\"outer\"><section><p>t</p></section></div>");
        let styles = leak(resolved_for("div.outer p { color: teal; }"));
        let rules = RuleSet::from_declarations(styles, &media());

        let matched = collect_matched_rules(&snapshot, node_with_tag(&snapshot, "p"), &rules, None);
        assert_eq!(matched[0].selector, "div.outer p");
        assert!(declaration(&matched[0], "color").applied);
    }

    #[test]
    fn inline_styles_beat_author_rules_but_not_important_ones() {
        let snapshot = snapshot_for("<body><p style=\"color: black\">t</p></body>");
        let target = node_with_tag(&snapshot, "p");

        let styles = leak(resolved_for("p { color: red; }"));
        let rules = RuleSet::from_declarations(styles, &media());
        let matched = collect_matched_rules(&snapshot, target, &rules, Some("color: black"));
        let inline = matched.first().expect("inline entry leads");
        assert!(inline.inline && inline.origin == StyleOrigin::Author);
        assert_eq!(inline.selector, "element.style");
        assert!(declaration(inline, "color").applied);

        let styles = leak(resolved_for("p { color: red !important; }"));
        let rules = RuleSet::from_declarations(styles, &media());
        let matched = collect_matched_rules(&snapshot, target, &rules, Some("color: black"));
        let inline = matched.first().unwrap();
        let stylesheet = matched.iter().find(|rule| rule.selector == "p").unwrap();
        assert!(!declaration(inline, "color").applied);
        assert!(declaration(stylesheet, "color").applied);
    }

    #[test]
    fn user_agent_origin_ranks_below_author_origin() {
        let snapshot = snapshot_for("<body><p>t</p></body>");
        let mut combined = resolved_for_origin("p { margin-top: 0px; }", StyleOrigin::UserAgent);
        combined.extend(resolved_for_origin(
            "p { margin-top: 4px; }",
            StyleOrigin::Author,
        ));
        let styles = leak(combined);
        let rules = RuleSet::from_declarations(styles, &media());

        let matched = collect_matched_rules(&snapshot, node_with_tag(&snapshot, "p"), &rules, None);
        let ua = matched
            .iter()
            .find(|rule| rule.origin == StyleOrigin::UserAgent)
            .expect("user-agent rule reported");
        let author_rule = matched
            .iter()
            .find(|rule| rule.origin == StyleOrigin::Author)
            .expect("author rule reported");
        assert!(!declaration(ua, "margin-top").applied);
        assert!(declaration(author_rule, "margin-top").applied);
    }
}
