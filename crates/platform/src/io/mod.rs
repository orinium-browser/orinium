//! ファイルI/O関連の機能を提供するモジュール

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

#[allow(dead_code)]
pub fn load_local_file(path: &str) -> Result<Vec<u8>> {
    fs::read(path).with_context(|| format!("Failed to read file: {path}"))
}

/// リソースファイルを探して読み込む。
/// 順序は以下の通り：
/// - 実行ファイルのあるディレクトリ/resource/<rel_path> (build.rs が同期・生成したリソース)
/// - ./resource/<rel_path>
/// - カレントディレクトリ/resource/<rel_path>
///
/// ビルド成果物 (test インデックス等) が CWD のソースツリーより優先されるよう、
/// 実行ファイルの隣にある resource を最初に試す。
pub fn load_resource(rel_path: &str) -> Result<Vec<u8>> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // executable directory/resource/<rel_path>
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("resource").join(rel_path));
    }

    // ./resource/<rel_path>
    candidates.push(PathBuf::from("resource").join(rel_path));

    // current_dir()/resource/<rel_path>
    if let Ok(cd) = std::env::current_dir() {
        candidates.push(cd.join("resource").join(rel_path));
    }

    for cand in candidates {
        if cand.is_file() {
            return fs::read(&cand).with_context(|| format!("Failed to read resource {:?}", cand));
        }
    }

    anyhow::bail!("Resource not found: {}", rel_path)
}
