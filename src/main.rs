use anyhow::Result;
use orinium_browser::engine::renderer::{Color, DrawCommand};
use std::env;

use orinium_browser::engine::html::parser::Parser;
use orinium_browser::engine::renderer::Renderer;
use orinium_browser::platform::ui::App;
use winit::event_loop::EventLoop;

#[tokio::main]
async fn main() -> Result<()> {
    let _args: Vec<String> = env::args().collect::<Vec<String>>();
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
                <div>
                    <p>Nested paragraph in a div.</p>
                </div>
            </body>
        </html>
    "#;

    // HTMLをパース
    let mut parser = Parser::new(html);
    let dom_tree = parser.parse();

    log::info!("DOM Tree parsed successfully");

    // レンダラーを作成して描画命令を生成
    let renderer = Renderer::new(800.0, 600.0);
    let mut draw_commands = renderer.generate_draw_commands(&dom_tree);

    // テスト用の矩形を追加（色の値を0.0〜1.0の範囲に修正）
    draw_commands.push(DrawCommand::DrawRect {
        x: 0.0,
        y: 100.0,
        width: 100.0,
        height: 100.0,
        color: Color { r: 0.8, g: 0.2, b: 0.2, a: 1.0 }
    });

    log::info!("Generated {} draw commands", draw_commands.len());
    log::info!("Generated draw commands: {:#?}", draw_commands);

    // ウィンドウとイベントループを作成
    let event_loop = EventLoop::<orinium_browser::platform::ui::State>::with_user_event().build()?;
    let mut app = App::new();

    // 描画命令をAppに設定
    app.set_draw_commands(draw_commands);

    event_loop.run_app(&mut app)?;

    Ok(())
}
