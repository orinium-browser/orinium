use orinium_browser::engine::html::parser;
use orinium_browser::engine::html::parser::{
    ClassicScriptDescriptor, ClassicScriptExecution, ClassicScriptSource, DomTree, HtmlNodeType,
    ScriptingMode,
};

#[test]
fn test_dom_parse() {
    let html = r#"<!DOCTYPE html>
<html lang="ja">
<head>
    <title>Orinium Browser DOM Test</title>
    <!-- コメント -->
</head>
<body>
    <p>This is a <b>test page</b> for DOM module debuging.</p>
    <div>
        <p>Nested <span>span glyph</span></p>
        <img src="image.png">
        <br />
        <input type="glyph" value="Hello" />
        <p>Unclosed paragraph
    </div>
    <footer>Footer content</footer>
</body>
</html>
"#;

    html.to_string();
    let mut parser = parser::Parser::new(&html);
    let dom = parser.parse();
    println!("DOM Tree:\n{}", dom);
}

#[test]
fn test_dom_parse_malformed() {
    let html = r#"<html><head><title>Test</title></head><body><p>Paragraph 1<p>Paragraph 2<div>Div content"#;

    let mut parser = parser::Parser::new(&html);
    let dom = parser.parse();
    println!("DOM Tree:\n{}", dom);
}

#[test]
fn classic_scripts_are_collected_in_document_order() {
    let html = r#"
        <script>window.order = "inline-1";</script>
        <script src="one.js"></script>
        <script type="module" src="module.js"></script>
        <script type="application/json">{"not":"javascript"}</script>
        <script type="text/javascript">window.order = "inline-2";</script>
        <script src="two.js"></script>
    "#;

    let dom = parser::Parser::new(html).parse();
    assert_eq!(
        dom.collect_classic_scripts(),
        [
            ClassicScriptSource::Inline("window.order = \"inline-1\";".to_string()),
            ClassicScriptSource::External("one.js".to_string()),
            ClassicScriptSource::Inline("window.order = \"inline-2\";".to_string()),
            ClassicScriptSource::External("two.js".to_string()),
        ]
    );
}

#[test]
fn classic_script_scheduling_attributes_are_collected() {
    let html = r#"
        <script async defer src="async-wins.js"></script>
        <script defer src="defer.js"></script>
        <script src="default.js"></script>
        <script async>window.inline = true;</script>
    "#;

    let dom = parser::Parser::new(html).parse();
    assert_eq!(
        dom.collect_classic_script_descriptors(),
        [
            ClassicScriptDescriptor {
                source: ClassicScriptSource::External("async-wins.js".to_string()),
                execution: ClassicScriptExecution::Async,
            },
            ClassicScriptDescriptor {
                source: ClassicScriptSource::External("defer.js".to_string()),
                execution: ClassicScriptExecution::Defer,
            },
            ClassicScriptDescriptor {
                source: ClassicScriptSource::External("default.js".to_string()),
                execution: ClassicScriptExecution::Default,
            },
            ClassicScriptDescriptor {
                source: ClassicScriptSource::Inline("window.inline = true;".to_string()),
                execution: ClassicScriptExecution::Default,
            },
        ]
    );
}

#[test]
fn query_selectors_match_in_document_order_and_with_element_scope() {
    let html = r#"
        <main id="content">
            <section class="card featured"><span data-kind="first">one</span></section>
            <section class="card"><span data-kind="second">two</span></section>
        </main>
        <section class="card" id="outside"></section>
    "#;
    let dom = parser::Parser::new(html).parse();

    let first = dom.query_selector("main > section.card span[data-kind=\"first\"]");
    assert_eq!(
        first.unwrap().borrow().value.get_attr("data-kind"),
        Some("first")
    );

    let cards = dom.query_selector_all("section.card");
    assert_eq!(cards.len(), 3);
    assert_eq!(cards[2].borrow().value.get_attr("id"), Some("outside"));

    let content = dom.get_element_by_id("content").unwrap();
    let scoped = DomTree::query_selector_all_within(&content, ".card, [data-kind=\"second\"]");
    assert_eq!(scoped.len(), 3);
    assert!(
        scoped
            .iter()
            .all(|node| { node.borrow().value.get_attr("id") != Some("outside") })
    );
    assert!(DomTree::query_selector_within(&content, "#content").is_none());
}

fn parse_noscript(html: &str, mode: ScriptingMode) -> DomTree {
    parser::Parser::new(html).with_scripting_mode(mode).parse()
}

