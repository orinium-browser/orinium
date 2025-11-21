/// ブラウザ関連のモジュールをまとめたトップレベルモジュール
pub mod browser;

/// ブラウザのコア機能を提供するモジュール
/// このモジュールには、HTML/CSSパーサー、DOMツリー構築、
/// JavaScriptエンジンなどブラウザの中核となる機能が含まれます。
pub mod engine;

/// プラットフォーム依存の機能を提供するモジュール
/// このモジュールには、ネットワーク処理、レンダリング、UI表示、
/// ファイルI/Oなどプラットフォーム固有の実装が含まれます。
pub mod platform;

pub use engine::html;
pub use engine::html::parser;
pub use engine::renderer;
pub use platform::network;
pub use platform::ui;
