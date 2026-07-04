use orinium_browser::engine::css::parser::Parser as CssParser;
use orinium_browser::engine::html::parser;

#[test]
fn test_sub_sup_rendering() {
    // Test 1: Basic sub/sup tag parsing
    let html1 = "<html><body><p>Normal <sub>subscript</sub> normal <sup>superscript</sup> normal</p></body></html>";
    let mut parser1 = parser::Parser::new(html1);
    let dom1 = parser1.parse();
    println!("DOM1 Tree: {}\n", dom1);

    // Test 2: Font styling with sub/sup
    let css2 = r"#
    sub {
        font-size: 0.7em;
        vertical-align: sub;
    }
    sup {
        font-size: 0.7em;
        vertical-align: super;
    }
    ";
    let mut css_parser2 = CssParser::new(css2);
    let stylesheet2 = css_parser2.parse();
    println!("CSS2 Stylesheet: {:?}\n", stylesheet2);

    // Test 3: Combined HTML+CSS
    let html3 = "<html><body><p>Formula: E = mc² <sub>2</sub></p></body></html>";
    let mut parser3 = parser::Parser::new(html3);
    let dom3 = parser3.parse();
    println!("DOM3 Tree: {}\n", dom3);
}
