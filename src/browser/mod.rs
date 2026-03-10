//! ブラウザ高レベルモジュール — application / tab / UI のエントリポイント
//!
//! 概要:
//! - このモジュールはブラウザアプリケーションの高レベルな接着部分を提供します。
//! - `core` サブモジュールに実装された `BrowserApp`, `Tab`, コマンド類を公開します。
//!
//! 目的（新しいコントリビュータ向け）:
//! - コアの処理フローを理解しやすくするための導線を提供します。
//! - まずは `core` の `BrowserApp` を起点にコード構造を辿ってください。
//!
//! 典型的な処理の流れ（概観）:
//! 1. resource / network から HTML/CSS/リソースを取得
//! 2. トークナイザ → パーサ → DOM ツリーの構築
//! 3. CSS のカスケードとスタイル解決
//! 4. レイアウトビルダーでレイアウトツリーを生成
//! 5. レンダラモデルで DrawCommand を生成
//! 6. Platform 層（GPU / テキスト / イメージ）で実際に描画
//!
//! 簡単な例（ローカル開発）:
//! ```no_run
//! use orinium_browser::browser::BrowserApp;
//! use orinium_browser::browser::Tab;
//!
//! // BrowserApp を生成して実行する（テストや実行時の簡易例）
//! let mut app = BrowserApp::default();
//! let mut tab = Tab::new();
//! tab.navigate("resource:///test/compatibility_test.html".parse().unwrap());
//! app.add_tab(tab);
//! app.run().unwrap();
//! ```
//!

pub mod core;

pub use core::BrowserApp;
pub use core::BrowserCommand;
pub use core::Tab;
