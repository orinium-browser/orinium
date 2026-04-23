//! Browser UI components. WIP stub for future UI development.

use crate::engine::renderer_model::DrawCommand;
use ui_layout::{LayoutEngine, LayoutNode, Length, Style};

/// チェックボックスやラジオボタンなどのUIコンポーネントを定義するモジュール
pub mod compoments;
use compoments::{Button, ComponentEvent, DrawCommandEmitter};

#[derive(Debug, Default)]
pub struct BrowserUi {
    pub buttons: Vec<Button>,
    pub focused: Option<usize>,
}

pub fn init_browser_ui(window_size: (u32, u32)) -> BrowserUi {
    let mut ui = BrowserUi {
        buttons: Vec::new(),
        focused: None,
    };
    let button = Button::new(
        "default",
        "Click",
        create_button_layout(window_size.0 as f32 - 140.0, 16.0, 120.0, 36.0),
    );
    ui.buttons.push(button);
    ui
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

        let handled = button.on_event(ComponentEvent::PointerDown { x, y });
        if handled {
            self.focused = Some(button_index);
            for (i, b) in self.buttons.iter_mut().enumerate() {
                b.focused = i == button_index;
            }
        }
        handled
    }

    pub fn add_button(&mut self, id: impl Into<String>, label: impl Into<String>, x: f32, y: f32, width: f32, height: f32) {
        let layout = create_button_layout(x, y, width, height);
        let btn = Button::new(id.into(), label.into(), layout);
        self.buttons.push(btn);
    }

    pub fn focus_next(&mut self) {
        if self.buttons.is_empty() {
            self.focused = None;
            return;
        }
        let next = match self.focused {
            Some(idx) => (idx + 1) % self.buttons.len(),
            None => 0,
        };
        self.focused = Some(next);
        for (i, b) in self.buttons.iter_mut().enumerate() {
            b.focused = i == next;
        }
    }

    pub fn activate_focused(&mut self) {
        if let Some(idx) = self.focused {
            if let Some(b) = self.buttons.get(idx) {
                if let Some(box_model) = b.layout.layout_boxes.get(0) {
                    let rect = box_model.padding_box;
                    let cx = rect.x + rect.width / 2.0;
                    let cy = rect.y + rect.height / 2.0;
                    let _ = self.notify_pointer_down(idx, cx, cy);
                }
            }
        }
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
