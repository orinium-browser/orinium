//! Demonstrates the custom-node / SystemUi draw-command pipeline.
//!
//! Builds a small HTML fragment containing `<iframe>`, `<input>`, and
//! `<button>` tags, runs layout, and dumps the resulting draw commands.
//! `<iframe>` and `<input>` emit `SystemUi`; `<button>` renders itself
//! with plain `Fill` + `DrawText` commands.
//!
//! ```sh
//! cargo run --example custom_node
//! ```

use orinium_browser::engine::{
    css::parser::Parser as CssParser,
    html::parser::Parser as HtmlParser,
    layouter::{
        InheritedCss, build_layout_and_info,
        css_resolver::{CssResolver, ResolvedStyles},
        types::TextStyle,
    },
    renderer_model::{DrawCommand, SystemUiKind, generate_draw_commands},
};
use std::sync::Arc;

use orinium_browser::platform::renderer::text_measurer::PlatformTextMeasurer;
use ui_layout::LayoutEngine;

const HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
  <style>
    body   { margin: 16px; font-family: sans-serif; }
    iframe  { display: block; width: 300px; height: 200px;
              border: 2px solid #999; margin-bottom: 12px; }
    input   { display: block; width: 250px; height: 32px;
              border: 1px solid #bbb; }
    button  { display: block; width: 120px; height: 36px;
              margin-top: 12px; }
  </style>
</head>
<body>
  <h1>Custom Node Demo</h1>
  <iframe src="about:blank"></iframe>
  <input placeholder="Type here…">
  <button>Click me</button>
  <p>Trailing paragraph</p>
</body>
</html>"#;

fn main() {
    let mut parser = HtmlParser::new(HTML);
    let dom = parser.parse();

    // ── Resolve styles ──────────────────────────────────────
    let mut resolved = ResolvedStyles::default();

    let ua_css = include_str!("../resource/user-agent.css");
    let ua_sheet = CssParser::new(ua_css).parse().expect("parse UA CSS");
    resolved.extend(CssResolver::resolve(&ua_sheet));

    // inline styles from <style> elements
    let inline = dom.collect_text_by_tag("style");
    for css in &inline {
        if let Ok(sheet) = CssParser::new(css).parse() {
            resolved.extend(CssResolver::resolve(&sheet));
        }
    }

    // ── Build layout + info ─────────────────────────────────
    let measurer =
        PlatformTextMeasurer::new().expect("PlatformTextMeasurer requires a system font");
    let measurer = Arc::new(measurer);
    let (mut layout, info) = build_layout_and_info(
        &dom.root,
        &resolved,
        measurer,
        InheritedCss {
            text_style: TextStyle {
                font_size: 16.0,
                ..Default::default()
            },
            color_scheme: Default::default(),
        },
        Vec::new(),
        dark_light::detect().map(Into::into).unwrap_or_else(|e| {
            log::error!("Failed to detect system color scheme, using default: {e}");
            Default::default()
        }),
        orinium_browser::engine::html::ScriptingMode::Enabled,
    );

    // ── Layout pass ─────────────────────────────────────────
    LayoutEngine::layout(&mut layout, 800.0, 600.0);

    // ── Generate draw commands ──────────────────────────────
    let mut cmds: Vec<DrawCommand> = Vec::new();
    generate_draw_commands(&mut cmds, &layout, &info, (800.0, 600.0));

    // ── Dump ────────────────────────────────────────────────
    println!("Generated {} draw commands:\n", cmds.len());
    for (i, cmd) in cmds.iter().enumerate() {
        println!("  [{i:>3}] {cmd:?}");
    }

    // ── Highlight SystemUi commands ─────────────────────────
    let system_ui: Vec<_> = cmds
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c, DrawCommand::SystemUi { .. }))
        .collect();

    println!("\n── SystemUi commands ({}) ──", system_ui.len());
    for (i, cmd) in &system_ui {
        if let DrawCommand::SystemUi { kind, rect } = cmd {
            match kind {
                SystemUiKind::WebView { surface_id } => {
                    println!(
                        "  [{i:>3}] Iframe   surface_id={surface_id}  rect=({}, {}, {}, {})",
                        rect.x, rect.y, rect.width, rect.height
                    );
                }
                SystemUiKind::Input { value, placeholder } => {
                    println!(
                        "  [{i:>3}] Input    value={value:?}  placeholder={placeholder:?}  rect=({}, {}, {}, {})",
                        rect.x, rect.y, rect.width, rect.height
                    );
                }
            }
        }
    }
}
