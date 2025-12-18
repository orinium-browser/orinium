use anyhow::Result;
use orinium_browser::{
    browser::BrowserApp,
    renderer::{Color, DrawCommand, RenderTree},
};
use orinium_browser::engine::bridge::text::{FontDescription, LayoutConstraints, TextMeasurementRequest, TextMeasurer};
use orinium_browser::platform::renderer::text_measurer::PlatformTextMeasurer;

fn main() -> Result<()> {
    env_logger::init();

    let text = "This is a sample text to demonstrate text measurement and overlay rendering in the Orinium Browser example application. The quick brown fox jumps over the lazy dog. 1234567890!@#$%^&*()_+-=[]{}|;':\",.<>/?`~";
    let measurer = PlatformTextMeasurer::new().map_err(|e| anyhow::anyhow!("{}", e))?;

    let req = TextMeasurementRequest {
        text: text.to_string(),
        font: FontDescription { family: None, size_px: 24.0 },
        constraints: LayoutConstraints { max_width: Some(400.0), wrap: true, max_lines: None },
    };

    let measurement = measurer.measure(&req).map_err(|e| anyhow::anyhow!("{}", e))?;
    let mut commands = Vec::new();

    commands.push(DrawCommand::DrawRect { x: 0.0, y: 0.0, width: 800.0, height: 600.0, color: Color::WHITE });

    let x = 50.0;
    let y = 50.0;
    
    commands.push(DrawCommand::PushClip { x, y, width: measurement.width, height: measurement.height });
    commands.push(DrawCommand::DrawText { x, y, text: text.to_string(), font_size: 24.0, color: Color::BLACK });
    commands.push(DrawCommand::PopClip);

    commands.push(DrawCommand::DrawRect { x, y, width: measurement.width, height: measurement.height, color: Color::new(1.0, 0.0, 0.0, 0.3) });

    let root = orinium_browser::engine::renderer::render_node::RenderNode::new(orinium_browser::engine::renderer::render_node::NodeKind::Unknown, 0.0, 0.0, 800.0, 600.0);
    let render_tree = RenderTree::new(root);
    let app = BrowserApp::default().with_draw_info(render_tree, commands);
    app.run()?;

    Ok(())
}
