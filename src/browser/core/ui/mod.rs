//! Browser UI components. WIP stub for future UI development.

use crate::engine::renderer_model::DrawCommand;
use ui_layout::{LayoutEngine, LayoutNode, Length, Style};

/// チェックボックスやラジオボタンなどのUIコンポーネントを定義するモジュール
pub mod compoments;
use compoments::{Button, CompomentEvent, DrawCommandEmitter};

#[derive(Debug, Default)]
pub struct BrowserUi {
    pub buttons: Vec<Button>,
}

pub fn init_browser_ui(window_size: (u32, u32)) -> BrowserUi {
    let button = Button::new(
        "default",
        "Click",
        create_button_layout(window_size.0 as f32 - 140.0, 16.0, 120.0, 36.0),
    );
    BrowserUi {
        buttons: vec![button],
    }
}

impl BrowserUi {
    pub fn relayout(&mut self, viewport: (f32, f32)) {
        for button in &mut self.buttons {
            LayoutEngine::layout(&mut button.layout, viewport.0, viewport.1);
        }
    }

    pub fn draw_commands(&self) -> Vec<DrawCommand> {
        let mut commands = Vec::new();
        for button in &self.buttons {
            commands.extend(button.draw_commands());
        }
        commands
    }

    pub fn hit_button_index(&self, x: f32, y: f32) -> Option<usize> {
        self.buttons.iter().position(|button| button.hit_test(x, y))
    }

    pub fn notify_pointer_down(&mut self, button_index: usize, x: f32, y: f32) -> bool {
        let Some(button) = self.buttons.get_mut(button_index) else {
            return false;
        };

        button.on_event(CompomentEvent::PointerDown { x, y })
    }
}

fn create_button_layout(x: f32, y: f32, width: f32, height: f32) -> LayoutNode {
    let mut style = Style::default();
    style.spacing.margin_left = Length::Px(x);
    style.spacing.margin_top = Length::Px(y);
    style.size.width = Length::Px(width);
    style.size.height = Length::Px(height);
    LayoutNode::with_children(style, Vec::new())
}
