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

pub fn init_browser_ui(_window_size: (u32, u32)) -> BrowserUi {
    BrowserUi {
        buttons: Vec::new(),
        focused: None,
    }
}

impl BrowserUi {
    pub fn relayout(&mut self, viewport: (f32, f32)) {
        self.buttons.iter_mut().for_each(|button| {
            LayoutEngine::layout(&mut button.layout, viewport.0, viewport.1);
        });
    }

    pub fn draw_commands(&self) -> Vec<DrawCommand> {
        self.buttons.iter().flat_map(|button| button.draw_commands()).collect()
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
            self.buttons.iter_mut().enumerate().for_each(|(i, b)| b.focused = i == button_index);
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
        self.buttons.iter_mut().enumerate().for_each(|(i, b)| b.focused = i == next);
    }

    pub fn activate_focused(&mut self) {
        if let Some(idx) = self.focused {
            // extract rect coordinates without holding an immutable borrow across a mutable call
            if let Some(rect) = self
                .buttons
                .get(idx)
                .and_then(|b| b.layout.layout_boxes.iter().next().map(|bm| bm.padding_box))
            {
                let cx = rect.x + rect.width / 2.0;
                let cy = rect.y + rect.height / 2.0;
                let _ = self.notify_pointer_down(idx, cx, cy);
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
