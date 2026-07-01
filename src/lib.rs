//! # OriniumBrowser
//!
//! ## 最小実行
//!
//! [`browser::BrowserApp`] を通じて Window が作成されます。
//! 詳細については、 [`browser::BrowserApp`] のドキュメントを参照してください。
//!
//! ```no_run
//! use orinium_browser::browser::BrowserApp;
//!
//! let browser = BrowserApp::default();
//! browser.run();
//! ```
//!
//! ## 開発／寄稿のためのヒント:
//! - [`browser::core`] -> [`engine`] -> [`platform`] の順で実装を辿ると理解が進みます。
//!
//! ### 参照するべきモジュール:
//! - `core` — アプリケーションライフサイクル、タブ管理、イベントループ
//! - `engine` — パーサ、レイアウト、描画コマンドの生成（仕様中心のロジック）
//! - `platform` — ネットワーク、フォント、GPU、OS 統合（プラットフォーム依存実装）
#![warn(clippy::perf)]
#![warn(clippy::correctness)]
#![warn(clippy::suspicious)]
#![warn(clippy::clone_on_ref_ptr)]
#![warn(clippy::assigning_clones)]
#![warn(clippy::large_enum_variant)]
#![warn(clippy::redundant_clone)]
#![warn(clippy::needless_collect)]
#![warn(clippy::implicit_clone)]
#![warn(clippy::manual_memcpy)]
#![warn(clippy::manual_slice_fill)]
#![warn(clippy::ptr_as_ptr)]
#![warn(clippy::cast_ptr_alignment)]

#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

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

mod process;
pub use process::*;
