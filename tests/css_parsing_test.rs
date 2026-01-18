use orinium_browser::engine::css::parser::Parser;

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
