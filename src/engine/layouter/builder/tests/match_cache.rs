use super::*;

fn build_with_cache<'a>(
    snapshot: &'a DomSnapshot,
    rule_set: &'a RuleSet,
    cache: Option<&'a mut SelectorMatchCache>,
) -> (LayoutNode, InfoNode) {
    build_layout_and_info_from_snapshot(
        snapshot,
        snapshot.roots()[0],
        rule_set,
        Arc::new(FallbackTextMeasurer),
        Arc::new(NullAudioSinkFactory),
        InheritedCss::default(),
        ElementChain::default(),
        ColorScheme::Light,
        ScriptingMode::default(),
        &HashMap::new(),
        &HashMap::new(),
        None,
        cache,
    )
}

#[test]
fn selector_matches_are_reused_across_builds_of_the_same_snapshot() {
    let html = "<html><body>\
                  <div class='box'><p class='label'>hello</p></div>\
                  <span id='tail'>world</span>\
                </body></html>";
    let css = ".box { padding: 4px; } \
               .label { color: red; } \
               body div p { font-size: 20px; } \
               #tail { color: blue; } \
               div > p, span { margin: 2px; }";
    let dom = HtmlParser::new(html).parse();
    let (snapshot, _dom_refs) = DomSnapshot::from_tree(&dom.root);
    let sheet = CssParser::new(css).parse().unwrap();
    let resolved = CssResolver::resolve(&sheet);
    let media = MediaEnvironment::new((0.0, 0.0), ColorScheme::Light);
    let rule_set = RuleSet::from_declarations(&resolved, &media);

    let mut cache = SelectorMatchCache::new();
    let (_, first_info) = build_with_cache(&snapshot, &rule_set, Some(&mut cache));
    let first_stats = cache.stats();
    assert!(
        first_stats.misses > 0,
        "the first build must populate the cache"
    );
    assert_eq!(first_stats.hits, 0);

    let (_, second_info) = build_with_cache(&snapshot, &rule_set, Some(&mut cache));
    let second_stats = cache.stats();

    assert_eq!(
        second_stats.misses, first_stats.misses,
        "the second build must not recompute selector matches"
    );
    assert_eq!(second_stats.hits, first_stats.misses);
    assert_eq!(second_stats.evictions, 0);
    assert_eq!(second_stats.skips, 0);

    // Cached matches must not change the cascade result.
    assert_eq!(
        text_style_for(&first_info, "hello"),
        text_style_for(&second_info, "hello")
    );
    assert_eq!(
        text_style_for(&first_info, "world"),
        text_style_for(&second_info, "world")
    );
    assert_eq!(text_flow_style_for(&second_info, "hello").font_size, 20.0);
    // The `body div p` rule must not leak onto the span through the cache.
    assert_eq!(text_flow_style_for(&second_info, "world").font_size, 16.0);
}
