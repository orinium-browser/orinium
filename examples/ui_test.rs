use anyhow::{Result, bail};
use orinium_browser::browser::{BrowserApp, Tab};
use orinium_browser::browser::core::ui::compoments::Components;
use orinium_browser::engine::input::hit_test;
use orinium_browser::engine::layouter::types::{InfoNode, NodeKind};
use orinium_browser::engine::renderer_model::{DrawCommand, generate_draw_commands};
use ui_layout::LayoutNode;

fn main() -> Result<()> {
    env_logger::init();

    let html_without_css = include_str!("../resource/test/ui_test_no_css_button.html");

    let mut tab = Tab::new();
    tab.navigate("resource:///ui_test.html".parse()?);

    // Initialize WebView once so UA CSS is loaded before HTML parse.
    let _ = tab.tick();

    tab.on_fetch_succeeded_html(html_without_css.to_string());
    let _ = tab.tick();
    tab.relayout((800.0, 600.0));

    let (layout, info) = tab
        .layout_and_info()
        .ok_or_else(|| anyhow::anyhow!("page layout was not built"))?;

    if !contains_uipart_button(info) {
        bail!("no UiPart(Button) node exists in info tree");
    }

    let page_commands = generate_draw_commands(layout, info, None);

    let Some((cx, cy)) = find_button_center(layout, info) else {
        bail!("UiPart(Button) exists but has no layout box after relayout");
    };

    let has_button_rect = page_commands
        .iter()
        .any(|c| matches!(c, DrawCommand::DrawRect { .. }));
    if !has_button_rect {
        bail!("UiPart(Button) exists but no DrawRect command was generated");
    }

    // UA CSSありでもなしでも、少なくとも draw command が生成されることを確認
    if page_commands.is_empty() {
        bail!("page draw commands were empty for html without css");
    }

    let hit_path = hit_test(layout, info, cx, cy);
    let hit_button = hit_path.iter().any(|entry| {
        matches!(
            entry.info.kind,
            NodeKind::UiPart {
                ref compoment
            } if matches!(compoment, Components::Button)
        )
    });
    if !hit_button {
        bail!("hit_test on button center did not hit UiPart(Button)");
    }

    println!("ok: <button> maps to UiPart(Button), draws rect, and is hit-testable");

    let mut browser = BrowserApp::default();
    let mut render_tab = Tab::new();
    render_tab.navigate("resource:///test/ui_test_no_css_button.html".parse()?);
    browser.add_tab(render_tab);
    browser.run()?;

    Ok(())
}

fn find_button_center(layout: &LayoutNode, info: &InfoNode) -> Option<(f32, f32)> {
    if let NodeKind::UiPart { compoment } = &info.kind
        && matches!(compoment, Components::Button)
        && let Some(box_model) = layout.layout_boxes.iter().next()
    {
        let rect = box_model.padding_box;
        return Some((rect.x + rect.width * 0.5, rect.y + rect.height * 0.5));
    }

    for (child_layout, child_info) in layout.children.iter().zip(&info.children) {
        if let Some(center) = find_button_center(child_layout, child_info) {
            return Some(center);
        }
    }

    None
}

fn contains_uipart_button(info: &InfoNode) -> bool {
    if matches!(
        info.kind,
        NodeKind::UiPart {
            compoment: Components::Button
        }
    ) {
        return true;
    }

    info.children.iter().any(contains_uipart_button)
}
