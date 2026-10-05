//! Engine と Platform 間の橋渡しモジュール。
//!
//! ここには engine が platform の実装に依存せずに済むよう、両者の境界に
//! 置于かれる抽象（trait と値型）だけを置く。engine から platform への
//! 参照は禁止されるため、これらの trait の実装は必ず platform 側にある。

pub mod audio;
pub mod text;
