//! UIコンポーメントの定義

pub mod button;
pub use button::Button;
use crate::engine::renderer_model::DrawCommand;

/// UIコンポーメントのイベント
#[derive(Clone, Debug)]
pub enum CompomentEvent {
    PointerDown { x: f32, y: f32 },
}

pub trait DrawCommandEmitter {
    fn draw_commands(&self) -> Vec<DrawCommand>;
}

/// UIコンポーメント一覧
#[derive(Clone, Debug)]
#[derive(PartialEq)]
pub enum Compoments {
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
#[derive(Clone, Debug)]
#[derive(PartialEq)]
pub struct Compoment {
    pub kind: Compoments,
}
