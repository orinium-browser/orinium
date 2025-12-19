use anyhow::Result;
use orinium_browser::{
    browser::BrowserApp,
    renderer::{Color, DrawCommand, NodeKind, RenderNode, RenderTree},
};

fn main() -> Result<()> {
    env_logger::init();

    let mut commands = Vec::new();

    let text = "This is a very long sample text that should be clipped by the container's box.\nLorem ipsum dolor sit amet, consectetur adipiscing elit. Vivamus lacinia odio vitae vestibulum vestibulum. Cras venenatis euismod malesuada.";

    commands.push(DrawCommand::PushClip {
        x: 10.0,
        y: 10.0,
        width: 780.0,
        height: 580.0,
    });
    commands.push(DrawCommand::DrawRect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
        color: Color::new(0.8, 0.8, 0.8, 1.0),
    });
    commands.push(DrawCommand::DrawText {
        x: 0.0,
        y: 0.0,
        text: text.to_string(),
        font_size: 20.0,
        color: Color::BLACK,
    });
    commands.push(DrawCommand::PopClip);

    let root = RenderNode::new(NodeKind::Container, 0.0, 0.0, 800.0, 600.0);
    let render_tree = RenderTree::new(root);
    let app = BrowserApp::default().with_draw_info(render_tree, commands);
    app.run()?;

    Ok(())
}
