use anyhow::Result;
use orinium_browser::browser::BrowserApp;
//use orinium_browser::renderer::Color;
use std::env;
use std::sync::Arc;

use orinium_browser::engine::css::cssom::Parser as CssParser;
use orinium_browser::engine::html::parser::Parser as HtmlParser;
use orinium_browser::engine::renderer::Renderer;
use orinium_browser::platform::ui::App;
use winit::event_loop::EventLoop;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let font_path = if args.len() > 1 {
        Some(args[1].clone())
    } else {
        None
    };

    env_logger::init();

    // テスト用のHTML
    let html = r#"
        <!DOCTYPE html>
        <html>
            <head>
                <title>Test Page</title>
            </head>
            <body>
                <h1>Hello, Orinium Browser!</h1>
                <p>This is a test paragraph.</p>
                <p>日本語テスト</p>
                <div>
                    <p>Nested paragraph in a div.</p>
                </div>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
                <p>a</p>
            </body>
        </html>
    "#;

    // HTMLをパース
    let mut parser = HtmlParser::new(html);
    let dom_tree = parser.parse();

    log::info!("DOM Tree parsed successfully");

    // CSSOMツリーの構築
    let css: &str = r#"
    /* Reset / Normalize */
    html, body {
        margin: 0;
        padding: 0;
        border: 0;
        font-size: 16px;
        font-family: sans-serif;
        line-height: 1.2;
        background: white;
        color: black;
    }

    /* Headings */
    h1 { font-size: 2em; font-weight: bold; margin: 0.67em 0; }
    h2 { font-size: 1.5em; font-weight: bold; margin: 0.75em 0; }
    h3 { font-size: 1.17em; font-weight: bold; margin: 0.83em 0; }
    h4 { font-size: 1em; font-weight: bold; margin: 1.12em 0; }
    h5 { font-size: 0.83em; font-weight: bold; margin: 1.5em 0; }
    h6 { font-size: 0.75em; font-weight: bold; margin: 1.67em 0; }

    /* Paragraphs */
    p { margin: 1em 0; }

    /* Links */
    a {
        color: blue;
        text-decoration: underline;
    }

    /* Lists */
    ul, ol {
        margin: 1em 0;
        padding-left: 40px;
    }

    /* Table */
    table {
        border-collapse: collapse;
        border-spacing: 0;
    }

    /* Form elements */
    input, textarea, select, button {
        font: inherit;
        margin: 0;
        padding: 0;
    }

    /* Images */
    img {
        max-width: 100%;
        height: auto;
        display: inline-block;
    }

    /* Blockquotes */
    blockquote {
        margin: 1em 0;
        padding-left: 40px;
        border-left: 4px solid #ccc;
    }

    /* Horizontal rule */
    hr {
        border: none;
        border-top: 1px solid #ccc;
        margin: 1em 0;
    }

    /* Code */
    pre, code {
        font-family: monospace;
        font-size: 1em;
    }
    "#;

    // domから取得したスタイルを追加
    let additional_css = dom_tree.collect_text_by_tag("style").join("\n");
    let css: String = additional_css + css;

    let mut css_parser = CssParser::new(&css);
    let css_tree = css_parser.parse()?;

    // スタイルツリーの構築
    let mut style_tree = orinium_browser::engine::styler::StyleTree::transform(&dom_tree);
    style_tree = style_tree.style(&[css_tree]);

    // レンダラーを作成して描画命令を生成
    let renderer = Renderer::new(800.0, 600.0);
    let draw_commands = renderer.generate_draw_commands(&mut style_tree);

    log::info!("Generated {} draw commands", draw_commands.len());
    log::info!("Generated draw commands: {draw_commands:#?}");

    // ウィンドウとイベントループを作成
    let event_loop =
        EventLoop::<orinium_browser::platform::ui::State>::with_user_event().build()?;
    let mut app = App::new(BrowserApp::new().with_draw_commands(draw_commands));

    event_loop.run_app(&mut app)?;

    Ok(())
}
