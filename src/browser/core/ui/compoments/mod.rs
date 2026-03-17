//! UIコンポーメントの定義

mod button;

/// UIコンポーメント一覧
#[derive(Clone, Debug)]
#[derive(PartialEq)]
pub enum Compoments {
    Input(InputKind),
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
