use anyhow::Result;
use orinium_browser::engine::bridge::text::{
    FontDescription, LayoutConstraints, TextMeasurementRequest, TextMeasurer,
};
use orinium_browser::platform::renderer::text_measurer::PlatformTextMeasurer;
use orinium_browser::{
    browser::BrowserApp,
    renderer::{Color, DrawCommand, NodeKind, RenderTree},
};

fn main() -> Result<()> {
    env_logger::init();

    let measurer = PlatformTextMeasurer::new().map_err(|e| anyhow::anyhow!("{}", e))?;

    let mut commands = Vec::new();

    commands.extend(debug_text_commands(
        &measurer,
        "Test text",
        50.0,
        50.0,
        24.0,
    )?);
    commands.extend(debug_text_commands(
        &measurer,
        "こんにちは、世界！",
        50.0,
        100.0,
        32.0,
    )?);
    commands.extend(debug_text_commands(&measurer, "This is a longer piece of text that should wrap around to the next line when it exceeds the maximum width.", 50.0, 150.0, 20.0)?);
    commands.extend(debug_text_commands(
        &measurer,
        "1234567890-^#$%&()=~|",
        50.0,
        250.0,
        24.0,
    )?);

    let root = orinium_browser::engine::renderer::render_node::RenderNode::new(
        NodeKind::Container,
        0.0,
        0.0,
        800.0,
        600.0,
    );
    let render_tree = RenderTree::new(root);
    let app = BrowserApp::default().with_draw_info(render_tree, commands);
    app.run()?;

    Ok(())
}

fn debug_text_commands(
    measurer: &dyn TextMeasurer,
    text: &str,
    x: f32,
    y: f32,
    font_size: f32,
) -> anyhow::Result<Vec<DrawCommand>> {
    let req = TextMeasurementRequest {
        text: text.to_string(),
        font: FontDescription {
            family: None,
            size_px: font_size,
        },
        constraints: LayoutConstraints {
            max_width: None,
            wrap: true,
            max_lines: None,
        },
    };

    let measurement = measurer
        .measure(&req)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    Ok(vec![
        DrawCommand::DrawRect {
            x,
            y,
            width: measurement.width,
            height: measurement.height,
            color: Color::BLACK,
        },
        DrawCommand::DrawText {
            x,
            y,
            text: text.to_string(),
            font_size,
            color: Color::WHITE,
        },
    ])
}
