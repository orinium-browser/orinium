use orinium_browser::engine::css::parser::{CssNodeType, Parser};

#[test]
fn test_parse_all_css_syntax() {
    // CSS 読み込み
    let css = r#"
    /* コメント */
    body {
        margin: 0;
        padding: 1em 2px 3% 4rem;
        font-size: 16px;
        color: #f00;
        background-color: rgb(255, 255, 255);
        border: 1px solid black !important;
    }

    /* 属性セレクタ */
    input[type="text"] {
        border: 1px dashed blue;
    }

    /* 複数セレクタ */
    h1, h2, h3 {
        font-weight: bold;
    }

    /* クラス・ID・擬似クラス */
    #main.container:hover::before {
        content: "Hello";
    }

    /* ネストした関数 */
    div {
        width: calc(100% - 20px);
        color: rgba(255, calc(128 + 127), 0, 0.5);
    }

    /* 関数・変数・!important */
    :root {
        --main-color: #00f;
    }

    p {
        color: var(--main-color) !important;
    }

    /* メディアクエリ */
    @media screen and (max-width: 600px) {
        body { font-size: 14px; }
    }

    /* サポート条件 */
    @supports (display: grid) {
        div { display: grid; }
    }

    /* 擬似要素・擬似クラス複合 */
    a:hover::after, a:active::before {
        content: "";
    }

    /* 隣接・子・兄弟セレクタ */
    div > p + span ~ a {
        text-decoration: underline;
    }

    /* 文字列・url */
    img[alt~="logo"] {
        content: url("logo.png");
    }

    /* 無効宣言（エラー回復テスト） */
    h4 {
        invalid-property value
    }
"#;

    // パーサー生成
    let mut parser = Parser::new(&css);

    // パース実行
    let result = parser.parse();
    assert!(result.is_ok(), "CSS parser failed: {:?}", result.err());

    // デバッグ出力
    let stylesheet = result.unwrap();
    println!("{}", stylesheet);

    // ここで必要なら、CSS ノードの個数や種類のアサーションも追加可能
    let children = stylesheet.children();
    assert!(!children.is_empty(), "No rules parsed");
}

#[test]
fn test_css_nesting_with_ampersand() {
    let css = r#"
    .parent {
        color: red;
        & {
            font-size: 14px;
        }
        &.highlight {
            background: yellow;
        }
        & > span {
            color: blue;
        }
        .sidebar& {
            width: 200px;
        }
        &.a, &.b {
            margin: 0;
        }
    }
    "#;

    let mut parser = Parser::new(css);
    let result = parser.parse();
    assert!(result.is_ok(), "CSS parser failed: {:?}", result.err());

    let stylesheet = result.unwrap();
    let parent_rule = &stylesheet.children()[0];

    // Verify parent rule has 6 children: 1 declaration + 5 nested rules
    let children = parent_rule.children();
    assert_eq!(children.len(), 6);

    // First child: declaration (color: red)
    let CssNodeType::Declaration { name, .. } = children[0].node() else {
        panic!("expected declaration");
    };
    assert_eq!(name, "color");

    // Verify nested rules have is_nesting flag
    for child in &children[1..] {
        let CssNodeType::Rule { selectors } = child.node() else {
            panic!("expected rule");
        };
        assert!(
            selectors[0].parts.iter().any(|p| p.selector.is_nesting),
            "nested rule should have is_nesting selector: {}",
            selectors[0]
        );
    }
}

#[test]
fn test_css_nesting_without_ampersand() {
    let css = r#"
    .parent {
        span {
            color: red;
        }
    }
    "#;

    let mut parser = Parser::new(css);
    let result = parser.parse();
    assert!(result.is_ok(), "CSS parser failed: {:?}", result.err());

    let stylesheet = result.unwrap();
    let parent_rule = &stylesheet.children()[0];

    let CssNodeType::Rule {
        selectors: child_selectors,
    } = parent_rule.children()[0].node()
    else {
        panic!("expected nested rule");
    };

    // Child selector should not have is_nesting
    assert!(!child_selectors[0].parts[0].selector.is_nesting);
    assert_eq!(child_selectors[0].to_string(), "span");
}

#[test]
fn test_css_nesting_resolves_correctly() {
    let css = r#"
    .parent {
        &.highlight { color: yellow; }
        & > span { color: blue; }
        .sidebar& { width: 200px; }
    }
    "#;

    let mut parser = Parser::new(css);
    let stylesheet = parser.parse().unwrap();

    let CssNodeType::Rule { selectors: parent } = stylesheet.children()[0].node() else {
        panic!("expected parent rule");
    };
    let parent_sel = &parent[0];

    // &.highlight → .parent.highlight
    let CssNodeType::Rule { selectors: child1 } = stylesheet.children()[0].children()[0].node()
    else {
        panic!("expected nested rule");
    };
    let resolved1 = parent_sel.nest(&child1[0]);
    assert_eq!(resolved1.to_string(), ".parent.highlight");

    // & > span → .parent > span
    let CssNodeType::Rule { selectors: child2 } = stylesheet.children()[0].children()[1].node()
    else {
        panic!("expected nested rule");
    };
    let resolved2 = parent_sel.nest(&child2[0]);
    assert_eq!(resolved2.to_string(), ".parent > span");

    // .sidebar& → .sidebar.parent
    let CssNodeType::Rule { selectors: child3 } = stylesheet.children()[0].children()[2].node()
    else {
        panic!("expected nested rule");
    };
    let resolved3 = parent_sel.nest(&child3[0]);
    assert_eq!(resolved3.to_string(), ".parent.sidebar");
}
