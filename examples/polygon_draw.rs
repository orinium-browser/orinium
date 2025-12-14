use anyhow::Result;
use orinium_browser::browser::BrowserApp;
use orinium_browser::engine::renderer::DrawCommand;
use orinium_browser::engine::renderer::render::Color;
use orinium_browser::engine::renderer::RenderTree;
use orinium_browser::engine::renderer::render_node::{RenderNode, NodeKind};

fn main() -> Result<()> {
    env_logger::init();

    let mut commands = Vec::new();



    // bg
    commands.push(DrawCommand::DrawRect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
        color: Color::WHITE,
    });

    // polygon
    commands.push(DrawCommand::DrawPolygon {
        points: vec![
            (200.0, 150.0),
            (350.0, 120.0),
            (500.0, 200.0),
            (420.0, 360.0),
            (220.0, 340.0),
        ],
        color: Color::new(0.2, 0.6, 0.9, 1.0),
    });

    commands.push(DrawCommand::DrawPolygon {
        points: vec![
            (100.0, 400.0),
            (300.0, 380.0),
            (320.0, 500.0),
            (220.0, 460.0),
            (180.0, 520.0),
            (80.0, 480.0),
            (60.0, 420.0),
        ],
        color: Color::new(0.9, 0.3, 0.4, 1.0),
    });

    commands.push(DrawCommand::PushClip {
        x: 50.0,
        y: 20.0,
        width: 300.0,
        height: 200.0,
    });
    commands.push(DrawCommand::DrawPolygon {
        points: vec![
            (20.0, 10.0),
            (200.0, 10.0),
            (340.0, 140.0),
            (260.0, 250.0),
            (30.0, 200.0),
        ],
        color: Color::new(0.3, 0.8, 0.4, 1.0),
    });
    commands.push(DrawCommand::PopClip);

    let root = RenderNode::new(NodeKind::Unknown, 0.0, 0.0, 800.0, 600.0);
    let render_tree = RenderTree::new(root);
    let app = BrowserApp::default().with_draw_info(render_tree, commands);
    app.run()?;

    Ok(())
}
