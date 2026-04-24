//! UIコンポーネントの定義

pub mod button;
use crate::engine::renderer_model::DrawCommand;
pub use button::Button;

/// UIコンポーネントのイベント
#[derive(Clone, Debug)]
pub enum ComponentEvent {
    PointerDown { x: f32, y: f32 },
    PointerUp { x: f32, y: f32 },
}

pub trait DrawCommandEmitter {
    fn draw_commands(&self) -> Vec<DrawCommand>;
}

/// UIコンポーネント一覧
#[derive(Clone, Debug, PartialEq)]
pub enum Components {
    Button,
    Input(InputKind),
    Audio,
    Video,
}

#[derive(PartialEq, Debug, Clone)]
pub enum InputKind {
    Text,
    Checkbox,
    Radio,
    Slider,
}

/// UIコンポーメントの管理
#[derive(Clone, Debug, PartialEq)]
pub struct Component {
    pub kind: Components,
}