#[test]
fn noscript_plain_text_when_scripting_enabled() {
    let dom = parse_noscript("<noscript>fallback</noscript>", ScriptingMode::Enabled);
    let noscript = dom.get_elements_by_tag_name("noscript");
    assert_eq!(noscript.len(), 1);
    let children = noscript[0].borrow().children().to_vec();
    assert_eq!(children.len(), 1);
    assert!(matches!(
        &children[0].borrow().value,
        HtmlNodeType::Text(text) if text == "fallback"
    ));
}

#[test]
fn noscript_plain_text_when_scripting_disabled() {
    let dom = parse_noscript("<noscript>fallback</noscript>", ScriptingMode::Disabled);
    let noscript = dom.get_elements_by_tag_name("noscript");
    assert_eq!(noscript.len(), 1);
    let children = noscript[0].borrow().children().to_vec();
    assert_eq!(children.len(), 1);
    assert!(matches!(
        &children[0].borrow().value,
        HtmlNodeType::Text(text) if text == "fallback"
    ));
}

#[test]
fn noscript_markup_is_raw_text_when_scripting_enabled() {
    let dom = parse_noscript(
        "<noscript><div>fallback</div></noscript>",
        ScriptingMode::Enabled,
    );
    let noscript = dom.get_elements_by_tag_name("noscript");
    assert_eq!(noscript.len(), 1);
    let noscript = &noscript[0];
    let children = noscript.borrow().children().to_vec();
    assert_eq!(children.len(), 3);
    assert!(
        children
            .iter()
            .all(|child| matches!(child.borrow().value, HtmlNodeType::Text(_)))
    );
    assert_eq!(DomTree::inner_text(noscript), "<div>fallback</div>");
    assert!(DomTree::query_selector_within(noscript, "div").is_none());
}

#[test]
fn noscript_markup_is_parsed_as_html_when_scripting_disabled() {
    let dom = parse_noscript(
        "<noscript><div>fallback</div></noscript>",
        ScriptingMode::Disabled,
    );
    let noscript = dom.get_elements_by_tag_name("noscript");
    assert_eq!(noscript.len(), 1);
    let div = DomTree::query_selector_within(&noscript[0], "div").unwrap();
    assert_eq!(DomTree::inner_text(&div), "fallback");
}

#[test]
fn noscript_raw_text_does_not_leak_into_body_when_scripting_enabled() {
    let html = "<p>Hello</p><noscript><p>Fallback</p></noscript><p>World</p>";
    let dom = parse_noscript(html, ScriptingMode::Enabled);
    let noscript = dom.get_elements_by_tag_name("noscript");
    assert_eq!(noscript.len(), 1);
    let noscript = &noscript[0];
    assert_eq!(DomTree::inner_text(noscript), "<p>Fallback</p>");
    assert!(DomTree::query_selector_within(noscript, "p").is_none());

    let paragraphs = dom.get_elements_by_tag_name("p");
    assert_eq!(paragraphs.len(), 2);
    assert_eq!(DomTree::inner_text(&paragraphs[0]), "Hello");
    assert_eq!(DomTree::inner_text(&paragraphs[1]), "World");
}

#[test]
fn noscript_parsed_as_html_when_scripting_disabled_stays_nested() {
    let html = "<p>Hello</p><noscript><p>Fallback</p></noscript><p>World</p>";
    let dom = parse_noscript(html, ScriptingMode::Disabled);
    let noscript = dom.get_elements_by_tag_name("noscript");
    assert_eq!(noscript.len(), 1);
    let fallback = DomTree::query_selector_within(&noscript[0], "p").unwrap();
    assert_eq!(DomTree::inner_text(&fallback), "Fallback");

    let paragraphs = dom.get_elements_by_tag_name("p");
    assert_eq!(paragraphs.len(), 3);
    assert_eq!(DomTree::inner_text(&paragraphs[0]), "Hello");
    assert_eq!(DomTree::inner_text(&paragraphs[1]), "Fallback");
    assert_eq!(DomTree::inner_text(&paragraphs[2]), "World");
}

#[test]
fn noscript_defaults_to_scripting_enabled() {
    let dom = parser::Parser::new("<noscript><div>fallback</div></noscript>").parse();
    let noscript = dom.get_elements_by_tag_name("noscript");
    assert_eq!(noscript.len(), 1);
    assert!(DomTree::query_selector_within(&noscript[0], "div").is_none());
    assert_eq!(DomTree::inner_text(&noscript[0]), "<div>fallback</div>");
}
